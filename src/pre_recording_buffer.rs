use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc, Duration};
use bytes::Bytes;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct BufferedFrame {
    pub timestamp: DateTime<Utc>,
    pub data: Bytes,
}

#[derive(Clone)]
pub struct PreRecordingBuffer {
    buffer: Arc<RwLock<VecDeque<BufferedFrame>>>,
    buffer_duration_minutes: u64,
    cleanup_interval_seconds: u64,
}

impl PreRecordingBuffer {
    pub fn new(buffer_duration_minutes: u64, cleanup_interval_seconds: u64) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(VecDeque::new())),
            buffer_duration_minutes,
            cleanup_interval_seconds,
        }
    }

    /// Add a frame to the pre-recording buffer
    pub async fn add_frame(&self, frame_data: Bytes) {
        let frame = BufferedFrame {
            timestamp: Utc::now(),
            data: frame_data,
        };

        let mut buffer = self.buffer.write().await;
        buffer.push_back(frame);
    }

    /// Get all buffered frames and return them in chronological order
    /// This is called when recording starts to include pre-recorded content
    pub async fn get_buffered_frames(&self) -> Vec<BufferedFrame> {
        let buffer = self.buffer.read().await;
        buffer.iter().cloned().collect()
    }

    /// Get the timestamp of the first (oldest) frame in the buffer
    /// This will be used as the recording start time
    pub async fn get_first_frame_timestamp(&self) -> Option<DateTime<Utc>> {
        let buffer = self.buffer.read().await;
        buffer.front().map(|frame| frame.timestamp)
    }

    /// Clean up old frames that are older than the buffer duration
    pub async fn cleanup_old_frames(&self) {
        let cutoff_time = Utc::now() - Duration::minutes(self.buffer_duration_minutes as i64);
        let mut buffer = self.buffer.write().await;
        
        let _initial_count = buffer.len();
        
        // Remove frames older than the cutoff time
        while let Some(frame) = buffer.front() {
            if frame.timestamp < cutoff_time {
                buffer.pop_front();
            } else {
                break;
            }
        }
        
        /*
        let removed_count = initial_count - buffer.len();
        if removed_count > 0 {
            debug!("Cleaned up {} old frames from pre-recording buffer, {} frames remaining", 
                   removed_count, buffer.len());
        }
        */
    }

    /// Start the cleanup task that runs periodically to remove old frames
    pub async fn start_cleanup_task(&self, _camera_id: String) -> tokio::task::JoinHandle<()> {
        let buffer_clone = self.clone();
        let interval_seconds = self.cleanup_interval_seconds;
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_seconds));
            debug!("Pre-recording buffer cleanup task started with {} second interval", interval_seconds);
            loop {
                interval.tick().await;
                buffer_clone.cleanup_old_frames().await;
            }
        })
    }

    /// Get current buffer statistics
    pub async fn get_stats(&self) -> BufferStats {
        let buffer = self.buffer.read().await;
        let frame_count = buffer.len();
        let oldest_timestamp = buffer.front().map(|f| f.timestamp);
        let newest_timestamp = buffer.back().map(|f| f.timestamp);
        
        let total_size_bytes = buffer.iter().map(|f| f.data.len()).sum::<usize>();
        
        BufferStats {
            frame_count,
            oldest_timestamp,
            newest_timestamp,
            total_size_bytes,
        }
    }
}

#[derive(Debug)]
pub struct BufferStats {
    pub frame_count: usize,
    pub oldest_timestamp: Option<DateTime<Utc>>,
    pub newest_timestamp: Option<DateTime<Utc>>,
    pub total_size_bytes: usize,
}