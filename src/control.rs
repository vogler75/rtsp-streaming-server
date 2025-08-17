use std::sync::Arc;
use serde::{Deserialize, Serialize, Deserializer};
use chrono::{DateTime, Utc};
use tracing::{info, error, trace, debug};
use tokio::sync::broadcast;
use bytes::Bytes;
use axum::extract::ws::{WebSocket, Message};
use futures_util::{stream::StreamExt, SinkExt};

use crate::recording::RecordingManager;
use crate::database::RecordedFrame;


// Custom deserializer for timestamps that supports both string (ISO format) and number (ms since epoch)
fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct TimestampVisitor;

    impl<'de> Visitor<'de> for TimestampVisitor {
        type Value = DateTime<Utc>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a timestamp as either an ISO string or milliseconds since epoch")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // Try to parse as ISO string (existing behavior)
            DateTime::parse_from_rfc3339(value)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(de::Error::custom)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // Parse as milliseconds since epoch
            DateTime::from_timestamp_millis(value)
                .ok_or_else(|| de::Error::custom("invalid timestamp"))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // Parse as milliseconds since epoch
            DateTime::from_timestamp_millis(value as i64)
                .ok_or_else(|| de::Error::custom("invalid timestamp"))
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // Parse as milliseconds since epoch (handle floating point)
            DateTime::from_timestamp_millis(value as i64)
                .ok_or_else(|| de::Error::custom("invalid timestamp"))
        }
    }

    deserializer.deserialize_any(TimestampVisitor)
}

// Custom deserializer for optional timestamps
fn deserialize_optional_timestamp<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptionalTimestampVisitor;

    impl<'de> Visitor<'de> for OptionalTimestampVisitor {
        type Value = Option<DateTime<Utc>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an optional timestamp as either an ISO string or milliseconds since epoch")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserialize_timestamp(deserializer).map(Some)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            DateTime::parse_from_rfc3339(value)
                .map(|dt| Some(dt.with_timezone(&Utc)))
                .map_err(de::Error::custom)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            DateTime::from_timestamp_millis(value)
                .map(Some)
                .ok_or_else(|| de::Error::custom("invalid timestamp"))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            DateTime::from_timestamp_millis(value as i64)
                .map(Some)
                .ok_or_else(|| de::Error::custom("invalid timestamp"))
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            DateTime::from_timestamp_millis(value as i64)
                .map(Some)
                .ok_or_else(|| de::Error::custom("invalid timestamp"))
        }
    }

    deserializer.deserialize_option(OptionalTimestampVisitor)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd")]
