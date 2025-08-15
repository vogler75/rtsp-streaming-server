use std::sync::Arc;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use tracing::{info, error, debug};
use tokio::sync::broadcast;
use bytes::Bytes;
use axum::extract::ws::{WebSocket, Message};
use futures_util::{stream::StreamExt, SinkExt};

use crate::recording::RecordingManager;

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd")]
pub enum ControlCommand {
    #[serde(rename = "startrecording")]
    StartRecording {
        reason: Option<String>,
    },
    #[serde(rename = "stoprecording")]
    StopRecording,
    #[serde(rename = "startreplay")]
    StartReplay {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
    #[serde(rename = "stopreplay")]
    StopReplay,
    #[serde(rename = "replayspeed")]
    ReplaySpeed {
        speed: f32,
    },
    #[serde(rename = "listrecordings")]
    ListRecordings {
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    },
    #[serde(rename = "startlivestream")]
    StartLiveStream,
    #[serde(rename = "stoplivestream")]
    StopLiveStream,
}

#[derive(Debug, Serialize)]
pub struct CommandResponse {
    pub code: u16,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl CommandResponse {
    pub fn success(text: &str) -> Self {
        Self {
            code: 200,
            text: text.to_string(),
            data: None,
        }
    }

    pub fn success_with_data(text: &str, data: serde_json::Value) -> Self {
        Self {
            code: 200,
            text: text.to_string(),
            data: Some(data),
        }
    }

    pub fn error(code: u16, text: &str) -> Self {
        Self {
            code,
            text: text.to_string(),
            data: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplayState {
    pub active: bool,
    pub speed: f32,
    pub speed_sender: Option<broadcast::Sender<f32>>,
    pub stop_sender: Option<broadcast::Sender<()>>,
}

#[derive(Debug, Clone)]
pub struct LiveStreamState {
    pub active: bool,
    pub stop_sender: Option<broadcast::Sender<()>>,
}

impl Default for ReplayState {
    fn default() -> Self {
        Self {
            active: false,
            speed: 1.0,
            speed_sender: None,
            stop_sender: None,
        }
    }
}

impl Default for LiveStreamState {
    fn default() -> Self {
        Self {
            active: false,
            stop_sender: None,
        }
    }
}

pub struct ControlHandler {
    camera_id: String,
    client_id: String,
    recording_manager: Arc<RecordingManager>,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    replay_state: ReplayState,
    live_stream_state: LiveStreamState,
}

impl ControlHandler {
    pub fn new(
        camera_id: String,
        client_id: String,
        recording_manager: Arc<RecordingManager>,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
    ) -> Self {
        Self {
            camera_id,
            client_id,
            recording_manager,
            frame_sender,
            replay_state: ReplayState::default(),
            live_stream_state: LiveStreamState::default(),
        }
    }

    pub async fn handle_websocket(&mut self, socket: WebSocket) {
        let (sender, mut receiver) = socket.split();
        let sender = Arc::new(tokio::sync::Mutex::new(sender));
        info!("Control WebSocket connected for camera '{}' client '{}'", self.camera_id, self.client_id);


        // Handle incoming commands
        let recording_manager = self.recording_manager.clone();
        let camera_id = self.camera_id.clone();
        let client_id = self.client_id.clone();
        let frame_sender = self.frame_sender.clone();
        let sender_clone = sender.clone();
        let mut replay_state = self.replay_state.clone();
        let mut live_stream_state = self.live_stream_state.clone();

        let recv_task = tokio::spawn(async move {
            while let Some(msg) = receiver.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        debug!("Received control command: {}", text);
                        
                        match serde_json::from_str::<ControlCommand>(&text) {
                            Ok(command) => {
                                let response = Self::process_command(
                                    command,
                                    &camera_id,
                                    &client_id,
                                    &recording_manager,
                                    frame_sender.clone(),
                                    &mut replay_state,
                                    &mut live_stream_state,
                                    sender_clone.clone(),
                                ).await;
                                
                                if let Ok(response_json) = serde_json::to_string(&response) {
                                    let mut response_bytes = vec![0x01]; // Command response type
                                    response_bytes.extend_from_slice(response_json.as_bytes());
                                    
                                    let mut sender_guard = sender_clone.lock().await;
                                    if let Err(e) = sender_guard.send(Message::Binary(response_bytes)).await {
                                        error!("Failed to send command response: {}", e);
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Invalid command JSON: {}", e);
                                let error_response = CommandResponse::error(400, "Invalid command format");
                                
                                if let Ok(response_json) = serde_json::to_string(&error_response) {
                                    let mut response_bytes = vec![0x01];
                                    response_bytes.extend_from_slice(response_json.as_bytes());
                                    
                                    let mut sender_guard = sender_clone.lock().await;
                                    let _ = sender_guard.send(Message::Binary(response_bytes)).await;
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("Control WebSocket client disconnected");
                        break;
                    }
                    Err(e) => {
                        error!("Control WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
            info!("Control WebSocket receive task ended");
        });

        // Wait for tasks to complete
        let _ = recv_task.await;
        info!("Control WebSocket handler ended for camera '{}'", self.camera_id);
    }

    async fn process_command(
        command: ControlCommand,
        camera_id: &str,
        client_id: &str,
        recording_manager: &RecordingManager,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        replay_state: &mut ReplayState,
        live_stream_state: &mut LiveStreamState,
        sender: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ) -> CommandResponse {
        match command {
            ControlCommand::StartRecording { reason } => {
                Self::handle_start_recording(camera_id, client_id, reason, recording_manager, frame_sender).await
            }
            ControlCommand::StopRecording => {
                Self::handle_stop_recording(camera_id, recording_manager).await
            }
            ControlCommand::StartReplay { from, to } => {
                Self::handle_start_replay(camera_id, from, to, recording_manager, replay_state, live_stream_state, sender).await
            }
            ControlCommand::StopReplay => {
                Self::handle_stop_replay(replay_state).await
            }
            ControlCommand::ReplaySpeed { speed } => {
                Self::handle_replay_speed(speed, replay_state).await
            }
            ControlCommand::ListRecordings { from, to } => {
                Self::handle_list_recordings(camera_id, from, to, recording_manager).await
            }
            ControlCommand::StartLiveStream => {
                Self::handle_start_live_stream(frame_sender, replay_state, live_stream_state, sender).await
            }
            ControlCommand::StopLiveStream => {
                Self::handle_stop_live_stream(live_stream_state).await
            }
        }
    }

    async fn handle_start_recording(
        camera_id: &str,
        client_id: &str,
        reason: Option<String>,
        recording_manager: &RecordingManager,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
    ) -> CommandResponse {
        // Check if already recording
        if recording_manager.is_recording(camera_id).await {
            return CommandResponse::error(409, "Recording already in progress for this camera");
        }

        match recording_manager.start_recording(
            camera_id,
            client_id,
            reason.as_deref(),
            None, // No duration support
            frame_sender,
        ).await {
            Ok(session_id) => {
                let message = format!("Recording started (session {})", session_id);
                CommandResponse::success(&message)
            }
            Err(e) => {
                error!("Failed to start recording: {}", e);
                CommandResponse::error(500, "Failed to start recording")
            }
        }
    }

    async fn handle_stop_recording(
        camera_id: &str,
        recording_manager: &RecordingManager,
    ) -> CommandResponse {
        match recording_manager.stop_recording(camera_id).await {
            Ok(was_recording) => {
                if was_recording {
                    CommandResponse::success("Recording stopped")
                } else {
                    CommandResponse::error(404, "No active recording found")
                }
            }
            Err(e) => {
                error!("Failed to stop recording: {}", e);
                CommandResponse::error(500, "Failed to stop recording")
            }
        }
    }

    async fn handle_start_replay(
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        recording_manager: &RecordingManager,
        replay_state: &mut ReplayState,
        live_stream_state: &mut LiveStreamState,
        sender: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ) -> CommandResponse {
        // Stop any existing replay
        if replay_state.active {
            if let Some(stop_sender) = &replay_state.stop_sender {
                let _ = stop_sender.send(());
            }
            replay_state.active = false;
        }

        // Stop any active live stream
        if live_stream_state.active {
            Self::handle_stop_live_stream(live_stream_state).await;
        }

        // Check if frames exist first
        match recording_manager.get_replay_frames(camera_id, from, to).await {
            Ok(frames) => {
                if frames.is_empty() {
                    return CommandResponse::error(404, "No recorded frames found in the specified time range");
                }

                let frame_count = frames.len();

                // Create control channels
                let (speed_sender, mut speed_receiver) = broadcast::channel(1);
                let (stop_sender, mut stop_receiver) = broadcast::channel(1);
                
                replay_state.active = true;
                replay_state.speed_sender = Some(speed_sender.clone());
                replay_state.stop_sender = Some(stop_sender.clone());

                // Start the replay task
                let camera_id_clone = camera_id.to_string();
                let sender_clone = sender.clone();
                let recording_manager_clone = recording_manager.clone();
                
                tokio::spawn(async move {
                    info!("Starting replay for camera '{}' with {} frames", camera_id_clone, frame_count);
                    
                    // Get frames again for the replay task
                    if let Ok(frames) = recording_manager_clone.get_replay_frames(&camera_id_clone, from, to).await {
                        let mut current_speed = 1.0f32;
                        let mut last_timestamp = if !frames.is_empty() { frames[0].timestamp } else { Utc::now() };
                        
                        for frame in frames {
                            // Check for stop signal
                            if stop_receiver.try_recv().is_ok() {
                                info!("Replay stopped by user");
                                break;
                            }
                            
                            // Check for speed updates
                            if let Ok(new_speed) = speed_receiver.try_recv() {
                                current_speed = new_speed;
                                info!("Replay speed changed to {}x", current_speed);
                            }
                            
                            // Calculate delay between frames
                            let frame_delay = frame.timestamp.signed_duration_since(last_timestamp);
                            let adjusted_delay = if current_speed > 0.0 {
                                (frame_delay.num_milliseconds() as f32 / current_speed).max(0.0)
                            } else {
                                0.0
                            };
                            
                            // Wait for the appropriate time
                            if adjusted_delay > 0.0 {
                                tokio::time::sleep(tokio::time::Duration::from_millis(adjusted_delay as u64)).await;
                            }
                            
                            // Send frame with protocol byte
                            let mut frame_bytes = vec![0x00]; // Video frame type
                            frame_bytes.extend_from_slice(&frame.frame_data);
                            
                            let mut sender_guard = sender_clone.lock().await;
                            if let Err(e) = sender_guard.send(Message::Binary(frame_bytes)).await {
                                error!("Failed to send replay frame: {}", e);
                                break;
                            }
                            drop(sender_guard);
                            
                            last_timestamp = frame.timestamp;
                        }
                        
                        info!("Replay completed for camera '{}'", camera_id_clone);
                    }
                });
                
                let data = serde_json::json!({
                    "frame_count": frame_count,
                    "from": from,
                    "to": to
                });
                CommandResponse::success_with_data("Replay started", data)
            }
            Err(e) => {
                error!("Failed to get replay frames: {}", e);
                CommandResponse::error(500, "Failed to start replay")
            }
        }
    }

    async fn handle_stop_replay(replay_state: &mut ReplayState) -> CommandResponse {
        if replay_state.active {
            if let Some(stop_sender) = &replay_state.stop_sender {
                let _ = stop_sender.send(());
            }
            replay_state.active = false;
            replay_state.speed_sender = None;
            replay_state.stop_sender = None;
            CommandResponse::success("Replay stopped")
        } else {
            CommandResponse::error(404, "No active replay to stop")
        }
    }

    async fn handle_replay_speed(speed: f32, replay_state: &mut ReplayState) -> CommandResponse {
        if speed <= 0.0 || speed > 10.0 {
            CommandResponse::error(400, "Speed must be between 0.1 and 10.0")
        } else if !replay_state.active {
            CommandResponse::error(404, "No active replay")
        } else {
            replay_state.speed = speed;
            if let Some(speed_sender) = &replay_state.speed_sender {
                let _ = speed_sender.send(speed);
            }
            CommandResponse::success(&format!("Replay speed set to {}x", speed))
        }
    }

    async fn handle_list_recordings(
        camera_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        recording_manager: &RecordingManager,
    ) -> CommandResponse {
        match recording_manager.list_recordings(Some(camera_id), from, to).await {
            Ok(recordings) => {
                let recordings_data: Vec<serde_json::Value> = recordings
                    .into_iter()
                    .map(|r| serde_json::json!({
                        "id": r.id,
                        "camera_id": r.camera_id,
                        "start_time": r.start_time,
                        "end_time": r.end_time,
                        "reason": r.reason,
                        "status": String::from(r.status),
                        "duration_seconds": r.end_time
                            .map(|end| end.signed_duration_since(r.start_time).num_seconds())
                    }))
                    .collect();

                let data = serde_json::json!({
                    "recordings": recordings_data,
                    "count": recordings_data.len()
                });

                CommandResponse::success_with_data("Recordings retrieved", data)
            }
            Err(e) => {
                error!("Failed to list recordings: {}", e);
                CommandResponse::error(500, "Failed to list recordings")
            }
        }
    }

    async fn handle_start_live_stream(
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        replay_state: &mut ReplayState,
        live_stream_state: &mut LiveStreamState,
        sender: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ) -> CommandResponse {
        // Stop any active replay first
        if replay_state.active {
            Self::handle_stop_replay(replay_state).await;
        }

        // Check if already streaming
        if live_stream_state.active {
            return CommandResponse::error(409, "Live stream already active");
        }

        // Create stop signal channel
        let (stop_sender, mut stop_receiver) = broadcast::channel::<()>(1);
        let mut frame_receiver = frame_sender.subscribe();

        // Start the live streaming task
        let sender_clone = sender.clone();
        let stream_task = tokio::spawn(async move {
            info!("Starting live stream forwarding");
            
            loop {
                tokio::select! {
                    // Check for stop signal
                    _ = stop_receiver.recv() => {
                        info!("Received stop signal for live stream");
                        break;
                    }
                    // Forward frames from camera
                    frame_result = frame_receiver.recv() => {
                        match frame_result {
                            Ok(frame_data) => {
                                // Prepend frame type (0x00 for video)
                                let mut message_data = vec![0x00];
                                message_data.extend_from_slice(&frame_data);
                                
                                let message = Message::Binary(message_data);
                                if let Ok(mut sender_guard) = sender_clone.try_lock() {
                                    if sender_guard.send(message).await.is_err() {
                                        error!("Failed to send live frame, stopping stream");
                                        break;
                                    }
                                } else {
                                    // Sender is busy, skip this frame
                                    continue;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                // Skip lagged frames
                                continue;
                            }
                            Err(_) => {
                                info!("Frame sender closed, stopping live stream");
                                break;
                            }
                        }
                    }
                }
            }
            
            info!("Live stream task ended");
        });

        // Update state
        live_stream_state.active = true;
        live_stream_state.stop_sender = Some(stop_sender);

        // Store task handle to clean up later if needed
        tokio::spawn(async move {
            let _ = stream_task.await;
        });

        CommandResponse::success("Live stream started")
    }

    async fn handle_stop_live_stream(
        live_stream_state: &mut LiveStreamState,
    ) -> CommandResponse {
        if !live_stream_state.active {
            return CommandResponse::error(400, "No active live stream");
        }

        // Send stop signal
        if let Some(stop_sender) = &live_stream_state.stop_sender {
            let _ = stop_sender.send(());
        }

        // Reset state
        live_stream_state.active = false;
        live_stream_state.stop_sender = None;

        CommandResponse::success("Live stream stopped")
    }
}

pub async fn handle_control_websocket(
    socket: WebSocket,
    camera_id: String,
    client_id: String,
    recording_manager: Arc<RecordingManager>,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
) {
    let mut handler = ControlHandler::new(camera_id, client_id, recording_manager, frame_sender);
    handler.handle_websocket(socket).await;
}