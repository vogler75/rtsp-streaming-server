use std::sync::Arc;
use async_trait::async_trait;
use chrono::{DateTime, Utc, Duration};
use tracing::{info, debug, warn, error};

use crate::database::{FrameStream, RecordedFrame, DatabaseProvider};
use crate::frame_cache::UnifiedFrameCache;
use crate::errors::Result;

/// A frame stream that uses the unified cache instead of database queries
pub struct CachedFrameStream {
    camera_id: String,
    current_timestamp: DateTime<Utc>,
    end_timestamp: DateTime<Utc>,
    cache: Arc<UnifiedFrameCache>,
    database: Arc<dyn DatabaseProvider>,
    current_window_id: i64,
    next_window_preloading: bool,
    frame_interval_ms: i64,
    finished: bool,
}

impl CachedFrameStream {
    pub async fn new(
        camera_id: String,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        cache: Arc<UnifiedFrameCache>,
        database: Arc<dyn DatabaseProvider>,
        fps: f32,
    ) -> Result<Self> {
        let frame_interval_ms = (1000.0 / fps) as i64;
        let current_window_id = UnifiedFrameCache::calculate_window_id(from);
        
        let stream = Self {
            camera_id,
            current_timestamp: from,
            end_timestamp: to,
            cache,
            database,
            current_window_id,
            next_window_preloading: false,
            frame_interval_ms,
            finished: false,
        };

        // Ensure initial window is available
        stream.ensure_cache_window_available(from).await?;
        
        info!(
            "Created CachedFrameStream for camera '{}' from {} to {}",
            stream.camera_id, from, to
        );

        Ok(stream)
    }

    /// Ensure a cache window is available for the given timestamp
    async fn ensure_cache_window_available(&self, timestamp: DateTime<Utc>) -> Result<()> {
        // Check if already cached
        if self.cache.is_timestamp_cached(&self.camera_id, timestamp).await {
            debug!("Window for timestamp {} already cached", timestamp);
            return Ok(());
        }

        info!("Loading cache window for timestamp {}", timestamp);
        
        // Calculate window range
        let (window_start, window_end) = UnifiedFrameCache::calculate_window_range(timestamp);
        
        // Find MP4 segments that overlap with this window
        let segments = self.database.list_video_segments(
            &self.camera_id,
            window_start,
            window_end,
        ).await?;

        if segments.is_empty() {
            warn!(
                "No MP4 segments found for camera '{}' in range {} - {}",
                self.camera_id, window_start, window_end
            );
            return Ok(());
        }

        // Convert and cache the segments
        self.cache.convert_and_cache_mp4_window(
            &self.camera_id,
            segments,
            window_start,
            window_end,
        ).await?;

        Ok(())
    }

