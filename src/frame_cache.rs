use std::collections::{HashMap, VecDeque, BTreeMap};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc, Timelike};
use bytes::Bytes;
use tracing::{info, warn, error, debug, trace};
use tokio::process::Command;

use crate::database::{RecordedFrame, VideoSegment};
use crate::errors::Result;

/// Represents a single timestamped frame
#[derive(Debug, Clone)]
pub struct TimestampedFrame {
    pub timestamp: DateTime<Utc>,
    pub frame_data: Bytes,
}

/// Source of cached frames
#[derive(Debug, Clone, PartialEq)]
pub enum CacheSource {
    LiveRecording,   // Frames from active recording buffer
    Mp4Conversion,   // Frames converted from MP4 segments
}

/// Live recording buffer that maintains recent frames in memory
#[derive(Debug)]
pub struct LiveRecordingBuffer {
    frames: VecDeque<TimestampedFrame>,
    max_frames: usize,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
}

impl LiveRecordingBuffer {
    pub fn new(duration_minutes: u32, fps: u32) -> Self {
        let max_frames = (duration_minutes * 60 * fps) as usize;
        Self {
            frames: VecDeque::with_capacity(max_frames),
            max_frames,
            start_time: None,
            end_time: None,
        }
    }

    /// Add a frame to the buffer, removing oldest if at capacity
    pub fn add_frame(&mut self, frame: TimestampedFrame) {
        // Update time bounds
        if self.start_time.is_none() {
            self.start_time = Some(frame.timestamp);
        }
        self.end_time = Some(frame.timestamp);

        // Add frame and maintain capacity
        self.frames.push_back(frame);
        while self.frames.len() > self.max_frames {
            let removed = self.frames.pop_front();
            if let Some(_removed_frame) = removed {
                // Update start time to the new oldest frame
                if let Some(oldest) = self.frames.front() {
                    self.start_time = Some(oldest.timestamp);
                }
            }
        }
    }

    /// Get a frame at or before the specified timestamp
    pub fn get_frame_at(&self, timestamp: DateTime<Utc>) -> Option<TimestampedFrame> {
        // Binary search for the frame closest to but not after the timestamp
        let target_millis = timestamp.timestamp_millis();
        
        // Use binary search since frames are ordered by timestamp
        let result = self.frames.binary_search_by(|frame| {
            frame.timestamp.timestamp_millis().cmp(&target_millis)
        });

        match result {
            Ok(index) => {
                // Exact match
                self.frames.get(index).cloned()
            }
            Err(index) => {
                // No exact match, get the frame just before
                if index > 0 {
                    self.frames.get(index - 1).cloned()
                } else {
                    None
                }
            }
        }
    }

    /// Check if the buffer contains a timestamp
    pub fn contains_timestamp(&self, timestamp: DateTime<Utc>) -> bool {
        if let (Some(start), Some(end)) = (self.start_time, self.end_time) {
            timestamp >= start && timestamp <= end
        } else {
            false
        }
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.frames.clear();
        self.start_time = None;
        self.end_time = None;
    }

    /// Get the number of frames in the buffer
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Get memory usage estimate in bytes
    pub fn memory_usage(&self) -> usize {
        self.frames.iter()
            .map(|f| f.frame_data.len() + std::mem::size_of::<TimestampedFrame>())
            .sum()
    }
}

/// A cache window containing frames for a specific time period
#[derive(Debug)]
pub struct CacheWindow {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub frames: BTreeMap<i64, RecordedFrame>,
    pub last_accessed: Instant,
    pub source: CacheSource,
}

impl CacheWindow {
    pub fn new(start_time: DateTime<Utc>, end_time: DateTime<Utc>, source: CacheSource) -> Self {
        Self {
            start_time,
            end_time,
            frames: BTreeMap::new(),
            last_accessed: Instant::now(),
            source,
        }
    }

    /// Add a frame to the window
    pub fn add_frame(&mut self, timestamp: DateTime<Utc>, frame_data: Vec<u8>) {
        let timestamp_millis = timestamp.timestamp_millis();
        self.frames.insert(timestamp_millis, RecordedFrame {
            timestamp,
            frame_data,
        });
    }

    /// Get a frame at or before the specified timestamp
    pub fn get_frame_at(&mut self, timestamp: DateTime<Utc>) -> Option<RecordedFrame> {
        self.last_accessed = Instant::now();
        
        let target_millis = timestamp.timestamp_millis();
        let one_second_ms = 1000;
        
        // Find the closest frame within 1 second before the timestamp
        self.frames
            .range((target_millis - one_second_ms)..=target_millis)
            .next_back()
            .map(|(_, frame)| frame.clone())
    }

