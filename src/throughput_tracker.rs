use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, OnceCell};
static GLOBAL_THROUGHPUT_TRACKER: OnceCell<Arc<ThroughputTracker>> = OnceCell::const_new();
use tokio::time::{Duration, interval};
use tracing::{info, error, debug};
use chrono::Utc;

use crate::database::DatabaseProvider;
use crate::mqtt::{MqttHandle, ThroughputStats as MqttThroughputStats};

#[derive(Debug, Clone)]
pub struct ThroughputStats {
    pub bytes_per_second: i64,
    pub frame_count: i32,
    pub ffmpeg_fps: f32,
    pub connection_count: i32,
}

#[derive(Debug)]
struct CameraThroughputData {
    bytes_this_second: i64,
    frames_this_second: i32,
    last_ffmpeg_fps: f32,
    last_connection_count: i32,
}

impl CameraThroughputData {
    fn new() -> Self {
        Self {
            bytes_this_second: 0,
            frames_this_second: 0,
            last_ffmpeg_fps: 0.0,
            last_connection_count: 0,
        }
    }
    
    fn reset(&mut self) {
        self.bytes_this_second = 0;
        self.frames_this_second = 0;
        // Keep last_ffmpeg_fps and last_connection_count for the next interval
    }
}

pub struct ThroughputTracker {
    cameras: Arc<RwLock<HashMap<String, Arc<RwLock<CameraThroughputData>>>>>,
    databases: Arc<RwLock<HashMap<String, Arc<dyn DatabaseProvider>>>>,
    mqtt_handle: Option<MqttHandle>,
    database_logging_enabled: bool,
}

impl ThroughputTracker {
    pub fn new_with_mqtt(mqtt_handle: Option<MqttHandle>, database_logging_enabled: bool) -> Self {
        Self {
            cameras: Arc::new(RwLock::new(HashMap::new())),
            databases: Arc::new(RwLock::new(HashMap::new())),
            mqtt_handle,
            database_logging_enabled,
        }
    }
    
    /// Register a camera for throughput tracking
    pub async fn register_camera(&self, camera_id: &str) {
        let mut cameras = self.cameras.write().await;
        cameras.insert(
            camera_id.to_string(), 
            Arc::new(RwLock::new(CameraThroughputData::new()))
        );
        info!("Registered camera '{}' for throughput tracking", camera_id);
    }
    
    /// Add a database for a specific camera
    pub async fn add_camera_database(&self, camera_id: &str, database: Arc<dyn DatabaseProvider>) {
        let mut databases = self.databases.write().await;
        databases.insert(camera_id.to_string(), database);
    }
    
    /// Record frame processing for a camera
    pub async fn record_frame(&self, camera_id: &str, frame_size: i64) {
        let cameras = self.cameras.read().await;
        if let Some(camera_data) = cameras.get(camera_id) {
            let mut data = camera_data.write().await;
            data.bytes_this_second += frame_size;
            data.frames_this_second += 1;
        }
    }
    
    /// Update FFmpeg FPS for a camera
    pub async fn update_ffmpeg_fps(&self, camera_id: &str, fps: f32) {
        let cameras = self.cameras.read().await;
        if let Some(camera_data) = cameras.get(camera_id) {
            let mut data = camera_data.write().await;
            data.last_ffmpeg_fps = fps;
        }
    }
    
    /// Update connection count for a camera
    pub async fn update_connection_count(&self, camera_id: &str, count: i32) {
        let cameras = self.cameras.read().await;
        if let Some(camera_data) = cameras.get(camera_id) {
            let mut data = camera_data.write().await;
            data.last_connection_count = count;
        }
    }
    
