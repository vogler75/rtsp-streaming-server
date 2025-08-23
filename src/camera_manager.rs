use std::sync::Arc;
use tracing::{info, error, warn};

use crate::config;
use crate::errors::Result;
use crate::video_stream::VideoStream;
use crate::database::{DatabaseProvider, SqliteDatabase};
use crate::{AppState, CameraStreamInfo};

impl AppState {
    pub async fn add_camera(&self, camera_id: String, camera_config: config::CameraConfig) -> Result<()> {
        // Check if camera is enabled first (before acquiring any locks)
        let is_enabled = camera_config.enabled.unwrap_or(true);
        
        // Always update camera configuration (enabled or disabled)
        {
            let mut camera_configs = self.camera_configs.write().await;
            camera_configs.insert(camera_id.clone(), camera_config.clone());
            
            // Update recording manager with new camera configs
            if let Some(ref recording_manager) = self.recording_manager {
                recording_manager.update_camera_configs(camera_configs.clone()).await;
            }
        }
        
        if !is_enabled {
            info!("Camera '{}' is disabled, config updated but not starting stream", camera_id);
            // Remove from active streams if it was previously enabled (separate lock scope)
            let stream_info_to_stop = {
                let mut camera_streams = self.camera_streams.write().await;
                camera_streams.remove(&camera_id)
            };
            
            // Stop the video stream outside the lock
            if let Some(stream_info) = stream_info_to_stop {
                info!("Removed disabled camera '{}' from active streams", camera_id);
                if let Some(task_handle) = stream_info.task_handle {
                    task_handle.abort();
                }
            }
            return Ok(());
        }
        
        // Check if camera stream already exists (separate lock scope)
        {
            let camera_streams = self.camera_streams.read().await;
            if camera_streams.contains_key(&camera_id) {
                info!("Camera '{}' stream already exists, updating config only", camera_id);
                return Ok(());
            }
        }
        
        info!("Adding camera '{}' on path '{}'...", camera_id, camera_config.path);
        
        // Create video stream
        match VideoStream::new(
            camera_id.clone(),
            camera_config.clone(),
            &self.transcoding_config,
            self.mqtt_handle.clone(),
        ).await {
            Ok(video_stream) => {
                // Create database for this camera if recording is enabled
                if let Some(ref recording_manager_ref) = &self.recording_manager {
                    if let Some(recording_config) = &self.recording_config {
                        let camera_db_path = format!("{}/{}.db", recording_config.database_path, camera_id);
                        info!("Creating database for camera '{}' at '{}'", camera_id, camera_db_path);
                        
                        match SqliteDatabase::new(&camera_db_path).await {
                            Ok(database) => {
                                let database: Arc<dyn DatabaseProvider> = Arc::new(database);
                                if let Err(e) = recording_manager_ref.add_camera_database(&camera_id, database).await {
                                    error!("Failed to add database for camera '{}': {}", camera_id, e);
                                } else {
                                    info!("Database created successfully for camera '{}'", camera_id);
                                }
                            }
                            Err(e) => {
                                error!("Failed to create database for camera '{}': {}", camera_id, e);
                            }
                        }
                    }
                }

                // Extract frame sender and fps counter before starting (since start() consumes the video_stream)
                let frame_sender = video_stream.frame_sender.clone();
                let fps_counter = video_stream.get_fps_counter();
                
                // Start the video stream and get the task handle
                let task_handle = video_stream.start().await;
                
                // Store the camera stream info
                let camera_stream_info = CameraStreamInfo {
                    camera_id: camera_id.clone(),
                    frame_sender,
                    mqtt_handle: self.mqtt_handle.clone(),
                    camera_config: camera_config.clone(),
                    recording_manager: self.recording_manager.clone(),
                    task_handle: Some(Arc::new(task_handle)),
                    capture_fps: fps_counter,
                };
                
                // Add to camera streams
                {
                    let mut camera_streams = self.camera_streams.write().await;
                    camera_streams.insert(camera_id.clone(), camera_stream_info);
                }
                
                info!("Camera '{}' added and started successfully", camera_id);
                Ok(())
            }
            Err(e) => {
                error!("Failed to create video stream for camera '{}': {}", camera_id, e);
                Err(e)
            }
        }
    }
    