pub enum ControlCommand {
    #[serde(rename = "start")]
    StartReplay {
        #[serde(deserialize_with = "deserialize_timestamp")]
        from: DateTime<Utc>,
        #[serde(deserialize_with = "deserialize_optional_timestamp", default)]
        to: Option<DateTime<Utc>>,  // Optional - if None, play until end
    },
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "speed")]
    ReplaySpeed {
        speed: f32,
    },
    #[serde(rename = "live")]
    StartLiveStream,
    #[serde(rename = "goto")]
    GoToTimestamp {
        #[serde(deserialize_with = "deserialize_timestamp")]
        timestamp: DateTime<Utc>,
    },
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
        trace!("[CONTROL] handle_websocket starting for camera '{}' client '{}'", self.camera_id, self.client_id);
        let (sender, mut receiver) = socket.split();
        trace!("[CONTROL] WebSocket split completed for camera '{}' client '{}'", self.camera_id, self.client_id);
        let sender = Arc::new(tokio::sync::Mutex::new(sender));
        trace!("[CONTROL] WebSocket sender wrapped in Arc<Mutex> for camera '{}' client '{}'", self.camera_id, self.client_id);
        info!("Control WebSocket connected for camera '{}' client '{}'", self.camera_id, self.client_id);

        // Create a channel to signal cleanup when connection closes
        let (cleanup_tx, _cleanup_rx) = broadcast::channel::<()>(1);

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
                        trace!("[CONTROL-CMD] Received control command: {}", text);
                        
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
            
            // Stop any active streams when disconnecting
            Self::handle_stop(&mut replay_state, &mut live_stream_state).await;
        });

        // Wait for tasks to complete with timeout to prevent hanging
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            recv_task
        ).await {
            Ok(_) => trace!("Control receive task completed normally"),
            Err(_) => {
                debug!("Timeout waiting for control receive task to complete (this is normal for slow client disconnections)");
            }
        }
        
        // Send cleanup signal to any running tasks
        let _ = cleanup_tx.send(());
        
        // Give tasks a moment to clean up
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        info!("Control WebSocket handler ended for camera '{}'", self.camera_id);
    }

    async fn process_command(
        command: ControlCommand,
        camera_id: &str,
        _client_id: &str,
        recording_manager: &RecordingManager,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        replay_state: &mut ReplayState,
        live_stream_state: &mut LiveStreamState,
        sender: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ) -> CommandResponse {
        match command {
            ControlCommand::StartReplay { from, to } => {
                Self::handle_start_replay(camera_id, from, to, recording_manager, replay_state, live_stream_state, sender).await
            }
            ControlCommand::Stop => {
                Self::handle_stop(replay_state, live_stream_state).await
            }
            ControlCommand::ReplaySpeed { speed } => {
                Self::handle_replay_speed(speed, replay_state).await
            }
            ControlCommand::StartLiveStream => {
                Self::handle_start_live_stream(frame_sender, replay_state, live_stream_state, sender).await
            }
            ControlCommand::GoToTimestamp { timestamp } => {
                Self::handle_goto_timestamp(camera_id, timestamp, recording_manager, sender).await
            }
        }
    }


    async fn handle_start_replay(
        camera_id: &str,
        from: DateTime<Utc>,
        to: Option<DateTime<Utc>>,
        recording_manager: &RecordingManager,
        replay_state: &mut ReplayState,
        live_stream_state: &mut LiveStreamState,
        sender: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ) -> CommandResponse {
        // Stop any existing replay or live stream
        if replay_state.active || live_stream_state.active {
            Self::handle_stop(replay_state, live_stream_state).await;
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
                            
                            // Send frame with timestamp
                            let frame_bytes = Self::encode_frame_with_timestamp(&frame);
                            
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

    async fn handle_stop(
        replay_state: &mut ReplayState, 
        live_stream_state: &mut LiveStreamState
    ) -> CommandResponse {
        let mut stopped_operations = Vec::new();
        
        // Check if replay is active and stop it
        if replay_state.active {
            if let Some(stop_sender) = &replay_state.stop_sender {
                let _ = stop_sender.send(());
            }
            replay_state.active = false;
            replay_state.speed_sender = None;
            replay_state.stop_sender = None;
            stopped_operations.push("replay");
        }
        
        // Check if live stream is active and stop it
        if live_stream_state.active {
            if let Some(stop_sender) = &live_stream_state.stop_sender {
                let _ = stop_sender.send(());
            }
            live_stream_state.active = false;
            live_stream_state.stop_sender = None;
            stopped_operations.push("live stream");
        }
        
        // Return appropriate response based on what was stopped
        match stopped_operations.len() {
            0 => CommandResponse::error(404, "No active replay or live stream to stop"),
            1 => CommandResponse::success(&format!("{} stopped", stopped_operations[0].to_string())),
            _ => CommandResponse::success(&format!("{} stopped", stopped_operations.join(" and "))),
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


    async fn handle_start_live_stream(
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        replay_state: &mut ReplayState,
        live_stream_state: &mut LiveStreamState,
        sender: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ) -> CommandResponse {
        // Stop any active replay first
        if replay_state.active {
            Self::handle_stop(replay_state, live_stream_state).await;
        }

        // Check if already streaming
        if live_stream_state.active {
            return CommandResponse::error(409, "Live stream already active");
        }

        // Create stop signal channel
        let (stop_sender, mut stop_receiver) = broadcast::channel::<()>(1);
        
        let subscriber_count_before = frame_sender.receiver_count();
        trace!("[CONTROL-LIVE] Subscriber count before subscribe: {} for camera", subscriber_count_before);
        
        trace!("[CONTROL-LIVE] About to call frame_sender.subscribe()...");
        let mut frame_receiver = frame_sender.subscribe();
        trace!("[CONTROL-LIVE] Successfully subscribed to frame_sender");
        
        let subscriber_count_after = frame_sender.receiver_count();
        trace!("[CONTROL-LIVE] Subscriber count after subscribe: {} (delta: +{})", 
             subscriber_count_after, subscriber_count_after.saturating_sub(subscriber_count_before));

        // Start the live streaming task
        let sender_clone = sender.clone();
        let _stream_task = tokio::spawn(async move {
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
                                // Create frame with timestamp for live stream
                                let mut message_data = Vec::new();
                                
                                // Protocol byte (0x00 for video frame)
                                message_data.push(0x00);
                                
                                // Current timestamp as 8 bytes (i64 milliseconds since epoch)
                                let timestamp_ms = chrono::Utc::now().timestamp_millis();
                                message_data.extend_from_slice(&timestamp_ms.to_le_bytes());
                                
                                // Frame data
                                message_data.extend_from_slice(&frame_data);
                                
                                let message = Message::Binary(message_data);
                                // Use timeout instead of try_lock to avoid skipping frames unnecessarily
                                match tokio::time::timeout(
                                    std::time::Duration::from_millis(5), // Reduced timeout for faster dropping
                                    async {
                                        let mut sender_guard = sender_clone.lock().await;
                                        sender_guard.send(message).await
                                    }
                                ).await {
                                    Ok(Ok(())) => {
                                        // Frame sent successfully
                                    }
                                    Ok(Err(e)) => {
                                        error!("Failed to send live frame: {}, stopping stream", e);
                                        break;
                                    }
                                    Err(_) => {
                                        // Timeout - client is too slow, skip this frame
                                        trace!("Skipped frame due to slow client");
                                        continue;
                                    }
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
            
            // Explicitly drop the frame receiver to ensure cleanup
            drop(frame_receiver);
            info!("Live stream task ended");
        });

        // Update state
        live_stream_state.active = true;
        live_stream_state.stop_sender = Some(stop_sender);

        // No need for nested spawn - the task will clean itself up
        
        CommandResponse::success("Live stream started")
    }
    
    // Helper function to encode frame with timestamp
    fn encode_frame_with_timestamp(frame: &RecordedFrame) -> Vec<u8> {
        let mut frame_bytes = Vec::new();
        
        // Protocol byte (0x00 for video frame)
        frame_bytes.push(0x00);
        
        // Timestamp as 8 bytes (i64 milliseconds since epoch)
        let timestamp_ms = frame.timestamp.timestamp_millis();
        frame_bytes.extend_from_slice(&timestamp_ms.to_le_bytes());
        
        // Frame data
        frame_bytes.extend_from_slice(&frame.frame_data);
        
        frame_bytes
    }
    
    // Handle goto command - seek to specific timestamp
    async fn handle_goto_timestamp(
        camera_id: &str,
        timestamp: DateTime<Utc>,
        recording_manager: &RecordingManager,
        sender: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ) -> CommandResponse {
        match recording_manager.get_frame_at_timestamp(camera_id, timestamp).await {
            Ok(Some(frame)) => {
                // Send the frame with timestamp
                let frame_bytes = Self::encode_frame_with_timestamp(&frame);
                
                let mut sender_guard = sender.lock().await;
                if let Err(e) = sender_guard.send(Message::Binary(frame_bytes)).await {
                    error!("Failed to send goto frame: {}", e);
                    return CommandResponse::error(500, "Failed to send frame");
                }
                
                let data = serde_json::json!({
                    "requested_timestamp": timestamp,
                    "actual_timestamp": frame.timestamp,
                    "frame_size": frame.frame_data.len()
                });
                CommandResponse::success_with_data("Goto timestamp completed", data)
            }
            Ok(None) => {
                // Send empty frame (0 bytes) when no frame found within 1 second
                let empty_frame = RecordedFrame {
                    timestamp,
                    frame_data: Vec::new(), // Empty frame data
                };
                let frame_bytes = Self::encode_frame_with_timestamp(&empty_frame);
                
                let mut sender_guard = sender.lock().await;
                if let Err(e) = sender_guard.send(Message::Binary(frame_bytes)).await {
                    error!("Failed to send empty goto frame: {}", e);
                    return CommandResponse::error(500, "Failed to send empty frame");
                }
                
                let data = serde_json::json!({
                    "requested_timestamp": timestamp,
                    "actual_timestamp": timestamp,
                    "frame_size": 0,
                    "note": "No frame found within 1 second of requested timestamp"
                });
                CommandResponse::success_with_data("Goto timestamp completed with empty frame", data)
            }
            Err(e) => {
                error!("Failed to get frame at timestamp: {}", e);
                CommandResponse::error(500, "Failed to retrieve frame")
            }
        }
    }

}

pub async fn handle_control_websocket(
    socket: WebSocket,
    camera_id: String,
    client_id: String,
    recording_manager: Arc<RecordingManager>,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
) {
    trace!("[CONTROL] handle_control_websocket started for camera {} client {}", camera_id, client_id);
    let mut handler = ControlHandler::new(camera_id.clone(), client_id.clone(), recording_manager, frame_sender);
    trace!("[CONTROL] ControlHandler created for camera {} client {}", camera_id, client_id);
    handler.handle_websocket(socket).await;
    trace!("[CONTROL] handle_control_websocket completed for camera {} client {}", camera_id, client_id);
}