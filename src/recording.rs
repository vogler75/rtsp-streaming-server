use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, broadcast};
use chrono::{DateTime, Utc};
use tracing::{info, error, debug, warn};
use bytes::Bytes;

use crate::database::{DatabaseProvider, RecordingSession, RecordedFrame, RecordingQuery};
use crate::errors::Result;

#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub max_frame_size: usize,  // Maximum size for a single frame in bytes
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            max_frame_size: 10 * 1024 * 1024, // 10MB default max frame size
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActiveRecording {
    pub session_id: i64,
    pub start_time: DateTime<Utc>,
    pub frame_count: u64,
    pub requested_duration: Option<i64>,
}

#[derive(Clone)]
pub struct RecordingManager {
    config: RecordingConfig,
    database: Arc<dyn DatabaseProvider>,
    active_recordings: Arc<RwLock<HashMap<String, ActiveRecording>>>, // camera_id -> recording
    frame_subscribers: Arc<RwLock<HashMap<String, broadcast::Receiver<Bytes>>>>, // camera_id -> receiver
}

impl RecordingManager {
    pub async fn new(
        config: RecordingConfig,
        database: Arc<dyn DatabaseProvider>,
    ) -> Result<Self> {
        // Initialize database
        database.initialize().await?;

        Ok(Self {
            config,
            database,
            active_recordings: Arc::new(RwLock::new(HashMap::new())),
            frame_subscribers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn start_recording(
        &self,
        camera_id: &str,
        _client_id: &str,
        reason: Option<&str>,
        requested_duration: Option<i64>,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
    ) -> Result<i64> {
        // Stop any existing recording for this camera
        self.stop_camera_recordings(camera_id).await?;

        // Create new recording session in database
        let session_id = self.database.create_recording_session(
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

    async fn start_recording_task(
        &self,
        camera_id: String,
        session_id: i64,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
    ) {
        let database = self.database.clone();
        let config = self.config.clone();
        let active_recordings = self.active_recordings.clone();

        tokio::spawn(async move {
            let mut frame_receiver = frame_sender.subscribe();
            let mut frame_number = 0i64;

            loop {
                match frame_receiver.recv().await {
                    Ok(frame_data) => {
                        frame_number += 1;
                        let timestamp = Utc::now();

                        // Check if recording is still active
                        let active_recordings_guard = active_recordings.read().await;
                        let is_active = active_recordings_guard.contains_key(&camera_id);
                        drop(active_recordings_guard);

                        if !is_active {
                            debug!("Recording stopped for camera '{}', ending task", camera_id);
                            break;
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

    pub async fn stop_recording(&self, camera_id: &str) -> Result<bool> {
        let mut active_recordings = self.active_recordings.write().await;
        
        if let Some(recording) = active_recordings.remove(camera_id) {
            drop(active_recordings);
            
            // Stop the recording in database
            self.database.stop_recording_session(recording.session_id).await?;
            
            info!("Stopped recording for camera '{}' (session {})", camera_id, recording.session_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn stop_camera_recordings(&self, camera_id: &str) -> Result<()> {
        // Get active recordings from database and stop them
        let active_sessions = self.database.get_active_recordings(camera_id).await?;
        let session_count = active_sessions.len();
        
        for session in active_sessions {
            self.database.stop_recording_session(session.id).await?;
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
    ) -> Result<Vec<RecordingSession>> {
        let query = RecordingQuery {
            camera_id: camera_id.map(|s| s.to_string()),
            from,
            to,
        };

        self.database.list_recordings(&query).await
    }

    pub async fn get_replay_frames(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<RecordedFrame>> {
        self.database.get_frames_in_range(camera_id, from, to).await
    }

    pub async fn is_recording(&self, camera_id: &str) -> bool {
        let active_recordings = self.active_recordings.read().await;
        active_recordings.contains_key(camera_id)
    }

    pub async fn get_active_recording(&self, camera_id: &str) -> Option<ActiveRecording> {
        let active_recordings = self.active_recordings.read().await;
        active_recordings.get(camera_id).cloned()
    }
}