    pub async fn remove_camera(&self, camera_id: &str) -> Result<()> {
        info!("Removing camera '{}'...", camera_id);
        
        // Remove from camera configurations
        {
            let mut camera_configs = self.camera_configs.write().await;
            camera_configs.remove(camera_id);
            
            // Update recording manager with updated camera configs
            if let Some(ref recording_manager) = self.recording_manager {
                recording_manager.update_camera_configs(camera_configs.clone()).await;
            }
        }
        
        // Remove from camera streams and get the camera info for cleanup
        let removed = {
            let mut camera_streams = self.camera_streams.write().await;
            camera_streams.remove(camera_id)
        };
        
        if let Some(camera_info) = removed {
            // Stop and abort the video stream task
            if let Some(task_handle) = camera_info.task_handle {
                info!("Cancelling video stream task for camera '{}'", camera_id);
                task_handle.abort();
                
                // Wait a bit for the task to terminate
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
            
            // Stop recording if active
            if let Some(ref recording_manager_ref) = &self.recording_manager {
                info!("Stopping any active recordings for camera '{}'", camera_id);
                if let Err(e) = recording_manager_ref.stop_recording(camera_id).await {
                    error!("Failed to stop recording for camera '{}': {}", camera_id, e);
                }
            }
            
            // The frame_sender will be dropped which will close all WebSocket connections
            // for this camera automatically when the last reference is dropped
            info!("Frame sender dropped for camera '{}' - WebSocket connections will close", camera_id);
            
            info!("Camera '{}' removed successfully", camera_id);
            Ok(())
        } else {
            warn!("Camera '{}' was not found in active streams", camera_id);
            Ok(())
        }
    }
    
    pub async fn restart_camera(&self, camera_id: String, camera_config: config::CameraConfig) -> Result<()> {
        info!("Restarting camera '{}'...", camera_id);
        
        // Check if recording is active before removing the camera
        let was_recording = if let Some(ref recording_manager_ref) = &self.recording_manager {
            let active_recording = recording_manager_ref.get_active_recording(&camera_id).await;
            if let Some(recording) = active_recording {
                info!("Camera '{}' has active recording (session {}), will restart after camera restart", camera_id, recording.session_id);
                
                // Try to get the original recording reason from the database
                let original_reason = match recording_manager_ref.list_recordings(
                    Some(&camera_id), 
                    Some(recording.start_time), 
                    None
                ).await {
                    Ok(sessions) => {
                        sessions.into_iter()
                            .find(|s| s.id == recording.session_id)
                            .and_then(|s| s.reason)
                            .unwrap_or_else(|| "Camera restart".to_string())
                    }
                    Err(_) => "Camera restart".to_string()
                };
                
                Some((recording.requested_duration, original_reason.to_string()))
            } else {
                None
            }
        } else {
            None
        };
        
        // Remove the old camera
        self.remove_camera(&camera_id).await?;
        
        // Add the new camera with updated config
        self.add_camera(camera_id.clone(), camera_config.clone()).await?;
        
        // Restart recording if it was previously active
        if let Some((requested_duration, reason)) = was_recording {
            info!("Restarting recording for camera '{}' after restart", camera_id);
            if let Some(ref recording_manager_ref) = &self.recording_manager {
                // Get the frame sender for this camera
                if let Some(frame_sender) = {
                    let camera_streams = self.camera_streams.read().await;
                    camera_streams.get(&camera_id).map(|info| info.frame_sender.clone())
                } {
                    match recording_manager_ref.start_recording(
                        &camera_id,
                        "system", // client_id for system restarts
                        Some(&reason),
                        requested_duration,
                        frame_sender,
                        &camera_config,
                    ).await {
                        Ok(session_id) => {
                            info!("Successfully restarted recording for camera '{}' with session ID {}", camera_id, session_id);
                        }
                        Err(e) => {
                            error!("Failed to restart recording for camera '{}': {}", camera_id, e);
                        }
                    }
                } else {
                    error!("No frame sender found for camera '{}' after restart", camera_id);
                }
            }
        }
        
        Ok(())
    }
}