    /// Preload the next window in background
    async fn preload_next_window(&mut self) {
        if self.next_window_preloading {
            return; // Already preloading
        }

        self.next_window_preloading = true;
        
        // Calculate next window boundaries
        let next_window_start = self.current_timestamp + Duration::minutes(2) + Duration::seconds(30);
        let next_window_id = UnifiedFrameCache::calculate_window_id(next_window_start);
        
        // Skip if we've already loaded this window
        if next_window_id == self.current_window_id {
            return;
        }

        let cache = self.cache.clone();
        let database = self.database.clone();
        let camera_id = self.camera_id.clone();
        
        // Spawn background task for preloading
        tokio::spawn(async move {
            debug!("Preloading next window for camera '{}'", camera_id);
            
            let (window_start, window_end) = UnifiedFrameCache::calculate_window_range(next_window_start);
            
            // Check if already cached
            if cache.is_timestamp_cached(&camera_id, next_window_start).await {
                debug!("Next window already cached");
                return;
            }
            
            // Find and convert MP4 segments
            match database.list_video_segments(&camera_id, window_start, window_end).await {
                Ok(segments) => {
                    if !segments.is_empty() {
                        if let Err(e) = cache.convert_and_cache_mp4_window(
                            &camera_id,
                            segments,
                            window_start,
                            window_end,
                        ).await {
                            error!("Failed to preload next cache window: {}", e);
                        } else {
                            info!("Preloaded next window for camera '{}'", camera_id);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to list segments for preloading: {}", e);
                }
            }
        });
    }

    /// Check if we're approaching a window boundary and need to preload
    fn check_preload_needed(&self) -> bool {
        let current_window_id = UnifiedFrameCache::calculate_window_id(self.current_timestamp);
        let next_check_time = self.current_timestamp + Duration::seconds(30);
        let next_window_id = UnifiedFrameCache::calculate_window_id(next_check_time);
        
        current_window_id != next_window_id
    }

    /// Advance to the next frame timestamp
    fn advance_timestamp(&mut self) {
        self.current_timestamp = self.current_timestamp + Duration::milliseconds(self.frame_interval_ms);
        
        // Update current window ID
        let new_window_id = UnifiedFrameCache::calculate_window_id(self.current_timestamp);
        if new_window_id != self.current_window_id {
            debug!("Moved to new window {}", new_window_id);
            self.current_window_id = new_window_id;
            self.next_window_preloading = false; // Reset preloading flag
        }
    }
}

#[async_trait]
impl FrameStream for CachedFrameStream {
    async fn next_frame(&mut self) -> Result<Option<RecordedFrame>> {
        if self.finished || self.current_timestamp >= self.end_timestamp {
            self.finished = true;
            return Ok(None);
        }

        // Get frame from cache (should be instant for cached data)
        let frame_result = self.cache.get_frame_at_timestamp(
            &self.camera_id,
            self.current_timestamp,
        ).await?;

        if let Some(frame) = frame_result {
            // Check if we need to preload the next window
            if self.check_preload_needed() && !self.next_window_preloading {
                self.preload_next_window().await;
            }

            // Advance to next frame timestamp
            self.advance_timestamp();
            
            Ok(Some(frame))
        } else {
            // Frame not in cache, try to load the window
            debug!(
                "Frame not cached for timestamp {}, attempting to load window",
                self.current_timestamp
            );
            
            // Try to ensure the window is loaded
            if let Err(e) = self.ensure_cache_window_available(self.current_timestamp).await {
                error!("Failed to load cache window: {}", e);
                // Skip this frame and try the next one
                self.advance_timestamp();
                return self.next_frame().await;
            }
            
            // Retry getting the frame
            let frame = self.cache.get_frame_at_timestamp(
                &self.camera_id,
                self.current_timestamp,
            ).await?;
            
            // Advance timestamp regardless
            self.advance_timestamp();
            
            Ok(frame)
        }
    }

    async fn close(&mut self) -> Result<()> {
        self.finished = true;
        debug!("Closed CachedFrameStream for camera '{}'", self.camera_id);
        Ok(())
    }

    fn estimated_frame_count(&self) -> Option<usize> {
        // Calculate estimated frame count based on time range and FPS
        let duration_ms = self.end_timestamp
            .signed_duration_since(self.current_timestamp)
            .num_milliseconds();
        
        if duration_ms > 0 && self.frame_interval_ms > 0 {
            Some((duration_ms / self.frame_interval_ms) as usize)
        } else {
            None
        }
    }
}

/// Factory function to create the appropriate frame stream based on cache availability
pub async fn create_frame_stream(
    camera_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    cache: Option<Arc<UnifiedFrameCache>>,
    database: Arc<dyn DatabaseProvider>,
    fps: f32,
) -> Result<Box<dyn FrameStream>> {
    if let Some(cache) = cache {
        // Use cached frame stream (no database access during playback)
        let stream = CachedFrameStream::new(
            camera_id.to_string(),
            from,
            to,
            cache,
            database,
            fps,
        ).await?;
        Ok(Box::new(stream))
    } else {
        // Fallback to database stream (old behavior)
        warn!("No cache provided, falling back to database stream for camera '{}'", camera_id);
        database.create_frame_stream(camera_id, from, to).await
    }
}