    /// Start the throughput tracking task that runs every second
    pub async fn start_tracking_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(1));
            info!("Started throughput tracking task - recording every second");
            
            loop {
                interval.tick().await;
                
                if let Err(e) = self.record_throughput_stats().await {
                    error!("Failed to record throughput stats: {}", e);
                }
            }
        })
    }
    
    /// Record throughput statistics for all cameras
    async fn record_throughput_stats(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cameras = self.cameras.read().await;
        let databases = self.databases.read().await;
        let now = Utc::now();
        
        for (camera_id, camera_data_arc) in cameras.iter() {
            let mut camera_data = camera_data_arc.write().await;
            
            // Only record if we have processed any frames
            if camera_data.frames_this_second > 0 {
                let stats = ThroughputStats {
                    bytes_per_second: camera_data.bytes_this_second,
                    frame_count: camera_data.frames_this_second,
                    ffmpeg_fps: camera_data.last_ffmpeg_fps,
                    connection_count: camera_data.last_connection_count,
                };
                
                debug!(
                    "Throughput for camera '{}': {} bytes/s, {} frames, {:.1} fps, {} connections (DB: {}, MQTT: {})",
                    camera_id, stats.bytes_per_second, stats.frame_count, stats.ffmpeg_fps, stats.connection_count,
                    if self.database_logging_enabled { "enabled" } else { "disabled" },
                    if self.mqtt_handle.is_some() { "enabled" } else { "disabled" }
                );
                
                // Record to database if enabled and database is available
                if self.database_logging_enabled {
                    if let Some(database) = databases.get(camera_id) {
                        if let Err(e) = database.record_throughput_stats(
                            camera_id,
                            now,
                            stats.bytes_per_second,
                            stats.frame_count,
                            stats.ffmpeg_fps,
                            stats.connection_count,
                        ).await {
                            error!("Failed to record throughput stats for camera '{}': {}", camera_id, e);
                        }
                    } else {
                        debug!("Database logging enabled but no database available for camera '{}', skipping throughput recording", camera_id);
                    }
                }
                
                // Publish to MQTT if available
                if let Some(ref mqtt_handle) = self.mqtt_handle {
                    let mqtt_stats = MqttThroughputStats {
                        bytes_per_second: stats.bytes_per_second,
                        frame_count: stats.frame_count,
                        ffmpeg_fps: stats.ffmpeg_fps,
                        connection_count: stats.connection_count,
                        timestamp: now.to_rfc3339(),
                    };
                    
                    if let Err(e) = mqtt_handle.publish_throughput_stats(camera_id, &mqtt_stats).await {
                        error!("Failed to publish throughput stats to MQTT for camera '{}': {}", camera_id, e);
                    }
                }
            }
            
            // Reset counters for next second
            camera_data.reset();
        }
        
        Ok(())
    }
    
    /// Cleanup old throughput statistics (older than specified duration)
    #[allow(dead_code)]
    pub async fn cleanup_old_stats(&self, retention_days: u32) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let databases = self.databases.read().await;
        let cutoff_time = Utc::now() - chrono::Duration::days(retention_days as i64);
        let mut total_deleted = 0u64;
        
        for (camera_id, database) in databases.iter() {
            match database.cleanup_old_throughput_stats(cutoff_time).await {
                Ok(deleted) => {
                    if deleted > 0 {
                        info!("Cleaned up {} old throughput stats for camera '{}'", deleted, camera_id);
                    }
                    total_deleted += deleted;
                }
                Err(e) => {
                    error!("Failed to cleanup old throughput stats for camera '{}': {}", camera_id, e);
                }
            }
        }
        
        Ok(total_deleted)
    }
}

/// Set the global throughput tracker instance
pub fn set_global_tracker(tracker: Arc<ThroughputTracker>) {
    let _ = GLOBAL_THROUGHPUT_TRACKER.set(tracker);
}

/// Get the global throughput tracker instance
pub fn get_global_tracker() -> Option<Arc<ThroughputTracker>> {
    GLOBAL_THROUGHPUT_TRACKER.get().cloned()
}

/// Helper function to record frame processing from anywhere in the codebase
pub async fn record_frame_globally(camera_id: &str, frame_size: i64) {
    if let Some(tracker) = get_global_tracker() {
        tracker.record_frame(camera_id, frame_size).await;
    }
}

/// Helper function to update FFmpeg FPS from anywhere in the codebase
pub async fn update_ffmpeg_fps_globally(camera_id: &str, fps: f32) {
    if let Some(tracker) = get_global_tracker() {
        tracker.update_ffmpeg_fps(camera_id, fps).await;
    }
}

/// Helper function to update connection count from anywhere in the codebase
pub async fn update_connection_count_globally(camera_id: &str, count: i32) {
    if let Some(tracker) = get_global_tracker() {
        tracker.update_connection_count(camera_id, count).await;
    }
}