use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, broadcast, mpsc};
use chrono::{DateTime, Utc, Local, Datelike};
use tracing::{info, error, warn, trace, debug};
use bytes::Bytes;

use crate::config::RecordingConfig;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use crate::database::{DatabaseProvider, RecordingSession, RecordedFrame, RecordingQuery, VideoSegment, RecordingHlsSegment};

/// Sanitize a recording reason string for safe use in filenames.
/// Returns None if the sanitized result is empty.
fn sanitize_reason_for_filename(reason: &str) -> Option<String> {
    let sanitized: String = reason
        .chars()
        .map(|c| {
            if c == ' ' || "\\/:*?\"<>|".contains(c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();

    // Collapse consecutive hyphens
    let mut collapsed = String::with_capacity(sanitized.len());
    let mut prev_hyphen = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_hyphen {
                collapsed.push('-');
            }
            prev_hyphen = true;
        } else {
            collapsed.push(c);
            prev_hyphen = false;
        }
    }

    // Trim leading/trailing dots, spaces, and hyphens
    let trimmed = collapsed.trim_matches(|c: char| c == '.' || c == ' ' || c == '-');

    // Truncate to 80 characters
    let truncated: String = trimmed.chars().take(80).collect();
    let truncated = truncated.trim_end_matches(|c: char| c == '.' || c == ' ' || c == '-');

    if truncated.is_empty() {
        None
    } else {
        Some(truncated.to_string())
    }
}

/// Message sent from frame receiver to database writer task
enum FrameWriterMessage {
    /// A frame to be written to the database
    Frame {
        session_id: i64,
        timestamp: DateTime<Utc>,
        frame_number: i64,
        data: Vec<u8>,
    },
    /// Session has changed (due to segmentation)
    SessionChanged {
        new_session_id: i64,
    },
    /// Flush buffer immediately and confirm
    Flush,
}

/// Channel buffer size for writer - allows ~60 seconds of frames at 15fps
const WRITER_CHANNEL_BUFFER: usize = 900;

/// Bulk write settings - batch more frames to reduce write frequency
const BULK_WRITE_MAX_FRAMES: usize = 60;
const BULK_WRITE_MAX_INTERVAL_MS: u64 = 1000;

/// Dedicated database writer task - receives frames via mpsc channel and writes in batches
async fn frame_writer_loop(
    database: Arc<dyn DatabaseProvider>,
    camera_id: String,
    mut receiver: mpsc::Receiver<FrameWriterMessage>,
) {
    let mut frame_buffer: Vec<(DateTime<Utc>, i64, Vec<u8>)> = Vec::with_capacity(BULK_WRITE_MAX_FRAMES);
    let mut current_session_id: Option<i64> = None;
    let mut last_flush_time = std::time::Instant::now();

    debug!("Frame writer started for camera '{}'", camera_id);

    loop {
        // Use timeout to ensure we flush periodically even without new frames
        let timeout = tokio::time::Duration::from_millis(BULK_WRITE_MAX_INTERVAL_MS);

        match tokio::time::timeout(timeout, receiver.recv()).await {
            Ok(Some(msg)) => {
                match msg {
                    FrameWriterMessage::Frame { session_id, timestamp, frame_number, data } => {
                        // Initialize session_id on first frame
                        if current_session_id.is_none() {
                            current_session_id = Some(session_id);
                        }

                        // If session changed, flush old session's frames first
                        if current_session_id != Some(session_id) && !frame_buffer.is_empty() {
                            if let Some(old_session_id) = current_session_id {
                                let count = frame_buffer.len();
                                if let Err(e) = database.add_recorded_frames_bulk(old_session_id, &camera_id, &frame_buffer).await {
                                    error!("Failed to flush {} frames for old session {}: {}", count, old_session_id, e);
                                } else {
                                    trace!("Flushed {} frames for old session {} before session change", count, old_session_id);
                                }
                                frame_buffer.clear();
                            }
                            current_session_id = Some(session_id);
                        }

                        frame_buffer.push((timestamp, frame_number, data));

                        // Flush if buffer is full
                        if frame_buffer.len() >= BULK_WRITE_MAX_FRAMES {
                            if let Some(sid) = current_session_id {
                                let count = frame_buffer.len();
                                let total_bytes: usize = frame_buffer.iter().map(|(_, _, d)| d.len()).sum();
                                let write_start = std::time::Instant::now();
                                match database.add_recorded_frames_bulk(sid, &camera_id, &frame_buffer).await {
                                    Ok(inserted) => {
                                        let write_ms = write_start.elapsed().as_millis();
                                        if write_ms > 500 {
                                            warn!("Slow frame write for camera '{}': {} frames ({} KB) in {}ms",
                                                  camera_id, inserted, total_bytes / 1024, write_ms);
                                        } else {
                                            debug!("Bulk inserted {} frames ({} KB) for camera '{}' in {}ms",
                                                   inserted, total_bytes / 1024, camera_id, write_ms);
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to bulk insert {} frames for camera '{}': {}", count, camera_id, e);
                                    }
                                }
                                frame_buffer.clear();
                                last_flush_time = std::time::Instant::now();
                            }
                        }
                    }
                    FrameWriterMessage::SessionChanged { new_session_id } => {
                        // Flush current buffer before session change
                        if !frame_buffer.is_empty() {
                            if let Some(old_session_id) = current_session_id {
                                let count = frame_buffer.len();
                                if let Err(e) = database.add_recorded_frames_bulk(old_session_id, &camera_id, &frame_buffer).await {
                                    error!("Failed to flush {} frames before session change: {}", count, e);
                                }
                                frame_buffer.clear();
                            }
                        }
                        current_session_id = Some(new_session_id);
                        last_flush_time = std::time::Instant::now();
                        debug!("Writer switched to session {} for camera '{}'", new_session_id, camera_id);
                    }
                    FrameWriterMessage::Flush => {
                        if !frame_buffer.is_empty() {
                            if let Some(sid) = current_session_id {
                                let count = frame_buffer.len();
                                if let Err(e) = database.add_recorded_frames_bulk(sid, &camera_id, &frame_buffer).await {
                                    error!("Failed to flush {} frames on request: {}", count, e);
                                } else {
                                    trace!("Flushed {} frames on request for camera '{}'", count, camera_id);
                                }
                                frame_buffer.clear();
                                last_flush_time = std::time::Instant::now();
                            }
                        }
                    }
                }
            }
            Ok(None) => {
                // Channel closed - flush remaining frames and exit
                if !frame_buffer.is_empty() {
                    if let Some(sid) = current_session_id {
                        let count = frame_buffer.len();
                        if let Err(e) = database.add_recorded_frames_bulk(sid, &camera_id, &frame_buffer).await {
                            error!("Failed to flush {} remaining frames on shutdown: {}", count, e);
                        } else {
                            debug!("Flushed {} remaining frames on writer shutdown for camera '{}'", count, camera_id);
                        }
                    }
                }
                debug!("Frame writer stopped for camera '{}'", camera_id);
                break;
            }
            Err(_) => {
                // Timeout - flush buffer if there are frames and enough time has passed
                if !frame_buffer.is_empty() && last_flush_time.elapsed().as_millis() >= BULK_WRITE_MAX_INTERVAL_MS as u128 {
                    if let Some(sid) = current_session_id {
                        let count = frame_buffer.len();
                        let total_bytes: usize = frame_buffer.iter().map(|(_, _, d)| d.len()).sum();
                        let write_start = std::time::Instant::now();
                        match database.add_recorded_frames_bulk(sid, &camera_id, &frame_buffer).await {
                            Ok(inserted) => {
                                let write_ms = write_start.elapsed().as_millis();
                                if write_ms > 500 {
                                    warn!("Slow periodic flush for camera '{}': {} frames ({} KB) in {}ms",
                                          camera_id, inserted, total_bytes / 1024, write_ms);
                                } else {
                                    debug!("Periodic flush: {} frames ({} KB) for camera '{}' in {}ms",
                                           inserted, total_bytes / 1024, camera_id, write_ms);
                                }
                            }
                            Err(e) => {
                                error!("Failed periodic flush of {} frames for camera '{}': {}", count, camera_id, e);
                            }
                        }
                        frame_buffer.clear();
                        last_flush_time = std::time::Instant::now();
                    }
                }
            }
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
    config: Arc<RecordingConfig>,
    pub databases: Arc<RwLock<HashMap<String, Arc<dyn DatabaseProvider>>>>, // camera_id -> database
    active_recordings: Arc<RwLock<HashMap<String, ActiveRecording>>>, // camera_id -> recording
    frame_subscribers: Arc<RwLock<HashMap<String, broadcast::Receiver<Bytes>>>>, // camera_id -> receiver
    camera_configs: Arc<RwLock<HashMap<String, crate::config::CameraConfig>>>, // camera configs for cleanup
    mp4_buffer_stats: Arc<RwLock<HashMap<String, Arc<tokio::sync::RwLock<crate::Mp4BufferStats>>>>>, // camera_id -> buffer stats
}

impl RecordingManager {
    pub async fn new(config: Arc<RecordingConfig>) -> crate::errors::Result<Self> {        
        Ok(Self {
            config,
            databases: Arc::new(RwLock::new(HashMap::new())),
            active_recordings: Arc::new(RwLock::new(HashMap::new())),
            frame_subscribers: Arc::new(RwLock::new(HashMap::new())),
            camera_configs: Arc::new(RwLock::new(HashMap::new())),
            mp4_buffer_stats: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Update camera configs for cleanup purposes
    pub async fn update_camera_configs(&self, configs: HashMap<String, crate::config::CameraConfig>) {
        let mut camera_configs = self.camera_configs.write().await;
        *camera_configs = configs;
    }
    
    /// Register MP4 buffer stats for a camera
    pub async fn register_mp4_buffer_stats(&self, camera_id: &str, stats: Arc<tokio::sync::RwLock<crate::Mp4BufferStats>>) {
        let mut buffer_stats = self.mp4_buffer_stats.write().await;
        buffer_stats.insert(camera_id.to_string(), stats);
    }
    
    /// Get MP4 buffer stats for a camera
    pub async fn get_mp4_buffer_stats(&self, camera_id: &str) -> Option<Arc<tokio::sync::RwLock<crate::Mp4BufferStats>>> {
        let buffer_stats = self.mp4_buffer_stats.read().await;
        buffer_stats.get(camera_id).cloned()
    }

    /// Get the recording configuration
    pub fn get_recording_config(&self) -> &RecordingConfig {
        &self.config
    }

    /// Add a database for a specific camera
    pub async fn add_camera_database(
        &self,
        camera_id: &str,
        database: Arc<dyn DatabaseProvider>,
    ) -> crate::errors::Result<()> {
        // Initialize the database
        database.initialize().await?;
        
        // Perform initial cleanup for this camera database
        info!("Performing initial cleanup for camera '{}' database", camera_id);
        let camera_configs = self.camera_configs.read().await;
        if let Err(e) = database.cleanup_database(&self.config, &camera_configs).await {
            error!("Failed to perform initial cleanup for camera '{}': {}", camera_id, e);
        }
        drop(camera_configs);
        
        // Add to the databases map
        let mut databases = self.databases.write().await;
        databases.insert(camera_id.to_string(), database);
        
        Ok(())
    }

    /// Get the database for a specific camera
    pub async fn get_camera_database(&self, camera_id: &str) -> Option<Arc<dyn DatabaseProvider>> {
        let databases = self.databases.read().await;
        databases.get(camera_id).cloned()
    }

    /// Get all camera IDs that have databases
    pub async fn get_all_camera_ids(&self) -> Vec<String> {
        let databases = self.databases.read().await;
        databases.keys().cloned().collect()
    }


    /// Get the effective storage type for a camera
    pub fn get_storage_type_for_camera(&self, camera_config: &crate::config::CameraConfig) -> crate::config::Mp4StorageType {
        camera_config.get_mp4_storage_type()
            .cloned()
            .unwrap_or(self.config.mp4_storage_type.clone())
    }

    pub async fn start_recording(
        &self,
        camera_id: &str,
        _client_id: &str,
        reason: Option<&str>,
        requested_duration: Option<i64>,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        camera_config: &crate::config::CameraConfig,
        pre_recording_buffer: Option<&crate::pre_recording_buffer::PreRecordingBuffer>,
    ) -> crate::errors::Result<i64> {
        // Get the database for this camera
        let database = self.get_camera_database(camera_id).await
            .ok_or_else(|| crate::errors::StreamError::config(&format!("No database found for camera '{}'", camera_id)))?;

        // Stop any existing recording for this camera
        self.stop_camera_recordings(camera_id).await?;

        // Determine the recording start time - use first frame from pre-recording buffer if available
        let recording_start_time = if let Some(buffer) = pre_recording_buffer {
            buffer.get_first_frame_timestamp().await.unwrap_or_else(|| Utc::now())
        } else {
            Utc::now()
        };

        // Create new recording session in database
        let session_id = database.create_recording_session(
            camera_id,
            reason,
            recording_start_time,
        ).await?;

        // If pre-recording buffer exists, store all buffered frames first using bulk insert
        let mut initial_frame_count = 0u64;
        if let Some(buffer) = pre_recording_buffer {
            let buffered_frames = buffer.get_buffered_frames().await;
            info!("Adding {} pre-recorded frames to recording session {} using bulk insert", buffered_frames.len(), session_id);
            
            if !buffered_frames.is_empty() {
                // Prepare data for bulk insert: (timestamp, frame_number, frame_data)
                let bulk_frames: Vec<(chrono::DateTime<chrono::Utc>, i64, Vec<u8>)> = buffered_frames
                    .iter()
                    .enumerate()
                    .map(|(index, frame)| (frame.timestamp, (index + 1) as i64, frame.data.to_vec()))
                    .collect();
                
                match database.add_recorded_frames_bulk(session_id, camera_id, &bulk_frames).await {
                    Ok(inserted_count) => {
                        initial_frame_count = inserted_count;
                        info!("Successfully bulk inserted {} pre-recorded frames for camera '{}'", inserted_count, camera_id);
                    }
                    Err(e) => {
                        error!("Failed to bulk insert pre-recorded frames: {}", e);
                        // Fallback to individual inserts if bulk insert fails
                        info!("Falling back to individual frame inserts for camera '{}'", camera_id);
                        for (frame_number, buffered_frame) in buffered_frames.iter().enumerate() {
                            if let Err(e) = database.add_recorded_frame(
                                session_id,
                                camera_id,
                                buffered_frame.timestamp,
                                (frame_number + 1) as i64,
                                &buffered_frame.data,
                            ).await {
                                error!("Failed to store pre-recorded frame in database: {}", e);
                            } else {
                                initial_frame_count += 1;
                            }
                        }
                        info!("Fallback completed: stored {} pre-recorded frames for camera '{}'", initial_frame_count, camera_id);
                    }
                }
            }
        }

        // Create active recording entry
        let active_recording = ActiveRecording {
            session_id,
            start_time: recording_start_time,
            frame_count: initial_frame_count,
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
        self.start_recording_task(camera_id.to_string(), session_id, frame_sender, camera_config.clone()).await;

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
        camera_config: crate::config::CameraConfig,
        writer_tx: mpsc::Sender<FrameWriterMessage>,
    ) {
        let mut frame_number = 0i64;
        let mut last_session_check = Utc::now();

        // Determine the effective session segment duration
        // Priority: camera-specific setting > global setting
        // 0 = disabled, None/null = use global, n = minutes
        let effective_session_segment_minutes = match camera_config.get_session_segment_minutes() {
            Some(0) => {
                info!("Session segmentation disabled for camera '{}' (camera override = 0)", camera_id);
                None
            }
            Some(minutes) => {
                info!("Using camera-specific session segmentation for '{}': {} minutes", camera_id, minutes);
                Some(minutes)
            }
            None => {
                if config.session_segment_minutes == 0 {
                    info!("Session segmentation disabled for camera '{}' (global setting = 0)", camera_id);
                    None
                } else {
                    info!("Using global session segmentation for '{}': {} minutes", camera_id, config.session_segment_minutes);
                    Some(config.session_segment_minutes)
                }
            }
        };

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
                        trace!("Recording stopped for camera '{}', ending receiver task", camera_id);
                        // Writer will flush when channel is dropped
                        break;
                    }

                    // Check for session segmentation based on configured interval (if enabled)
                    if let Some(segment_minutes) = effective_session_segment_minutes {
                        let session_segment_duration = chrono::Duration::minutes(segment_minutes as i64);
                        if timestamp.signed_duration_since(last_session_check) >= session_segment_duration {
                            info!("Session segment interval ({} minutes) reached for camera '{}', splitting recording session {}",
                                  segment_minutes, camera_id, session_id);

                            // Signal writer to flush before session split
                            let _ = writer_tx.send(FrameWriterMessage::Flush).await;

                            // Get the recording reason from the database to use for the new session
                            if let Ok(sessions) = database.get_active_recordings(&camera_id).await {
                                if let Some(current_session) = sessions.first() {
                                    let reason = current_session.reason.clone();

                                    // Stop the current session
                                    if let Err(e) = database.stop_recording_session(session_id).await {
                                        error!("Failed to stop recording session for segment split: {}", e);
                                    } else {
                                        info!("Stopped recording session {} for segment split", session_id);

                                        // Create a new session with the same reason
                                        match database.create_recording_session(&camera_id, reason.as_deref(), Utc::now()).await {
                                            Ok(new_session_id) => {
                                                info!("Created new recording session {} for segment continuation", new_session_id);

                                                // Notify writer about session change
                                                let _ = writer_tx.send(FrameWriterMessage::SessionChanged {
                                                    new_session_id,
                                                }).await;

                                                // Update the active recording with new session info
                                                let mut active_recordings_guard = active_recordings.write().await;
                                                if let Some(recording) = active_recordings_guard.get_mut(&camera_id) {
                                                    recording.session_id = new_session_id;
                                                    recording.start_time = timestamp;
                                                    recording.frame_count = 0;
                                                }
                                                drop(active_recordings_guard);

                                                session_id = new_session_id;
                                                frame_number = 1;
                                            }
                                            Err(e) => {
                                                error!("Failed to create new recording session for segment split: {}", e);
                                            }
                                        }
                                    }
                                }
                            }

                            last_session_check = timestamp;
                        }
                    }

                    // Check frame size
                    if frame_data.len() > config.max_frame_size {
                        error!("Frame size {} exceeds maximum {} for camera '{}'",
                                frame_data.len(), config.max_frame_size, camera_id);
                        continue;
                    }

                    // Send frame to writer (non-blocking with try_send for better performance)
                    match writer_tx.try_send(FrameWriterMessage::Frame {
                        session_id,
                        timestamp,
                        frame_number,
                        data: frame_data.to_vec(),
                    }) {
                        Ok(_) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            // Channel full - writer can't keep up, but we don't block
                            warn!("Frame writer channel full for camera '{}', dropping frame", camera_id);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            error!("Frame writer channel closed for camera '{}'", camera_id);
                            break;
                        }
                    }

                    // Update frame count (quick operation, acceptable to await)
                    let mut active_recordings_guard = active_recordings.write().await;
                    if let Some(recording) = active_recordings_guard.get_mut(&camera_id) {
                        recording.frame_count += 1;

                        // Check if duration-based recording should stop
                        if let Some(duration) = recording.requested_duration {
                            let elapsed = timestamp.signed_duration_since(recording.start_time);
                            if elapsed.num_seconds() >= duration {
                                info!("Recording duration reached for camera '{}', stopping", camera_id);
                                drop(active_recordings_guard);
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
        // Dropping writer_tx will signal the writer to flush and exit
    }

    async fn start_recording_task(
        &self,
        camera_id: String,
        session_id: i64,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        camera_config: crate::config::CameraConfig,
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
        
        // Get the effective video storage type for this camera
        let mp4_storage_type = self.get_storage_type_for_camera(&camera_config);
        
        // Get MP4 buffer stats for this camera before spawning
        let mp4_stats = self.get_mp4_buffer_stats(&camera_id).await;

        tokio::spawn(async move {
            let mut tasks = Vec::new();

            if config.frame_storage_enabled {
                // Create mpsc channel for frame writer
                let (writer_tx, writer_rx) = mpsc::channel::<FrameWriterMessage>(WRITER_CHANNEL_BUFFER);

                // Spawn the dedicated database writer task
                let writer_db = database.clone();
                let writer_camera_id = camera_id.clone();
                let writer_task = tokio::spawn(async move {
                    frame_writer_loop(writer_db, writer_camera_id, writer_rx).await;
                });
                tasks.push(writer_task);

                // Spawn the frame receiver task (sends to writer via channel)
                let frame_receiver = frame_sender.subscribe();
                let receiver_task = tokio::spawn(Self::frame_recording_loop(
                    config.clone(),
                    database.clone(),
                    active_recordings.clone(),
                    camera_id.clone(),
                    session_id,
                    frame_receiver,
                    camera_config.clone(),
                    writer_tx,
                ));
                tasks.push(receiver_task);
            }

            if mp4_storage_type != crate::config::Mp4StorageType::Disabled {
                let segmenter_task = tokio::spawn(Self::video_segmenter_loop(
                    config.clone(),
                    database.clone(),
                    active_recordings.clone(),
                    camera_id.clone(),
                    session_id, // Pass session_id
                    frame_sender.subscribe(),
                    mp4_storage_type,
                    mp4_stats,
                ));
                tasks.push(segmenter_task);
            }

            // Check if HLS storage is enabled for this camera
            let hls_enabled = camera_config.get_hls_storage_enabled()
                .unwrap_or(config.hls_storage_enabled);
            
            if hls_enabled {
                let hls_task = tokio::spawn(Self::hls_segmenter_loop(
                    config.clone(),
                    database.clone(),
                    active_recordings.clone(),
                    camera_id.clone(),
                    session_id,
                    frame_sender.subscribe(),
                    camera_config.clone(),
                ));
                tasks.push(hls_task);
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
            database.stop_recording_session(session.session_id).await?;
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

    pub async fn list_recordings_filtered(
        &self,
        camera_id: Option<&str>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        reason: Option<&str>,
    ) -> crate::errors::Result<Vec<RecordingSession>> {
        if let Some(cam_id) = camera_id {
            // Query specific camera's database
            if let Some(database) = self.get_camera_database(cam_id).await {
                database.list_recordings_filtered(cam_id, from, to, reason).await
            } else {
                Ok(Vec::new()) // No database for this camera
            }
        } else {
            // Query all camera databases and combine results
            let databases = self.databases.read().await;
            let mut all_recordings = Vec::new();
            
            for (camera_id, database) in databases.iter() {
                match database.list_recordings_filtered(camera_id, from, to, reason).await {
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
        let camera_configs = self.camera_configs.read().await;
        
        for (camera_id, database) in databases.iter() {
            if let Err(e) = database.cleanup_database(&self.config, &camera_configs).await {
                error!("Failed to cleanup database for camera '{}': {}", camera_id, e);
            }
        }
        Ok(())
    }
    
    pub async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        tolerance_seconds: Option<i64>,
    ) -> crate::errors::Result<Option<RecordedFrame>> {
        // Get the database for this camera
        let database = self.get_camera_database(camera_id).await
            .ok_or_else(|| crate::errors::StreamError::config(&format!("No database found for camera '{}'", camera_id)))?;

        database.get_frame_at_timestamp(camera_id, timestamp, tolerance_seconds).await
    }

    /// Check for active recordings at startup and restart them
    pub async fn restart_active_recordings_at_startup(
        &self,
        camera_frame_senders: &HashMap<String, Arc<broadcast::Sender<Bytes>>>,
        camera_configs: &HashMap<String, crate::config::CameraConfig>,
    ) -> crate::errors::Result<()> {
        // Update camera configs for cleanup
        self.update_camera_configs(camera_configs.clone()).await;

        // Run startup cleanup in background ONLY for PostgreSQL (concurrent-safe)
        // SQLite requires exclusive access for VACUUM, so skip startup cleanup and rely on periodic cleanup
        match self.config.database_type {
            crate::config::DatabaseType::PostgreSQL => {
                info!("Scheduling background cleanup for PostgreSQL databases at startup...");
                let databases_clone = self.databases.clone();
                let config_clone = self.config.clone();
                let camera_configs_clone = self.camera_configs.clone();

                tokio::spawn(async move {
                    // Small delay to let server finish starting up first
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                    info!("Starting background cleanup for all PostgreSQL camera databases...");
                    let databases = databases_clone.read().await;
                    for (camera_id, database) in databases.iter() {
                        info!("Performing background startup cleanup for camera '{}'", camera_id);
                        let configs = camera_configs_clone.read().await;
                        if let Err(e) = database.cleanup_database(&config_clone, &configs).await {
                            error!("Failed to perform startup cleanup for camera '{}': {}", camera_id, e);
                        }
                    }
                    info!("Background startup cleanup completed for all PostgreSQL camera databases");
                });
            }
            crate::config::DatabaseType::SQLite => {
                info!("Skipping startup cleanup for SQLite databases (will run on periodic schedule to avoid locking issues)");
            }
        }

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
                            session.session_id, camera_id
                        );

                        // Create active recording entry to track this session
                        let active_recording = ActiveRecording {
                            session_id: session.session_id,
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
                        if let Some(camera_config) = camera_configs.get(camera_id) {
                            self.start_recording_task(camera_id.clone(), session.session_id, frame_sender.clone(), camera_config.clone()).await;
                        } else {
                            error!("Camera config not found for camera '{}', skipping recording restart", camera_id);
                            continue;
                        }

                        restarted_count += 1;
                        info!(
                            "Restarted recording for camera '{}' with session ID {}",
                            camera_id, session.session_id
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

    pub async fn list_video_segments_filtered(
        &self,
        camera_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        reason: Option<&str>,
        limit: i64,
        sort_order: &str,
    ) -> crate::errors::Result<Vec<VideoSegment>> {
        if let Some(database) = self.get_camera_database(camera_id).await {
            database.list_video_segments_filtered(camera_id, from, to, reason, limit, sort_order).await
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
        session_id: i64, // Add session_id parameter
        mut frame_receiver: broadcast::Receiver<Bytes>,
        mp4_storage_type: crate::config::Mp4StorageType,
        mp4_buffer_stats: Option<Arc<tokio::sync::RwLock<crate::Mp4BufferStats>>>,
    ) {
        let segment_duration = chrono::Duration::minutes(config.mp4_segment_minutes as i64);
        
        // Get recording start time (which may include pre-recorded frames)
        let mut segment_start_time = {
            if let Some(active_recording) = active_recordings.read().await.get(&camera_id) {
                active_recording.start_time
            } else {
                Utc::now()
            }
        };
        
        let mut frame_buffer = Vec::new();

        // Track current session_id - may change due to session segmentation
        let mut current_session_id = session_id;

        // Process any pre-recorded frames first if they exist
        if let Some(active_recording) = active_recordings.read().await.get(&camera_id) {
            if active_recording.frame_count > 0 {
                info!("Processing {} pre-recorded frames for MP4 segmentation", active_recording.frame_count);
                
                // Get pre-recorded frames from database
                match database.get_recorded_frames(
                    active_recording.session_id, 
                    Some(active_recording.start_time),
                    None  // Get all frames from start time onwards
                ).await {
                    Ok(recorded_frames) => {
                        if !recorded_frames.is_empty() {
                            info!("Found {} pre-recorded frames for MP4 segmentation", recorded_frames.len());
                            
                            // Convert RecordedFrame to Bytes for processing
                            let pre_recorded_frame_data: Vec<Bytes> = recorded_frames.into_iter()
                                .map(|frame| Bytes::from(frame.frame_data))
                                .collect();
                            
                            // Add pre-recorded frames to buffer
                            frame_buffer.extend(pre_recorded_frame_data);
                            
                            // Update MP4 buffer stats
                            if let Some(ref stats) = mp4_buffer_stats {
                                let buffer_size = frame_buffer.iter().map(|f| f.len()).sum::<usize>();
                                let mut stats = stats.write().await;
                                stats.frame_count = frame_buffer.len();
                                stats.size_bytes = buffer_size;
                            }
                            
                            info!("Added {} pre-recorded frames to MP4 segment buffer", frame_buffer.len());
                        }
                    }
                    Err(e) => {
                        error!("Failed to retrieve pre-recorded frames for MP4 segmentation: {}", e);
                    }
                }
            }
        }

        loop {
            match frame_receiver.recv().await {
                Ok(frame_data) => {
                    // Check if recording is still active
                    if !active_recordings.read().await.contains_key(&camera_id) {
                        trace!("Recording stopped for camera '{}', ending segmenter task", camera_id);
                        
                        // Flush remaining frames in buffer before stopping
                        if !frame_buffer.is_empty() {
                            info!("Flushing {} remaining frames from MP4 buffer on recording stop for camera '{}'", frame_buffer.len(), camera_id);
                            let frames_to_process = std::mem::take(&mut frame_buffer);
                            let end_time = Utc::now();

                            // Update buffer stats to show empty buffer
                            if let Some(ref stats) = mp4_buffer_stats {
                                let mut stats = stats.write().await;
                                stats.frame_count = 0;
                                stats.size_bytes = 0;
                            }

                            // Spawn a task to process the final segment with current session_id
                            let final_config = config.clone();
                            let final_database = database.clone();
                            let final_camera_id = camera_id.clone();
                            let final_session_id = current_session_id;
                            let final_storage_type = mp4_storage_type.clone();
                            let log_camera_id = camera_id.clone(); // Clone for logging
                            tokio::spawn(async move {
                                if let Err(e) = Self::create_video_segment(
                                    final_config,
                                    final_database,
                                    final_camera_id,
                                    final_session_id,
                                    segment_start_time,
                                    end_time,
                                    frames_to_process,
                                    final_storage_type,
                                ).await {
                                    error!("Failed to create final video segment on recording stop: {}", e);
                                } else {
                                    info!("Successfully created final video segment on recording stop for camera '{}'", log_camera_id);
                                }
                            });
                        }
                        break;
                    }

                    frame_buffer.push(frame_data);

                    // Update MP4 buffer stats
                    if let Some(ref stats) = mp4_buffer_stats {
                        let buffer_size = frame_buffer.iter().map(|f| f.len()).sum::<usize>();
                        let mut stats = stats.write().await;
                        stats.frame_count = frame_buffer.len();
                        stats.size_bytes = buffer_size;
                    }

                    if Utc::now().signed_duration_since(segment_start_time) >= segment_duration {
                        let frames_to_process = std::mem::take(&mut frame_buffer);

                        // Update buffer stats after taking frames
                        if let Some(ref stats) = mp4_buffer_stats {
                            let buffer_size = frame_buffer.iter().map(|f| f.len()).sum::<usize>();
                            let mut stats = stats.write().await;
                            stats.frame_count = frame_buffer.len();
                            stats.size_bytes = buffer_size;
                        }
                        let end_time = Utc::now();

                        // Check if session has changed (due to session segmentation)
                        let new_session_id = active_recordings.read().await
                            .get(&camera_id)
                            .map(|r| r.session_id)
                            .unwrap_or(current_session_id);

                        if new_session_id != current_session_id {
                            info!("MP4 segmenter detected session change {} -> {} for camera '{}'",
                                  current_session_id, new_session_id, camera_id);
                            current_session_id = new_session_id;
                        }

                        // Spawn a task to process the segment
                        let task_config = config.clone();
                        let task_database = database.clone();
                        let task_camera_id = camera_id.clone();
                        let task_session_id = current_session_id;
                        let task_storage_type = mp4_storage_type.clone();
                        tokio::spawn(async move {
                            if let Err(e) = Self::create_video_segment(
                                task_config,
                                task_database,
                                task_camera_id,
                                task_session_id,
                                segment_start_time,
                                end_time,
                                frames_to_process,
                                task_storage_type,
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
        camera_id: String,  // Only needed for filesystem path
        session_id: i64,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        frames: Vec<Bytes>,
        mp4_storage_type: crate::config::Mp4StorageType,
    ) -> crate::errors::Result<()> {
        if frames.is_empty() {
            return Ok(());
        }

        // Create video segment based on storage type
        if mp4_storage_type == crate::config::Mp4StorageType::Database {
            // Store MP4 data in database as BLOB
            Self::create_database_video_segment(config.clone(), database, camera_id, session_id, start_time, end_time, frames).await
        } else {
            // Store MP4 file on filesystem
            Self::create_filesystem_video_segment(config.clone(), database, camera_id, session_id, start_time, end_time, frames).await
        }
    }

    async fn create_filesystem_video_segment(
        config: Arc<RecordingConfig>,
        database: Arc<dyn DatabaseProvider>,
        camera_id: String,
        session_id: i64,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        frames: Vec<Bytes>,
    ) -> crate::errors::Result<()> {
        let recordings_dir = config.get_mp4_storage_path();

        // Create hierarchical directory structure: recordings/cam1/2025/08/19/
        let year = start_time.year();
        let month = start_time.month();
        let day = start_time.day();

        let camera_dir = format!("{}/{}/{:04}/{:02}/{:02}",
            recordings_dir, camera_id, year, month, day);

        // Create directory structure if it doesn't exist
        if let Err(e) = tokio::fs::create_dir_all(&camera_dir).await {
            error!("Failed to create directory structure '{}': {}", camera_dir, e);
            return Err(crate::errors::StreamError::Io { source: e });
        }

        // Format timestamp for filename (filesystem-safe)
        let iso_timestamp = if config.mp4_filename_use_local_time {
            start_time.with_timezone(&Local).format("%Y-%m-%dT%H-%M-%S").to_string()
        } else {
            format!("{}Z", start_time.format("%Y-%m-%dT%H-%M-%S"))
        };

        let filename_stem = if config.mp4_filename_include_reason {
            match database.get_session_reason(session_id).await {
                Ok(Some(r)) => match sanitize_reason_for_filename(&r) {
                    Some(sanitized) => format!("{}_{}", iso_timestamp, sanitized),
                    None => iso_timestamp.to_string(),
                },
                _ => iso_timestamp.to_string(),
            }
        } else {
            iso_timestamp.to_string()
        };

        let file_path = format!("{}/{}.mp4", camera_dir, filename_stem);

        // Calculate actual framerate from frame count and duration
        let duration_secs = (end_time - start_time).num_milliseconds() as f32 / 1000.0;
        let actual_framerate = if duration_secs > 0.1 { // At least 100ms duration
            frames.len() as f32 / duration_secs
        } else {
            warn!("Invalid segment duration {:.3}s for camera '{}', using fallback framerate 10.0",
                  duration_secs, camera_id);
            10.0 // Fallback - should rarely happen
        };

        debug!("Creating MP4 segment for camera '{}': {} frames over {:.2}s = {:.2} FPS",
               camera_id, frames.len(), duration_secs, actual_framerate);

        let mp4_data = Self::create_mp4_from_frames(frames, actual_framerate).await?;
        
        // Write MP4 data to file
        tokio::fs::write(&file_path, &mp4_data).await?;
        
        let segment = VideoSegment {
            camera_id: camera_id.clone(),
            session_id,
            start_time,
            end_time,
            file_path: Some(file_path),
            size_bytes: mp4_data.len() as i64,
            mp4_data: None, // No blob data for filesystem storage
            recording_reason: None, // Will be filled by the database query when retrieved
        };

        database.add_video_segment(&segment).await?;
        Ok(())
    }

    async fn create_database_video_segment(
        _config: Arc<RecordingConfig>,
        database: Arc<dyn DatabaseProvider>,
        camera_id: String,
        session_id: i64,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        frames: Vec<Bytes>,
    ) -> crate::errors::Result<()> {
        // Calculate actual framerate from frame count and duration
        let duration_secs = (end_time - start_time).num_milliseconds() as f32 / 1000.0;
        let actual_framerate = if duration_secs > 0.1 { // At least 100ms duration
            frames.len() as f32 / duration_secs
        } else {
            warn!("Invalid segment duration {:.3}s for camera '{}', using fallback framerate 10.0",
                  duration_secs, camera_id);
            10.0 // Fallback - should rarely happen
        };

        debug!("Creating MP4 segment for camera '{}': {} frames over {:.2}s = {:.2} FPS",
               camera_id, frames.len(), duration_secs, actual_framerate);

        let mp4_data = Self::create_mp4_from_frames(frames, actual_framerate).await?;
        
        let segment = VideoSegment {
            camera_id: camera_id.clone(),
            session_id,
            start_time,
            end_time,
            file_path: None, // No file path for database storage
            size_bytes: mp4_data.len() as i64,
            mp4_data: Some(mp4_data), // Store as BLOB
            recording_reason: None, // Will be filled by the database query when retrieved
        };

        database.add_video_segment(&segment).await?;
        Ok(())
    }
    
    async fn create_mp4_from_frames(frames: Vec<Bytes>, framerate: f32) -> crate::errors::Result<Vec<u8>> {
        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-f", "mjpeg",
            "-framerate", &framerate.to_string(), // Input framerate
            "-i", "-",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            // No output framerate - use same as input
            "-f", "mp4", // Output format
            "-movflags", "frag_keyframe+empty_moov", // Enable streaming-friendly MP4
            "-", // Output to stdout
        ]);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().expect("Failed to open ffmpeg stdin");
        let stdout = child.stdout.take().expect("Failed to open ffmpeg stdout");

        // Write frames to FFmpeg stdin
        let write_task = tokio::spawn(async move {
            for frame in frames {
                if let Err(e) = stdin.write_all(&frame).await {
                    error!("Failed to write frame to ffmpeg stdin: {}", e);
                    break;
                }
            }
            drop(stdin);
        });

        // Read MP4 data from FFmpeg stdout
        let read_task = tokio::spawn(async move {
            let mut output = Vec::new();
            let mut reader = tokio::io::BufReader::new(stdout);
            tokio::io::copy(&mut reader, &mut output).await.map(|_| output)
        });

        let status = child.wait().await?;
        write_task.await.map_err(|e| crate::errors::StreamError::server(format!("Task join error: {}", e)))?;
        
        if !status.success() {
            return Err(crate::errors::StreamError::ffmpeg("ffmpeg command failed"));
        }
        
        let mp4_data = read_task.await.map_err(|e| crate::errors::StreamError::server(format!("Task join error: {}", e)))??;
        
        Ok(mp4_data)
    }

    async fn hls_segmenter_loop(
        config: Arc<RecordingConfig>,
        database: Arc<dyn DatabaseProvider>,
        active_recordings: Arc<RwLock<HashMap<String, ActiveRecording>>>,
        camera_id: String,
        session_id: i64,
        mut frame_receiver: broadcast::Receiver<Bytes>,
        camera_config: crate::config::CameraConfig,
    ) {
        // Get HLS segment duration (default 6 seconds)
        let segment_seconds = camera_config.get_hls_segment_seconds()
            .unwrap_or(config.hls_segment_seconds);
        
        let segment_duration = chrono::Duration::seconds(segment_seconds as i64);
        
        // Get recording start time (which may include pre-recorded frames)
        let mut segment_start_time = {
            if let Some(active_recording) = active_recordings.read().await.get(&camera_id) {
                active_recording.start_time
            } else {
                Utc::now()
            }
        };
        let mut frame_buffer = Vec::new();
        
        // Get the last segment index for this session to avoid duplicates
        let mut segment_index = match database.get_last_hls_segment_index_for_session(session_id).await {
            Ok(Some(last_index)) => {
                info!("Resuming HLS segments from index {} for session {}", last_index + 1, session_id);
                last_index + 1
            }
            Ok(None) => {
                info!("Starting new HLS segment sequence for session {}", session_id);
                0
            }
            Err(e) => {
                warn!("Failed to get last HLS segment index for session {}, starting from 0: {}", session_id, e);
                0
            }
        };

        // Track current session_id - may change due to session segmentation
        let mut current_session_id = session_id;

        // Process any pre-recorded frames first if they exist
        if let Some(active_recording) = active_recordings.read().await.get(&camera_id) {
            if active_recording.frame_count > 0 {
                info!("Processing {} pre-recorded frames for HLS segmentation", active_recording.frame_count);
                
                // Get pre-recorded frames from database
                match database.get_recorded_frames(
                    session_id, 
                    Some(active_recording.start_time),
                    None  // Get all frames from start time onwards
                ).await {
                    Ok(recorded_frames) => {
                        if !recorded_frames.is_empty() {
                            info!("Found {} pre-recorded frames for HLS segmentation", recorded_frames.len());
                            
                            // Convert RecordedFrame to Bytes for processing
                            let pre_recorded_frame_data: Vec<Bytes> = recorded_frames.into_iter()
                                .map(|frame| Bytes::from(frame.frame_data))
                                .collect();
                            
                            // Add pre-recorded frames to buffer
                            frame_buffer.extend(pre_recorded_frame_data);
                            
                            info!("Added {} pre-recorded frames to HLS segment buffer", frame_buffer.len());
                        }
                    }
                    Err(e) => {
                        error!("Failed to retrieve pre-recorded frames for HLS segmentation: {}", e);
                    }
                }
            }
        }

        info!("Starting HLS segmenter for camera '{}' with {} second segments, starting at index {}", 
              camera_id, segment_seconds, segment_index);
        loop {
            match frame_receiver.recv().await {
                Ok(frame_data) => {
                    // Check if recording is still active
                    if !active_recordings.read().await.contains_key(&camera_id) {
                        trace!("Recording stopped for camera '{}', ending HLS segmenter task", camera_id);
                        
                        // Flush remaining frames in buffer before stopping
                        if !frame_buffer.is_empty() {
                            info!("Flushing {} remaining frames from HLS buffer on recording stop", frame_buffer.len());
                            let frames_to_process = std::mem::take(&mut frame_buffer);
                            let end_time = Utc::now();

                            // Create final HLS segment with current session_id
                            let final_config = config.clone();
                            let final_database = database.clone();
                            let final_camera_id = camera_id.clone();
                            let final_session_id = current_session_id;
                            let final_segment_index = segment_index;
                            tokio::spawn(async move {
                                if let Err(e) = Self::create_hls_segment(
                                    final_config,
                                    final_database,
                                    final_camera_id,
                                    final_session_id,
                                    final_segment_index,
                                    segment_start_time,
                                    end_time,
                                    frames_to_process,
                                ).await {
                                    error!("Failed to create final HLS segment on recording stop: {}", e);
                                } else {
                                    info!("Successfully created final HLS segment on recording stop");
                                }
                            });
                        }
                        break;
                    }

                    frame_buffer.push(frame_data);

                    let elapsed = Utc::now().signed_duration_since(segment_start_time);
                    if elapsed >= segment_duration {
                        let frames_to_process = std::mem::take(&mut frame_buffer);
                        let end_time = Utc::now();

                        // Check if session has changed (due to session segmentation)
                        let new_session_id = active_recordings.read().await
                            .get(&camera_id)
                            .map(|r| r.session_id)
                            .unwrap_or(current_session_id);

                        if new_session_id != current_session_id {
                            info!("HLS segmenter detected session change {} -> {} for camera '{}'",
                                  current_session_id, new_session_id, camera_id);
                            current_session_id = new_session_id;
                            segment_index = 0;  // Reset segment index for new session
                        }

                        // Spawn a task to process the HLS segment
                        let task_config = config.clone();
                        let task_database = database.clone();
                        let task_camera_id = camera_id.clone();
                        let task_session_id = current_session_id;
                        let current_segment_index = segment_index;
                        let current_start_time = segment_start_time;

                        tokio::spawn(async move {
                            if let Err(e) = Self::create_hls_segment(
                                task_config,
                                task_database,
                                task_camera_id,
                                task_session_id,
                                current_segment_index,
                                current_start_time,
                                end_time,
                                frames_to_process,
                            ).await {
                                error!("Failed to create HLS segment: {}", e);
                            }
                        });

                        segment_start_time = end_time;
                        segment_index += 1;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!("HLS segmenter lagged for camera '{}', skipped {} frames", camera_id, skipped);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Frame channel closed for camera '{}', stopping HLS segmenter", camera_id);
                    break;
                }
            }
        }

        info!("HLS segmenter ended for camera '{}' session {}", camera_id, session_id);
    }

    async fn create_hls_segment(
        config: Arc<RecordingConfig>,
        database: Arc<dyn DatabaseProvider>,
        camera_id: String,
        session_id: i64,
        segment_index: i32,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        frames: Vec<Bytes>,
    ) -> crate::errors::Result<()> {
        if frames.is_empty() {
            return Ok(());
        }

        trace!("Creating HLS segment {} for camera '{}' session {} with {} frames",
               segment_index, camera_id, session_id, frames.len());

        // Calculate actual framerate from frame count and duration
        let duration_secs = (end_time - start_time).num_milliseconds() as f32 / 1000.0;
        let actual_framerate = if duration_secs > 0.1 { // At least 100ms duration
            frames.len() as f32 / duration_secs
        } else {
            warn!("Invalid HLS segment duration {:.3}s for camera '{}', using fallback framerate 10.0",
                  duration_secs, camera_id);
            10.0 // Fallback - should rarely happen
        };

        debug!("Creating HLS segment {} for camera '{}': {} frames over {:.2}s = {:.2} FPS",
               segment_index, camera_id, frames.len(), duration_secs, actual_framerate);

        // Convert frames to MPEG-TS segment using FFmpeg
        let segment_data = Self::create_hls_segment_from_frames(config.clone(), frames, actual_framerate).await?;
        
        if segment_data.is_empty() {
            warn!("Generated empty HLS segment for camera '{}' segment {}", camera_id, segment_index);
            return Ok(());
        }

        // Calculate segment duration in seconds
        let duration_seconds = (end_time.timestamp_millis() - start_time.timestamp_millis()) as f64 / 1000.0;
        let size_bytes = segment_data.len() as i64;

        // Create HLS segment struct
        let hls_segment = RecordingHlsSegment {
            camera_id: camera_id.clone(),
            session_id,
            segment_index,
            start_time,
            end_time,
            duration_seconds,
            segment_data,
            size_bytes,
            created_at: Utc::now(),
        };

        // Store segment in database with better error handling
        match database.add_recording_hls_segment(&hls_segment).await {
            Ok(_) => {
                debug!("Stored HLS segment {} for camera '{}' session {} ({} bytes, {:.2}s duration)", 
                      segment_index, camera_id, session_id, size_bytes, duration_seconds);
            }
            Err(e) => {
                // Check if this is a unique constraint violation
                if e.to_string().contains("UNIQUE constraint failed") {
                    warn!("HLS segment {} for session {} already exists, skipping duplicate insert", 
                          segment_index, session_id);
                    // This is not a fatal error - the segment already exists
                } else {
                    // For other errors, propagate them
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    async fn create_hls_segment_from_frames(
        _config: Arc<RecordingConfig>,
        frames: Vec<Bytes>,
        framerate: f32,
    ) -> crate::errors::Result<Vec<u8>> {
        use tokio::process::Command;

        let mut cmd = Command::new("ffmpeg");

        // Configure FFmpeg to create MPEG-TS segment from MJPEG frames
        cmd.args([
            "-f", "mjpeg",
            "-framerate", &framerate.to_string(), // Input framerate
            "-i", "-", // Input from stdin
            "-c:v", "libx264", // H.264 codec
            "-preset", "ultrafast", // Fast encoding
            "-f", "mpegts", // MPEG-TS format for HLS
            "-", // Output to stdout
        ]);
        
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().expect("Failed to open ffmpeg stdin");
        let stdout = child.stdout.take().expect("Failed to open ffmpeg stdout");

        // Write frames to FFmpeg stdin
        let write_task = tokio::spawn(async move {
            for frame in frames {
                if let Err(e) = stdin.write_all(&frame).await {
                    error!("Failed to write frame to ffmpeg stdin: {}", e);
                    break;
                }
            }
            drop(stdin);
        });

        // Read MPEG-TS data from FFmpeg stdout
        let read_task = tokio::spawn(async move {
            let mut output = Vec::new();
            let mut reader = tokio::io::BufReader::new(stdout);
            tokio::io::copy(&mut reader, &mut output).await.map(|_| output)
        });

        let status = child.wait().await?;
        write_task.await.map_err(|e| crate::errors::StreamError::server(format!("Task join error: {}", e)))?;
        
        if !status.success() {
            return Err(crate::errors::StreamError::ffmpeg("ffmpeg command failed for HLS segment"));
        }
        
        let segment_data = read_task.await.map_err(|e| crate::errors::StreamError::server(format!("Task join error: {}", e)))??;
        
        Ok(segment_data)
    }
}