    /// Get memory usage estimate in bytes
    pub fn memory_usage(&self) -> usize {
        self.frames.values()
            .map(|f| f.frame_data.len() + std::mem::size_of::<RecordedFrame>())
            .sum()
    }
}

/// Unified frame cache combining live buffers and MP4 conversion cache
pub struct UnifiedFrameCache {
    /// Live recording buffers for each camera
    live_buffers: Arc<RwLock<HashMap<String, LiveRecordingBuffer>>>,
    
    /// MP4 conversion cache for historical data
    mp4_cache: Arc<RwLock<HashMap<String, HashMap<i64, CacheWindow>>>>,
    
    /// Configuration
    config: CacheConfig,
}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub live_buffer_minutes: u32,
    pub live_buffer_fps: u32,
    pub mp4_window_minutes: u32,
    pub max_windows_per_camera: usize,
    pub mp4_conversion_fps: f32,
    pub ffmpeg_path: String,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            live_buffer_minutes: 5,
            live_buffer_fps: 15,
            mp4_window_minutes: 5,
            max_windows_per_camera: 3,
            mp4_conversion_fps: 15.0,
            ffmpeg_path: "ffmpeg".to_string(),
        }
    }
}

impl UnifiedFrameCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            live_buffers: Arc::new(RwLock::new(HashMap::new())),
            mp4_cache: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Initialize a live buffer for a camera
    pub async fn init_camera(&self, camera_id: &str) {
        let mut buffers = self.live_buffers.write().await;
        buffers.insert(
            camera_id.to_string(),
            LiveRecordingBuffer::new(self.config.live_buffer_minutes, self.config.live_buffer_fps)
        );
        
        let mut cache = self.mp4_cache.write().await;
        cache.insert(camera_id.to_string(), HashMap::new());
        
        info!("Initialized frame cache for camera '{}'", camera_id);
    }

    /// Add a frame to the live recording buffer
    pub async fn add_live_frame(&self, camera_id: &str, timestamp: DateTime<Utc>, frame_data: Bytes) {
        let mut buffers = self.live_buffers.write().await;
        if let Some(buffer) = buffers.get_mut(camera_id) {
            buffer.add_frame(TimestampedFrame { timestamp, frame_data });
        } else {
            warn!("No live buffer found for camera '{}', initializing", camera_id);
            drop(buffers);
            self.init_camera(camera_id).await;
            
            let mut buffers = self.live_buffers.write().await;
            if let Some(buffer) = buffers.get_mut(camera_id) {
                buffer.add_frame(TimestampedFrame { timestamp, frame_data });
            }
        }
    }

    /// Get a frame at a specific timestamp (main entry point for playback)
    pub async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Option<RecordedFrame>> {
        // 1. Check live recording buffer first (fastest)
        if let Some(frame) = self.get_frame_from_live_buffer(camera_id, timestamp).await {
            trace!("Frame found in live buffer for camera '{}' at {}", camera_id, timestamp);
            return Ok(Some(frame));
        }

        // 2. Check MP4 conversion cache
        let window_id = Self::calculate_window_id(timestamp);
        if let Some(frame) = self.get_cached_mp4_frame(camera_id, window_id, timestamp).await {
            trace!("Frame found in MP4 cache for camera '{}' at {}", camera_id, timestamp);
            return Ok(Some(frame));
        }

        // 3. Cache miss - frame will need to be converted from MP4
        debug!("Cache miss for camera '{}' at {}, conversion needed", camera_id, timestamp);
        Ok(None)
    }

    /// Get a frame from the live recording buffer
    async fn get_frame_from_live_buffer(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
    ) -> Option<RecordedFrame> {
        let buffers = self.live_buffers.read().await;
        if let Some(buffer) = buffers.get(camera_id) {
            if buffer.contains_timestamp(timestamp) {
                return buffer.get_frame_at(timestamp).map(|f| RecordedFrame {
                    timestamp: f.timestamp,
                    frame_data: f.frame_data.to_vec(),
                });
            }
        }
        None
    }

    /// Get a frame from the MP4 conversion cache
    async fn get_cached_mp4_frame(
        &self,
        camera_id: &str,
        window_id: i64,
        timestamp: DateTime<Utc>,
    ) -> Option<RecordedFrame> {
        let mut cache = self.mp4_cache.write().await;
        if let Some(camera_cache) = cache.get_mut(camera_id) {
            if let Some(window) = camera_cache.get_mut(&window_id) {
                return window.get_frame_at(timestamp);
            }
        }
        None
    }

    /// Convert and cache a 5-minute window from MP4 segments
    pub async fn convert_and_cache_mp4_window(
        &self,
        camera_id: &str,
        segments: Vec<VideoSegment>,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<()> {
        let window_id = Self::calculate_window_id(window_start);
        
        info!(
            "Converting MP4 segments to frames for camera '{}' window {} ({} - {})",
            camera_id, window_id, window_start, window_end
        );

        debug!("Found {} MP4 segments for conversion:", segments.len());
        for (i, seg) in segments.iter().enumerate() {
            debug!(
                "Segment {}: start={}, end={}, session_id={}, has_file_path={}, has_mp4_data={}, size_bytes={}",
                i + 1,
                seg.start_time,
                seg.end_time,
                seg.session_id,
                seg.file_path.is_some(),
                seg.mp4_data.is_some(),
                seg.size_bytes
            );
            if let Some(ref path) = seg.file_path {
                debug!("  File path: {}", path);
            }
            if seg.mp4_data.is_some() {
                debug!("  MP4 data length: {} bytes", seg.mp4_data.as_ref().unwrap().len());
            }
        }

        // Create a new cache window
        let mut window = CacheWindow::new(window_start, window_end, CacheSource::Mp4Conversion);

        // Process each segment that overlaps with our window
        for segment in segments {
            if let Some(frames) = self.extract_frames_from_segment(
                &segment,
                window_start,
                window_end,
            ).await? {
                for (timestamp, frame_data) in frames {
                    window.add_frame(timestamp, frame_data);
                }
            }
        }

        // Store the window in cache
        let mut cache = self.mp4_cache.write().await;
        let camera_cache = cache.entry(camera_id.to_string()).or_insert_with(HashMap::new);
        
        // Enforce max windows limit
        if camera_cache.len() >= self.config.max_windows_per_camera {
            self.cleanup_oldest_window(camera_cache).await;
        }
        
        let frame_count = window.frames.len();
        camera_cache.insert(window_id, window);
        
        info!(
            "Cached {} frames for camera '{}' window {}",
            frame_count, camera_id, window_id
        );

        Ok(())
    }

    /// Extract frames from an MP4 segment using FFmpeg
    async fn extract_frames_from_segment(
        &self,
        segment: &VideoSegment,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<Option<Vec<(DateTime<Utc>, Vec<u8>)>>> {
        // Determine the source of the MP4 data
        let (mp4_source, cleanup_file) = if let Some(file_path) = &segment.file_path {
            // MP4 is stored on filesystem
            (file_path.clone(), None)
        } else if let Some(mp4_data) = &segment.mp4_data {
            // MP4 is stored in database, write to temp file
            let temp_path = format!("/tmp/segment_{}.mp4", segment.start_time.timestamp());
            tokio::fs::write(&temp_path, mp4_data).await?;
            (temp_path.clone(), Some(temp_path))
        } else {
            warn!("Segment has no MP4 data");
            return Ok(None);
        };

        // Calculate time offsets within the segment
        let _segment_duration = segment.end_time.signed_duration_since(segment.start_time);
        let extract_start = window_start.max(segment.start_time);
        let extract_end = window_end.min(segment.end_time);
        
        if extract_start >= extract_end {
            // No overlap between segment and window
            return Ok(None);
        }

        let start_offset = extract_start
            .signed_duration_since(segment.start_time)
            .num_milliseconds() as f64 / 1000.0;
        let duration = extract_end
            .signed_duration_since(extract_start)
            .num_milliseconds() as f64 / 1000.0;

        debug!(
            "Extracting frames from segment: offset={:.2}s, duration={:.2}s",
            start_offset, duration
        );

        // Run FFmpeg to extract frames
        let mut cmd = Command::new(&self.config.ffmpeg_path);
        cmd.args([
            "-ss", &start_offset.to_string(),
            "-i", &mp4_source,
            "-t", &duration.to_string(),
            "-f", "image2pipe",
            "-vcodec", "mjpeg",
            "-r", &self.config.mp4_conversion_fps.to_string(),
            "-"
        ]);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let output = cmd.output().await?;

        // Clean up temp file if needed
        if let Some(temp_file) = cleanup_file {
            let _ = tokio::fs::remove_file(&temp_file).await;
        }

        if !output.status.success() {
            error!("FFmpeg failed to extract frames from segment");
            return Ok(None);
        }

        // Parse the output frames
        let frames = self.parse_mjpeg_stream(
            &output.stdout,
            extract_start,
            self.config.mp4_conversion_fps,
        )?;

        Ok(Some(frames))
    }

    /// Parse MJPEG stream from FFmpeg output
    fn parse_mjpeg_stream(
        &self,
        data: &[u8],
        start_time: DateTime<Utc>,
        fps: f32,
    ) -> Result<Vec<(DateTime<Utc>, Vec<u8>)>> {
        let mut frames = Vec::new();
        let mut cursor = 0;
        let frame_duration_ms = (1000.0 / fps) as i64;
        let mut frame_index = 0;

        // JPEG markers
        const JPEG_START: [u8; 2] = [0xFF, 0xD8];
        const JPEG_END: [u8; 2] = [0xFF, 0xD9];

        while cursor < data.len() - 1 {
            // Find JPEG start marker
            if let Some(start_pos) = Self::find_marker(&data[cursor..], &JPEG_START) {
                let absolute_start = cursor + start_pos;
                
                // Find JPEG end marker
                if let Some(end_pos) = Self::find_marker(&data[absolute_start + 2..], &JPEG_END) {
                    let absolute_end = absolute_start + 2 + end_pos + 2;
                    
                    // Extract frame data
                    let frame_data = data[absolute_start..absolute_end].to_vec();
                    
                    // Calculate timestamp for this frame
                    let frame_timestamp = start_time + chrono::Duration::milliseconds(
                        frame_index * frame_duration_ms
                    );
                    
                    frames.push((frame_timestamp, frame_data));
                    frame_index += 1;
                    
                    cursor = absolute_end;
                } else {
                    break; // No complete frame found
                }
            } else {
                break; // No more frames
            }
        }

        debug!("Parsed {} frames from MJPEG stream", frames.len());
        Ok(frames)
    }

    /// Find a marker in byte array
    fn find_marker(data: &[u8], marker: &[u8; 2]) -> Option<usize> {
        data.windows(2)
            .position(|window| window[0] == marker[0] && window[1] == marker[1])
    }

    /// Clean up the oldest window from a camera's cache
    async fn cleanup_oldest_window(&self, camera_cache: &mut HashMap<i64, CacheWindow>) {
        if let Some((&oldest_id, _)) = camera_cache
            .iter()
            .min_by_key(|(_, window)| window.last_accessed)
        {
            camera_cache.remove(&oldest_id);
            debug!("Removed oldest cache window {}", oldest_id);
        }
    }

    /// Calculate the window ID for a given timestamp
    pub fn calculate_window_id(timestamp: DateTime<Utc>) -> i64 {
        // Round to 5-minute boundaries
        let minutes = timestamp.minute();
        let rounded_minutes = (minutes / 5) * 5;
        
        timestamp
            .with_minute(rounded_minutes).unwrap()
            .with_second(0).unwrap()
            .with_nanosecond(0).unwrap()
            .timestamp()
    }

    /// Calculate the window time range for a timestamp
    pub fn calculate_window_range(timestamp: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
        // Center the window around the timestamp (±2.5 minutes)
        let window_start = timestamp - chrono::Duration::minutes(2) - chrono::Duration::seconds(30);
        let window_end = timestamp + chrono::Duration::minutes(2) + chrono::Duration::seconds(30);
        (window_start, window_end)
    }

    /// Clear the live buffer for a camera (e.g., when recording stops)
    pub async fn clear_live_buffer(&self, camera_id: &str) {
        let mut buffers = self.live_buffers.write().await;
        if let Some(buffer) = buffers.get_mut(camera_id) {
            buffer.clear();
            info!("Cleared live buffer for camera '{}'", camera_id);
        }
    }

    /// Get memory usage statistics
    pub async fn get_memory_stats(&self, camera_id: &str) -> (usize, usize) {
        let mut live_memory = 0;
        let mut cache_memory = 0;

        // Live buffer memory
        let buffers = self.live_buffers.read().await;
        if let Some(buffer) = buffers.get(camera_id) {
            live_memory = buffer.memory_usage();
        }

        // MP4 cache memory
        let cache = self.mp4_cache.read().await;
        if let Some(camera_cache) = cache.get(camera_id) {
            cache_memory = camera_cache.values()
                .map(|w| w.memory_usage())
                .sum();
        }

        (live_memory, cache_memory)
    }

    /// Check if a timestamp is available in cache (without loading)
    pub async fn is_timestamp_cached(&self, camera_id: &str, timestamp: DateTime<Utc>) -> bool {
        // Check live buffer
        let buffers = self.live_buffers.read().await;
        if let Some(buffer) = buffers.get(camera_id) {
            if buffer.contains_timestamp(timestamp) {
                return true;
            }
        }
        drop(buffers);

        // Check MP4 cache
        let window_id = Self::calculate_window_id(timestamp);
        let cache = self.mp4_cache.read().await;
        if let Some(camera_cache) = cache.get(camera_id) {
            if camera_cache.contains_key(&window_id) {
                return true;
            }
        }

        false
    }
}