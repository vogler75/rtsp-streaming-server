use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, broadcast};
use chrono::{DateTime, Utc, Timelike};
use tracing::{info, error, warn, trace};
use bytes::Bytes;

use crate::config::RecordingConfig;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use crate::database::{DatabaseProvider, RecordingSession, RecordedFrame, RecordingQuery, VideoSegment};


#[derive(Debug, Clone)]
pub struct ActiveRecording {
    pub session_id: i64,
    pub start_time: DateTime<Utc>,
    pub frame_count: u64,
    pub requested_duration: Option<i64>,
}

#[derive(Clone)]
pub struct RecordingManager {
    config: Arc<RecordingConfig>,
    databases: Arc<RwLock<HashMap<String, Arc<dyn DatabaseProvider>>>>, // camera_id -> database
    active_recordings: Arc<RwLock<HashMap<String, ActiveRecording>>>, // camera_id -> recording
    frame_subscribers: Arc<RwLock<HashMap<String, broadcast::Receiver<Bytes>>>>, // camera_id -> receiver
}

impl RecordingManager {
    pub async fn new(config: Arc<RecordingConfig>) -> crate::errors::Result<Self> {
        Ok(Self {
            config,
            databases: Arc::new(RwLock::new(HashMap::new())),
            active_recordings: Arc::new(RwLock::new(HashMap::new())),
            frame_subscribers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Add a database for a specific camera
    pub async fn add_camera_database(
        &self,
        camera_id: &str,
        database: Arc<dyn DatabaseProvider>,
    ) -> crate::errors::Result<()> {
        // Initialize the database
        database.initialize().await?;
        
        // Add to the databases map
        let mut databases = self.databases.write().await;
        databases.insert(camera_id.to_string(), database);
        
        Ok(())
    }

    /// Get the database for a specific camera
    async fn get_camera_database(&self, camera_id: &str) -> Option<Arc<dyn DatabaseProvider>> {
        let databases = self.databases.read().await;
        databases.get(camera_id).cloned()
    }

    pub async fn start_recording(
        &self,
        camera_id: &str,
        _client_id: &str,
        reason: Option<&str>,
        requested_duration: Option<i64>,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
    ) -> crate::errors::Result<i64> {
        // Get the database for this camera
        let database = self.get_camera_database(camera_id).await
            .ok_or_else(|| crate::errors::StreamError::config(&format!("No database found for camera '{}'", camera_id)))?;

        // Stop any existing recording for this camera
        self.stop_camera_recordings(camera_id).await?;

        // Create new recording session in database
        let session_id = database.create_recording_session(
            camera_id,
            reason,
        ).await?;

        // Create active recording entry
        let active_recording = ActiveRecording {
            session_id,
            start_time: Utc::now(),
            frame_count: 0,
            requested_duration,
        };

        // Store active recording
        let mut active_recordings = self.active_recordings.write().await;
        active_recordings.insert(camera_id.to_string(), active_recording);
        drop(active_recordings);

        // Subscribe to frame stream and start recording task
        let frame_receiver = frame_sender.subscribe();
        let mut frame_subscribers = self.frame_subscribers.write().await;
        frame_subscribers.insert(camera_id.to_string(), frame_receiver);
        drop(frame_subscribers);

        // Start recording task
        self.start_recording_task(camera_id.to_string(), session_id, frame_sender).await;

        info!("Started recording for camera '{}' with session ID {}", camera_id, session_id);
        Ok(session_id)
    }

    async fn frame_recording_loop(
        config: Arc<RecordingConfig>,
        database: Arc<dyn DatabaseProvider>,
        active_recordings: Arc<RwLock<HashMap<String, ActiveRecording>>>,
        camera_id: String,
        mut session_id: i64,
        mut frame_receiver: broadcast::Receiver<Bytes>,
    ) {
        let mut frame_number = 0i64;
        let mut last_hour = Utc::now().hour(); // Track the hour of the last frame

        loop {
            match frame_receiver.recv().await {
                Ok(frame_data) => {
                    frame_number += 1;
                    let timestamp = Utc::now();
                    let current_hour = timestamp.hour();

                    // Check if recording is still active
                    let active_recordings_guard = active_recordings.read().await;
                    let is_active = active_recordings_guard.contains_key(&camera_id);
                    drop(active_recordings_guard);

                    if !is_active {
                        trace!("Recording stopped for camera '{}', ending task", camera_id);
                        break;
                    }

                    // Check for hourly session splitting (when hour changes)
                    if last_hour != current_hour {
                        info!("Hour changed from {} to {} for camera '{}', splitting recording session {}", 
                                last_hour, current_hour, camera_id, session_id);
                        
                        // Get the recording reason from the database to use for the new session
                        if let Ok(sessions) = database.get_active_recordings(&camera_id).await {
                            if let Some(current_session) = sessions.first() {
                                let reason = current_session.reason.clone();
                                
                                // Stop the current session
                                if let Err(e) = database.stop_recording_session(session_id).await {
                                    error!("Failed to stop recording session for hourly split: {}", e);
                                } else {
                                    info!("Stopped recording session {} for hourly split", session_id);
                                    
                                    // Create a new session with the same reason
                                    match database.create_recording_session(&camera_id, reason.as_deref()).await {
                                        Ok(new_session_id) => {
                                            info!("Created new recording session {} for hourly continuation", new_session_id);
                                            
                                            // Update the active recording with new session info
                                            let mut active_recordings_guard = active_recordings.write().await;
                                            if let Some(recording) = active_recordings_guard.get_mut(&camera_id) {
                                                recording.session_id = new_session_id;
                                                recording.start_time = timestamp;
                                                recording.frame_count = 0;
                                            }
                                            drop(active_recordings_guard);
                                            
                                            // Update the session_id for subsequent frames
                                            session_id = new_session_id;
                                            frame_number = 1; // Reset frame number for new session
                                        }
                                        Err(e) => {
                                            error!("Failed to create new recording session for hourly split: {}", e);
                                            // Continue with the old session rather than stopping
                                        }
                                    }
                                }
                            }
                        }
                        
                        // Update the last hour tracker
                        last_hour = current_hour;
                    }

                    // Check frame size
                    if frame_data.len() > config.max_frame_size {
                        error!("Frame size {} exceeds maximum {} for camera '{}'", 
                                frame_data.len(), config.max_frame_size, camera_id);
                        continue;
                    }

                    // Store frame directly in database
                    if let Err(e) = database.add_recorded_frame(
                        session_id,
                        timestamp,
                        frame_number,
                        &frame_data,
                    ).await {
                        error!("Failed to store frame in database: {}", e);
                        continue;
                    }

                    // Update frame count
                    let mut active_recordings_guard = active_recordings.write().await;
                    if let Some(recording) = active_recordings_guard.get_mut(&camera_id) {
                        recording.frame_count += 1;

                        // Check if duration-based recording should stop
                        if let Some(duration) = recording.requested_duration {
                            let elapsed = timestamp.signed_duration_since(recording.start_time);
                            if elapsed.num_seconds() >= duration {
                                info!("Recording duration reached for camera '{}', stopping", camera_id);
                                break;
                            }
                        }
                    }
                    drop(active_recordings_guard);
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!("Recording lagged for camera '{}', skipped {} frames", camera_id, skipped);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Frame channel closed for camera '{}', stopping recording", camera_id);
                    break;
                }
            }
        }
    }

    async fn start_recording_task(
        &self,
        camera_id: String,
        session_id: i64,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
    ) {
        let database = match self.get_camera_database(&camera_id).await {
            Some(db) => db,
            None => {
                error!("No database found for camera '{}', cannot start recording task", camera_id);
                return;
            }
        };
        let config = self.config.clone();
        let active_recordings = self.active_recordings.clone();

        tokio::spawn(async move {
            let mut tasks = Vec::new();

            if config.frame_storage_enabled {
                let frame_receiver = frame_sender.subscribe();
                let task = tokio::spawn(Self::frame_recording_loop(
                    config.clone(),
                    database.clone(),
                    active_recordings.clone(),
                    camera_id.clone(),
                    session_id,
                    frame_receiver,
                ));
                tasks.push(task);
            }

            if config.video_storage_enabled {
                let segmenter_task = tokio::spawn(Self::video_segmenter_loop(
                    config.clone(),
                    database.clone(),
                    active_recordings.clone(),
                    camera_id.clone(),
                    frame_sender.subscribe(),
                ));
                tasks.push(segmenter_task);
            }

            // Wait for all recording tasks to complete
            for task in tasks {
                let _ = task.await;
            }

            // Clean up active recording
            let mut active_recordings_guard = active_recordings.write().await;
            active_recordings_guard.remove(&camera_id);
            drop(active_recordings_guard);

            // Mark session as completed in database
            if let Err(e) = database.stop_recording_session(session_id).await {
                error!("Failed to mark recording session as stopped: {}", e);
            }

            info!("Recording task ended for camera '{}' session {}", camera_id, session_id);
        });
    }

    pub async fn stop_recording(&self, camera_id: &str) -> crate::errors::Result<bool> {
        let mut active_recordings = self.active_recordings.write().await;
        
        if let Some(recording) = active_recordings.remove(camera_id) {
            drop(active_recordings);
            
            // Get the database for this camera and stop the recording
            if let Some(database) = self.get_camera_database(camera_id).await {
                database.stop_recording_session(recording.session_id).await?;
            } else {
                error!("No database found for camera '{}', cannot stop recording session", camera_id);
            }
            
            info!("Stopped recording for camera '{}' (session {})", camera_id, recording.session_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn stop_camera_recordings(&self, camera_id: &str) -> crate::errors::Result<()> {
        // Get the database for this camera
        let database = self.get_camera_database(camera_id).await
            .ok_or_else(|| crate::errors::StreamError::config(&format!("No database found for camera '{}'", camera_id)))?;

        // Get active recordings from database and stop them
        let active_sessions = database.get_active_recordings(camera_id).await?;
        let session_count = active_sessions.len();
        
        for session in active_sessions {
            database.stop_recording_session(session.id).await?;
        }

        // Remove from active recordings map
        let mut active_recordings = self.active_recordings.write().await;
        active_recordings.remove(camera_id);
        drop(active_recordings);

        if session_count > 0 {
            info!("Stopped {} active recordings for camera '{}'", session_count, camera_id);
        }

        Ok(())
    }

    pub async fn list_recordings(
        &self,
        camera_id: Option<&str>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> crate::errors::Result<Vec<RecordingSession>> {
        let query = RecordingQuery {
            camera_id: camera_id.map(|s| s.to_string()),
            from,
            to,
        };

        if let Some(cam_id) = camera_id {
            // Query specific camera's database
            if let Some(database) = self.get_camera_database(cam_id).await {
                database.list_recordings(&query).await
            } else {
                Ok(Vec::new()) // No database for this camera
            }
        } else {
            // Query all camera databases and combine results
            let mut all_recordings = Vec::new();
            let databases = self.databases.read().await;
            
            for (_, database) in databases.iter() {
                match database.list_recordings(&query).await {
                    Ok(recordings) => all_recordings.extend(recordings),
                    Err(e) => error!("Failed to query recordings from database: {}", e),
                }
            }
            
            // Sort by start time
            all_recordings.sort_by(|a, b| a.start_time.cmp(&b.start_time));
            Ok(all_recordings)
        }
    }

    pub async fn create_replay_stream(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: Option<DateTime<Utc>>,
    ) -> crate::errors::Result<Box<dyn crate::database::FrameStream>> {
        // Get the database for this camera
        let database = self.get_camera_database(camera_id).await
            .ok_or_else(|| crate::errors::StreamError::config(&format!("No database found for camera '{}'", camera_id)))?;

        // If no end time specified, use start time plus 1 hour
        let end_time = to.unwrap_or_else(|| from + chrono::Duration::hours(1));
        database.create_frame_stream(camera_id, from, end_time).await
    }

    pub async fn is_recording(&self, camera_id: &str) -> bool {
        let active_recordings = self.active_recordings.read().await;
        active_recordings.contains_key(camera_id)
    }

    pub async fn get_active_recording(&self, camera_id: &str) -> Option<ActiveRecording> {
        let active_recordings = self.active_recordings.read().await;
        active_recordings.get(camera_id).cloned()
    }

    pub async fn get_recorded_frames(
        &self,
        session_id: i64,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> crate::errors::Result<Vec<RecordedFrame>> {
        // Since we don't know which camera this session belongs to, search all databases
        let databases = self.databases.read().await;
        
        for (_camera_id, database) in databases.iter() {
            match database.get_recorded_frames(session_id, from, to).await {
                Ok(frames) => {
                    if !frames.is_empty() {
                        return Ok(frames);
                    }
                }
                Err(_) => {
                    // Continue to next database if this one doesn't have the session
                    continue;
                }
            }
        }
        
        // No frames found in any database
        Ok(Vec::new())
    }
    
    pub async fn cleanup_task(&self) -> crate::errors::Result<()> {
        let databases = self.databases.read().await;
        for (camera_id, database) in databases.iter() {
            if let Err(e) = database.cleanup_database(&self.config).await {
                error!("Failed to cleanup database for camera '{}': {}", camera_id, e);
            }
        }
        Ok(())
    }
    
    pub async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
    ) -> crate::errors::Result<Option<RecordedFrame>> {
        // Get the database for this camera
        let database = self.get_camera_database(camera_id).await
            .ok_or_else(|| crate::errors::StreamError::config(&format!("No database found for camera '{}'", camera_id)))?;

        database.get_frame_at_timestamp(camera_id, timestamp).await
    }

    /// Check for active recordings at startup and restart them
    pub async fn restart_active_recordings_at_startup(
        &self,
        camera_frame_senders: &HashMap<String, Arc<broadcast::Sender<Bytes>>>,
    ) -> crate::errors::Result<()> {
        info!("Checking for active recordings to restart at startup...");
        
        let mut restarted_count = 0;
        
        for (camera_id, frame_sender) in camera_frame_senders {
            // Get the database for this camera
            let database = match self.get_camera_database(camera_id).await {
                Some(db) => db,
                None => {
                    error!("No database found for camera '{}', skipping restart check", camera_id);
                    continue;
                }
            };

            // Check database for active recording sessions for this camera
            match database.get_active_recordings(camera_id).await {
                Ok(active_sessions) => {
                    for session in active_sessions {
                        info!(
                            "Found active recording session {} for camera '{}', restarting recording...",
                            session.id, camera_id
                        );
                        
                        // Create active recording entry to track this session
                        let active_recording = ActiveRecording {
                            session_id: session.id,
                            start_time: session.start_time,
                            frame_count: 0, // Will be updated as new frames come in
                            requested_duration: None, // Not tracked for restarted sessions
                        };
                        
                        // Store active recording
                        let mut active_recordings = self.active_recordings.write().await;
                        active_recordings.insert(camera_id.clone(), active_recording);
                        drop(active_recordings);
                        
                        // Subscribe to frame stream and start recording task
                        let frame_receiver = frame_sender.subscribe();
                        let mut frame_subscribers = self.frame_subscribers.write().await;
                        frame_subscribers.insert(camera_id.clone(), frame_receiver);
                        drop(frame_subscribers);
                        
                        // Start recording task
                        self.start_recording_task(camera_id.clone(), session.id, frame_sender.clone()).await;
                        
                        restarted_count += 1;
                        info!(
                            "Restarted recording for camera '{}' with session ID {}",
                            camera_id, session.id
                        );
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to check for active recordings for camera '{}': {}",
                        camera_id, e
                    );
                }
            }
        }
        
        if restarted_count > 0 {
            info!("Restarted {} active recording(s) at startup", restarted_count);
        } else {
            info!("No active recordings found to restart at startup");
        }
        
        Ok(())
    }
    
    /// Get the database size for a specific camera
    pub async fn get_database_size(&self, camera_id: &str) -> crate::errors::Result<i64> {
        let databases = self.databases.read().await;
        
        if let Some(database) = databases.get(camera_id) {
            database.get_database_size().await
        } else {
            Err(crate::errors::StreamError::database(format!(
                "No database found for camera '{}'", camera_id
            )))
        }
    }

    pub async fn list_video_segments(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> crate::errors::Result<Vec<VideoSegment>> {
        if let Some(database) = self.get_camera_database(camera_id).await {
            database.list_video_segments(camera_id, from, to).await
        } else {
            Err(crate::errors::StreamError::database(format!(
                "No database found for camera '{}'", camera_id
            )).into())
        }
    }

    pub fn get_recordings_path(&self) -> &str {
        &self.config.database_path
    }

    async fn video_segmenter_loop(
        config: Arc<RecordingConfig>,
        database: Arc<dyn DatabaseProvider>,
        active_recordings: Arc<RwLock<HashMap<String, ActiveRecording>>>,
        camera_id: String,
        mut frame_receiver: broadcast::Receiver<Bytes>,
    ) {
        let segment_duration = chrono::Duration::minutes(config.video_segment_minutes as i64);
        let mut segment_start_time = Utc::now();
        let mut frame_buffer = Vec::new();

        loop {
            match frame_receiver.recv().await {
                Ok(frame_data) => {
                    // Check if recording is still active
                    if !active_recordings.read().await.contains_key(&camera_id) {
                        trace!("Recording stopped for camera '{}', ending segmenter task", camera_id);
                        break;
                    }

                    frame_buffer.push(frame_data);

                    if Utc::now().signed_duration_since(segment_start_time) >= segment_duration {
                        let frames_to_process = std::mem::take(&mut frame_buffer);
                        let end_time = Utc::now();

                        // Spawn a task to process the segment
                        let task_config = config.clone();
                        let task_database = database.clone();
                        let task_camera_id = camera_id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = Self::create_video_segment(
                                task_config,
                                task_database,
                                task_camera_id,
                                segment_start_time,
                                end_time,
                                frames_to_process,
                            ).await {
                                error!("Failed to create video segment: {}", e);
                            }
                        });

                        segment_start_time = end_time;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!("Video segmenter lagged for camera '{}', skipped {} frames", camera_id, skipped);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Frame channel closed for camera '{}', stopping video segmenter", camera_id);
                    break;
                }
            }
        }
    }

    async fn create_video_segment(
        config: Arc<RecordingConfig>,
        database: Arc<dyn DatabaseProvider>,
        camera_id: String,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        frames: Vec<Bytes>,
    ) -> crate::errors::Result<()> {
        if frames.is_empty() {
            return Ok(());
        }

        let recordings_dir = &config.database_path;
        let temp_file_path = format!("{}/{}_{}.mp4", recordings_dir, camera_id, start_time.timestamp());

        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-f", "mjpeg",
            "-i", "-",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            "-y", // Overwrite output file if it exists
            &temp_file_path,
        ]);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().expect("Failed to open ffmpeg stdin");

        let write_task = tokio::spawn(async move {
            for frame in frames {
                if let Err(e) = stdin.write_all(&frame).await {
                    error!("Failed to write frame to ffmpeg stdin: {}", e);
                    break;
                }
            }
            drop(stdin);
        });

        let status = child.wait().await?;
        write_task.await.map_err(|e| crate::errors::StreamError::server(format!("Task join error: {}", e)))?;

        if !status.success() {
            return Err(crate::errors::StreamError::ffmpeg("ffmpeg command failed"));
        }

        let metadata = tokio::fs::metadata(&temp_file_path).await?;
        let size_bytes = metadata.len() as i64;

        let segment = VideoSegment {
            id: 0, // DB will assign
            camera_id,
            start_time,
            end_time,
            file_path: temp_file_path,
            size_bytes,
        };

        database.add_video_segment(&segment).await?;

        Ok(())
    }
}
