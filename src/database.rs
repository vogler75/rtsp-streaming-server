use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{SqlitePool, PgPool, Row, FromRow};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous, SqlitePoolOptions};
use tracing::{error, info, debug};
use std::sync::Arc;
use std::str::FromStr;
use crate::errors::{Result, StreamError};

// Table name constants for easy configuration
const TABLE_RECORDING_SESSIONS: &str = "recording_sessions";
const TABLE_RECORDING_MJPEG: &str = "recording_mjpeg";  // formerly recorded_frames
const TABLE_RECORDING_MP4: &str = "recording_mp4";      // formerly video_segments
const TABLE_HLS_PLAYLISTS: &str = "hls_playlists";
const TABLE_HLS_SEGMENTS: &str = "hls_segments";
const TABLE_RECORDING_HLS: &str = "recording_hls";
const TABLE_THROUGHPUT_STATS: &str = "throughput_stats";

#[derive(Debug, Clone)]
pub struct RecordingSession {
    pub session_id: i64,  // Primary key
    pub camera_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub status: RecordingStatus,
    pub keep_session: bool,
}

#[derive(Debug, Clone)]
pub struct RecordedFrame {
    pub timestamp: DateTime<Utc>,
    pub frame_data: Vec<u8>,  // Store actual frame data
}

#[derive(Debug, Clone, FromRow)]
pub struct VideoSegment {
    pub camera_id: String,    // Part of composite primary key (camera_id, start_time)
    pub session_id: i64,      // Foreign key to recording_sessions
    pub start_time: DateTime<Utc>,  // Part of composite primary key
    pub end_time: DateTime<Utc>,
    pub file_path: Option<String>,  // Optional for database storage
    pub size_bytes: i64,
    pub mp4_data: Option<Vec<u8>>,  // Optional blob data for database storage
    #[sqlx(default)]
    pub recording_reason: Option<String>,  // Recording reason from recording_sessions (JOIN)
}

#[derive(Debug, Clone, FromRow)]
pub struct HlsPlaylist {
    pub playlist_id: String,  // Unique identifier for the playlist
    pub camera_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub segment_duration: i32,  // Segment duration in seconds
    pub playlist_content: String,  // M3U8 playlist content
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,  // When this playlist expires
}

#[derive(Debug, Clone, FromRow)]
pub struct HlsSegment {
    pub playlist_id: String,  // References HlsPlaylist
    pub segment_name: String,  // e.g., "segment_000.ts"
    pub segment_index: i32,    // Segment number in playlist
    pub segment_data: Vec<u8>, // MPEG-TS segment data
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct RecordingHlsSegment {
    pub camera_id: String,    // Part of composite primary key (camera_id, segment_index)
    pub session_id: i64,      // Foreign key to recording_sessions
    pub segment_index: i32,   // Part of composite primary key
    pub start_time: DateTime<Utc>, // Start timestamp of this segment
    pub end_time: DateTime<Utc>,   // End timestamp of this segment
    pub duration_seconds: f64,     // Actual segment duration in seconds
    pub segment_data: Vec<u8>,     // MPEG-TS segment data
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ThroughputStats {
    pub camera_id: String,
    pub timestamp: DateTime<Utc>,
    pub bytes_per_second: i64,  // Amount of data streamed in this second
    pub frame_count: i32,       // Number of frames processed in this second
    pub ffmpeg_fps: f32,        // FFmpeg reported FPS
    pub connection_count: i32,  // Number of active WebSocket connections
}


// Streaming interface for database-agnostic frame iteration
#[async_trait]
pub trait FrameStream: Send {
    /// Get the next frame from the stream
    async fn next_frame(&mut self) -> Result<Option<RecordedFrame>>;
    
    /// Close the stream and cleanup resources
    async fn close(&mut self) -> Result<()>;
    
    /// Get an estimated frame count (optional, may not be accurate)
    fn estimated_frame_count(&self) -> Option<usize> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecordingStatus {
    Active,
    Stopped,
    Completed,
}

impl From<String> for RecordingStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "active" => RecordingStatus::Active,
            "stopped" => RecordingStatus::Stopped,
            "completed" => RecordingStatus::Completed,
            _ => RecordingStatus::Stopped,
        }
    }
}

impl From<RecordingStatus> for String {
    fn from(status: RecordingStatus) -> Self {
        match status {
            RecordingStatus::Active => "active".to_string(),
            RecordingStatus::Stopped => "stopped".to_string(),
            RecordingStatus::Completed => "completed".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecordingQuery {
    pub camera_id: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct DeletedRecordingStats {
    pub session_id: i64,
    pub frames_deleted: u64,
    pub mp4_segments_deleted: u64,
    pub hls_segments_deleted: u64,
}

#[derive(Debug, Clone)]
pub struct BulkDeleteResult {
    pub deleted_count: usize,
    pub failed: Vec<String>, // filenames that failed to delete
    pub total_size_bytes: i64,
}

#[async_trait]
pub trait DatabaseProvider: Send + Sync {
    async fn initialize(&self) -> Result<()>;
    
    async fn create_recording_session(
        &self,
        camera_id: &str,
        reason: Option<&str>,
        start_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64>;
    
    async fn stop_recording_session(&self, session_id: i64) -> Result<()>;
    
    async fn get_active_recordings(&self, camera_id: &str) -> Result<Vec<RecordingSession>>;
    
    async fn add_recorded_frame(
        &self,
        session_id: i64,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        frame_number: i64,
        frame_data: &[u8],
    ) -> Result<i64>;

    /// Bulk insert multiple recorded frames for better performance
    async fn add_recorded_frames_bulk(
        &self,
        session_id: i64,
        camera_id: &str,
        frames: &[(DateTime<Utc>, i64, Vec<u8>)], // (timestamp, frame_number, frame_data)
    ) -> Result<u64>;
    
    async fn list_recordings(&self, query: &RecordingQuery) -> Result<Vec<RecordingSession>>;
    async fn list_recordings_filtered(&self, camera_id: &str, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>, reason: Option<&str>) -> Result<Vec<RecordingSession>>;
    
    async fn get_recorded_frames(
        &self,
        session_id: i64,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordedFrame>>;
    
    async fn delete_old_frames(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize>;
    
    async fn delete_unused_sessions(
        &self,
        camera_id: Option<&str>,
    ) -> Result<usize>;
    
    async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        tolerance_seconds: Option<i64>,
    ) -> Result<Option<RecordedFrame>>;
    
    /// Create a streaming cursor for frames in the given time range
    async fn create_frame_stream(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Box<dyn FrameStream>>;
    
    async fn get_database_size(&self) -> Result<i64>;

    async fn add_video_segment(&self, segment: &VideoSegment) -> Result<i64>;

    async fn list_video_segments(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<VideoSegment>>;

    async fn list_video_segments_filtered(
        &self,
        camera_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        reason: Option<&str>,
        limit: i64,
        sort_order: &str,
    ) -> Result<Vec<VideoSegment>>;

    async fn delete_old_video_segments(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize>;

    async fn cleanup_database(
        &self,
        config: &crate::config::RecordingConfig,
        camera_configs: &std::collections::HashMap<String, crate::config::CameraConfig>,
    ) -> Result<()>;
    
    
    /// Get a specific video segment by timestamp (efficient query)
    async fn get_video_segment_by_time(
        &self,
        camera_id: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<VideoSegment>>;
        
    // HLS-specific methods
    async fn store_hls_playlist(&self, playlist: &HlsPlaylist) -> Result<()>;
    async fn store_hls_segment(&self, segment: &HlsSegment) -> Result<()>;
    async fn store_hls_playlist_with_segments(&self, playlist: &HlsPlaylist, segments: &[HlsSegment]) -> Result<()>;
    async fn get_hls_playlist(&self, playlist_id: &str) -> Result<Option<HlsPlaylist>>;
    async fn get_hls_segment(&self, playlist_id: &str, segment_name: &str) -> Result<Option<HlsSegment>>;
    async fn cleanup_expired_hls(&self) -> Result<usize>;
    
    // Recording HLS methods
    async fn add_recording_hls_segment(&self, segment: &RecordingHlsSegment) -> Result<i64>;
    async fn list_recording_hls_segments(
        &self,
        session_id: i64,
        from_time: Option<DateTime<Utc>>,
        to_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordingHlsSegment>>;
    async fn get_recording_hls_segments_for_timerange(
        &self,
        camera_id: &str,
        from_time: DateTime<Utc>,
        to_time: DateTime<Utc>,
    ) -> Result<Vec<RecordingHlsSegment>>;
    async fn delete_old_recording_hls_segments(
        &self,
        retention_duration: &str,
        camera_id: Option<&str>,
    ) -> Result<usize>;
    async fn get_recording_hls_segment_by_session_and_index(
        &self,
        session_id: i64,
        segment_index: i32,
    ) -> Result<Option<RecordingHlsSegment>>;
    async fn get_last_hls_segment_index_for_session(
        &self,
        session_id: i64,
    ) -> Result<Option<i32>>;

    async fn set_session_keep_flag(
        &self,
        session_id: i64,
        keep_session: bool,
    ) -> Result<()>;

    // Delete functions for manual recording management
    async fn delete_recording_session(&self, session_id: i64) -> Result<DeletedRecordingStats>;
    async fn delete_mp4_segment_by_filename(&self, camera_id: &str, filename: &str) -> Result<i64>;
    async fn delete_mp4_segments_bulk(&self, camera_id: &str, filenames: Vec<String>) -> Result<BulkDeleteResult>;
    async fn delete_hls_segments_by_session(&self, session_id: i64) -> Result<u64>;
    async fn delete_hls_segments_by_timerange(&self, camera_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<u64>;

    /// Vacuum/compact tables after cleanup to reclaim disk space
    /// For PostgreSQL: VACUUM FULL on recording tables
    /// For SQLite: VACUUM (entire database)
    async fn vacuum_tables(&self) -> Result<()>;

    // Export methods
    async fn get_mp4_segments_in_range(&self, camera_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<crate::export_jobs::Mp4SegmentInfo>>;
    async fn extract_mp4_segment_to_file(&self, segment_id: i64, output_path: &str) -> Result<()>;

    // Throughput tracking methods
    async fn record_throughput_stats(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        bytes_per_second: i64,
        frame_count: i32,
        ffmpeg_fps: f32,
        connection_count: i32,
    ) -> Result<()>;
    
    async fn get_throughput_stats(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<ThroughputStats>>;
    
    async fn cleanup_old_throughput_stats(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64>;
}

pub struct SqliteDatabase {
    pool: SqlitePool,
    /// RwLock to coordinate between recording (read) and cleanup (write) operations.
    /// Recording operations acquire read lock (can run concurrently).
    /// Cleanup operations acquire write lock (exclusive access).
    cleanup_lock: tokio::sync::RwLock<()>,
}

// SQLite-specific frame streaming implementation
pub struct SqliteFrameStream {
    connection: sqlx::pool::PoolConnection<sqlx::Sqlite>,
    camera_id: String,
    to: DateTime<Utc>,
    current_timestamp: Option<DateTime<Utc>>,
    batch_size: i64,
    current_batch: Vec<RecordedFrame>,
    batch_index: usize,
    finished: bool,
}

impl SqliteFrameStream {
    async fn new(
        pool: &SqlitePool,
        camera_id: String,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Self> {
        let connection = pool.acquire().await?;
        Ok(Self {
            connection,
            camera_id,
            to,
            current_timestamp: Some(from),
            batch_size: 50, // Process 50 frames at a time for memory efficiency
            current_batch: Vec::with_capacity(50), // Pre-allocate for efficiency
            batch_index: 0,
            finished: false,
        })
    }
    
    async fn fetch_next_batch(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        
        let current_ts = match self.current_timestamp {
            Some(ts) => ts,
            None => {
                self.finished = true;
                return Ok(());
            }
        };
        
        // Use camera_id directly from recording_mjpeg table with idx_camera_timestamp index
        let query = format!(
            r#"
            SELECT timestamp, frame_data
            FROM {}
            WHERE camera_id = ?
              AND timestamp >= ?
              AND timestamp <= ?
            ORDER BY timestamp ASC
            LIMIT ?
            "#,
            TABLE_RECORDING_MJPEG
        );
        let rows = sqlx::query(&query)
        .bind(&self.camera_id)
        .bind(current_ts)
        .bind(self.to)
        .bind(self.batch_size)
        .fetch_all(self.connection.as_mut())
        .await?;
        
        self.current_batch.clear();
        self.batch_index = 0;
        
        for row in rows {
            let timestamp: DateTime<Utc> = row.get("timestamp");
            let frame_data: Vec<u8> = row.get("frame_data");
            
            self.current_batch.push(RecordedFrame {
                timestamp,
                frame_data,
            });
            
            // Update current timestamp for next batch
            self.current_timestamp = Some(timestamp + chrono::Duration::microseconds(1));
        }
        
        // If we got fewer rows than requested, we've reached the end
        if self.current_batch.len() < self.batch_size as usize {
            self.finished = true;
        }
        
        Ok(())
    }
}

#[async_trait]
impl FrameStream for SqliteFrameStream {
    async fn next_frame(&mut self) -> Result<Option<RecordedFrame>> {
        // If we've consumed all frames in current batch, fetch the next batch
        if self.batch_index >= self.current_batch.len() {
            self.fetch_next_batch().await?;
            
            // If still no frames after fetching, we're done
            if self.current_batch.is_empty() {
                return Ok(None);
            }
        }
        
        // Double-check that batch_index is within bounds after fetch
        if self.batch_index >= self.current_batch.len() {
            // This shouldn't happen, but protect against it
            error!("Unexpected state: batch_index {} >= batch length {} after fetch", 
                   self.batch_index, self.current_batch.len());
            return Ok(None);
        }
        
        // Return the next frame from current batch - use safe indexing
        let frame = match self.current_batch.get(self.batch_index) {
            Some(frame) => frame.clone(),
            None => {
                error!("Failed to get frame at index {} from batch of length {}", 
                       self.batch_index, self.current_batch.len());
                return Ok(None);
            }
        };
        
        self.batch_index += 1;
        Ok(Some(frame))
    }
    
    async fn close(&mut self) -> Result<()> {
        // SQLite connection will be dropped automatically
        self.finished = true;
        self.current_batch.clear();
        self.current_batch.shrink_to_fit(); // Release memory
        self.current_timestamp = None;
        Ok(())
    }
    
    fn estimated_frame_count(&self) -> Option<usize> {
        // Could implement a count query here if needed
        None
    }
}

impl SqliteDatabase {
    pub async fn new(database_path: &str) -> Result<Self> {
        // Ensure the directory exists
        if let Some(parent) = std::path::Path::new(database_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Configure SQLite connection options for better concurrency:
        // - WAL mode: allows concurrent reads during writes
        // - busy_timeout: wait up to 60 seconds for locks instead of failing immediately
        // - synchronous=NORMAL: good balance of safety and performance with WAL
        // These options are applied to EVERY connection in the pool
        let database_url = format!("sqlite://{}?mode=rwc", database_path);
        let connect_options = SqliteConnectOptions::from_str(&database_url)?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(60));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options)
            .await?;

        info!("SQLite database configured with WAL mode and 60s busy timeout: {}", database_path);

        Ok(Self {
            pool,
            cleanup_lock: tokio::sync::RwLock::new(()),
        })
    }
}

#[async_trait]
impl DatabaseProvider for SqliteDatabase {
    async fn initialize(&self) -> Result<()> {
        let init_start = std::time::Instant::now();
        info!("Starting SQLite database initialization...");

        let create_sessions_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                session_id INTEGER PRIMARY KEY AUTOINCREMENT,
                camera_id TEXT NOT NULL,
                start_time TIMESTAMP NOT NULL,
                end_time TIMESTAMP,
                reason TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                keep_session BOOLEAN NOT NULL DEFAULT 0
            )
            "#,
            TABLE_RECORDING_SESSIONS
        );
        info!("Executing CREATE TABLE recording_sessions...");
        let query_start = std::time::Instant::now();
        sqlx::query(&create_sessions_query)
            .execute(&self.pool)
            .await?;
        info!("CREATE recording_sessions took {:?}", query_start.elapsed());

        let create_mjpeg_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                session_id INTEGER NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                frame_data BLOB NOT NULL,
                PRIMARY KEY (camera_id, timestamp),
                FOREIGN KEY (session_id) REFERENCES {}(session_id)
            )
            "#,
            TABLE_RECORDING_MJPEG, TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_mjpeg_query)
            .execute(&self.pool)
            .await?;
        info!("CREATE recording_mjpeg done, elapsed {:?}", init_start.elapsed());

        let idx_camera_timestamp = format!(
            "CREATE INDEX IF NOT EXISTS idx_camera_timestamp ON {}(camera_id, timestamp)",
            TABLE_RECORDING_MJPEG
        );
        sqlx::query(&idx_camera_timestamp)
            .execute(&self.pool)
            .await?;

        // Add index on session_id for FK lookups
        let idx_mjpeg_session = format!(
            "CREATE INDEX IF NOT EXISTS idx_recording_mjpeg_session ON {}(session_id)",
            TABLE_RECORDING_MJPEG
        );
        sqlx::query(&idx_mjpeg_session)
            .execute(&self.pool)
            .await?;
        info!("CREATE idx_camera_timestamp done, elapsed {:?}", init_start.elapsed());

        let create_mp4_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                session_id INTEGER NOT NULL,
                start_time TIMESTAMP NOT NULL,
                end_time TIMESTAMP NOT NULL,
                file_path TEXT,
                size_bytes INTEGER NOT NULL,
                mp4_data BLOB,
                PRIMARY KEY (camera_id, start_time),
                FOREIGN KEY (session_id) REFERENCES {}(session_id) ON DELETE CASCADE
            )
            "#,
            TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_mp4_query)
            .execute(&self.pool)
            .await?;

        let idx_segment_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_segment_time ON {}(start_time, end_time)",
            TABLE_RECORDING_MP4
        );
        sqlx::query(&idx_segment_time)
            .execute(&self.pool)
            .await?;

        // Add index on session_id for the JOIN operation
        let idx_segment_session = format!(
            "CREATE INDEX IF NOT EXISTS idx_segment_session ON {}(session_id)",
            TABLE_RECORDING_MP4
        );
        sqlx::query(&idx_segment_session)
            .execute(&self.pool)
            .await?;
        info!("MP4 table and indexes done, elapsed {:?}", init_start.elapsed());

        // Add indexes on recording_sessions for common query patterns
        let idx_camera_start_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_camera_start_time ON {}(camera_id, start_time)",
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&idx_camera_start_time)
            .execute(&self.pool)
            .await?;
        info!("Sessions index done, elapsed {:?}", init_start.elapsed());

        // Create HLS playlists table
        let create_hls_playlists_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                playlist_id TEXT PRIMARY KEY,
                camera_id TEXT NOT NULL,
                start_time TIMESTAMP NOT NULL,
                end_time TIMESTAMP NOT NULL,
                segment_duration INTEGER NOT NULL,
                playlist_content TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                expires_at TIMESTAMP NOT NULL
            )
            "#,
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&create_hls_playlists_query)
            .execute(&self.pool)
            .await?;

        // Create HLS segments table
        let create_hls_segments_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                playlist_id TEXT NOT NULL,
                segment_name TEXT NOT NULL,
                segment_index INTEGER NOT NULL,
                segment_data BLOB NOT NULL,
                size_bytes INTEGER NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (playlist_id, segment_name),
                FOREIGN KEY (playlist_id) REFERENCES {}(playlist_id) ON DELETE CASCADE
            )
            "#,
            TABLE_HLS_SEGMENTS, TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&create_hls_segments_query)
            .execute(&self.pool)
            .await?;

        // Create recording HLS segments table
        let create_recording_hls_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                session_id INTEGER NOT NULL,
                segment_index INTEGER NOT NULL,
                start_time TIMESTAMP NOT NULL,
                end_time TIMESTAMP NOT NULL,
                duration_seconds REAL NOT NULL,
                segment_data BLOB NOT NULL,
                size_bytes INTEGER NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (camera_id, segment_index),
                FOREIGN KEY (session_id) REFERENCES {}(session_id) ON DELETE CASCADE
            )
            "#,
            TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_recording_hls_query)
            .execute(&self.pool)
            .await?;
        info!("HLS tables done, elapsed {:?}", init_start.elapsed());

        // Add indexes for HLS tables
        let idx_hls_playlists_camera = format!(
            "CREATE INDEX IF NOT EXISTS idx_hls_playlists_camera ON {}(camera_id, start_time, end_time)",
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&idx_hls_playlists_camera)
            .execute(&self.pool)
            .await?;

        let idx_hls_playlists_expires = format!(
            "CREATE INDEX IF NOT EXISTS idx_hls_playlists_expires ON {}(expires_at)",
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&idx_hls_playlists_expires)
            .execute(&self.pool)
            .await?;

        let idx_hls_segments_playlist = format!(
            "CREATE INDEX IF NOT EXISTS idx_hls_segments_playlist ON {}(playlist_id, segment_index)",
            TABLE_HLS_SEGMENTS
        );
        sqlx::query(&idx_hls_segments_playlist)
            .execute(&self.pool)
            .await?;

        let idx_recording_hls_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_recording_hls_time ON {}(start_time, end_time)",
            TABLE_RECORDING_HLS
        );
        sqlx::query(&idx_recording_hls_time)
            .execute(&self.pool)
            .await?;

        // Add index on session_id for FK lookups
        let idx_recording_hls_session = format!(
            "CREATE INDEX IF NOT EXISTS idx_recording_hls_session ON {}(session_id)",
            TABLE_RECORDING_HLS
        );
        sqlx::query(&idx_recording_hls_session)
            .execute(&self.pool)
            .await?;

        let idx_camera_status = format!(
            "CREATE INDEX IF NOT EXISTS idx_camera_status ON {}(camera_id, status)",
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&idx_camera_status)
            .execute(&self.pool)
            .await?;

        // Create throughput stats table
        let create_throughput_stats_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                bytes_per_second INTEGER NOT NULL,
                frame_count INTEGER NOT NULL,
                ffmpeg_fps REAL NOT NULL,
                connection_count INTEGER NOT NULL,
                PRIMARY KEY (camera_id, timestamp)
            )
            "#,
            TABLE_THROUGHPUT_STATS
        );
        sqlx::query(&create_throughput_stats_query)
            .execute(&self.pool)
            .await?;

        // Add index for throughput stats queries
        let idx_throughput_camera_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_throughput_camera_time ON {}(camera_id, timestamp)",
            TABLE_THROUGHPUT_STATS
        );
        sqlx::query(&idx_throughput_camera_time)
            .execute(&self.pool)
            .await?;

        info!("SQLite database initialization completed in {:?}", init_start.elapsed());
        Ok(())
    }

    async fn create_recording_session(
        &self,
        camera_id: &str,
        reason: Option<&str>,
        start_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64> {
        let query = format!(
            r#"
            INSERT INTO {} (camera_id, start_time, reason)
            VALUES (?, ?, ?)
            "#,
            TABLE_RECORDING_SESSIONS
        );
        let result = sqlx::query(&query)
        .bind(camera_id)
        .bind(start_time)
        .bind(reason)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    async fn stop_recording_session(&self, session_id: i64) -> Result<()> {
        let query = format!(
            "UPDATE {} SET end_time = ?, status = 'stopped' WHERE session_id = ?",
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&query)
        .bind(Utc::now())
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_active_recordings(&self, camera_id: &str) -> Result<Vec<RecordingSession>> {
        let query = format!(
            "SELECT session_id, camera_id, start_time, end_time, reason, status, COALESCE(keep_session, 0) as keep_session FROM {} WHERE camera_id = ? AND status = 'active'",
            TABLE_RECORDING_SESSIONS
        );
        let rows = sqlx::query(&query)
        .bind(camera_id)
        .fetch_all(&self.pool)
        .await?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(RecordingSession {
                session_id: row.get("session_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
                keep_session: row.get("keep_session"),
            });
        }

        Ok(sessions)
    }

    async fn add_recorded_frame(
        &self,
        session_id: i64,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        _frame_number: i64,
        frame_data: &[u8],
    ) -> Result<i64> {
        // Acquire read lock - allows concurrent frame writes but blocks during cleanup
        let _lock = self.cleanup_lock.read().await;

        let query = format!(
            r#"
            INSERT INTO {} (session_id, camera_id, timestamp, frame_data)
            VALUES (?, ?, ?, ?)
            "#,
            TABLE_RECORDING_MJPEG
        );
        let result = sqlx::query(&query)
        .bind(session_id)
        .bind(camera_id)
        .bind(timestamp)
        .bind(frame_data)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
    }

    async fn add_recorded_frames_bulk(
        &self,
        session_id: i64,
        camera_id: &str,
        frames: &[(DateTime<Utc>, i64, Vec<u8>)],
    ) -> Result<u64> {
        if frames.is_empty() {
            return Ok(0);
        }

        // Acquire read lock - allows concurrent frame writes but blocks during cleanup
        let _lock = self.cleanup_lock.read().await;

        debug!("SQLite bulk insert: inserting {} frames for session {} camera {}", frames.len(), session_id, camera_id);
        let start_time = std::time::Instant::now();

        // Build bulk insert query with placeholders
        let placeholders = frames.iter()
            .map(|_| "(?, ?, ?, ?)")
            .collect::<Vec<_>>()
            .join(", ");

        let query = format!(
            r#"
            INSERT INTO {} (session_id, camera_id, timestamp, frame_data)
            VALUES {}
            "#,
            TABLE_RECORDING_MJPEG, placeholders
        );

        // Create query builder and bind all parameters
        let mut query_builder = sqlx::query(&query);
        for frame in frames {
            query_builder = query_builder
                .bind(session_id)
                .bind(camera_id)
                .bind(frame.0)
                .bind(&frame.2);
        }
        
        let result = query_builder.execute(&self.pool).await?;
        
        let elapsed = start_time.elapsed();
        debug!(
            "SQLite bulk insert completed in {:.3}ms, inserted {} frames",
            elapsed.as_secs_f64() * 1000.0,
            result.rows_affected()
        );
        
        Ok(result.rows_affected() as u64)
    }

    async fn list_recordings(&self, query: &RecordingQuery) -> Result<Vec<RecordingSession>> {
        let start_time = std::time::Instant::now();
        
        let mut conditions = Vec::new();
        let mut bind_values: Vec<String> = Vec::new();
        
        if let Some(ref camera_id) = query.camera_id {
            conditions.push("camera_id = ?");
            bind_values.push(camera_id.clone());
        }
        
        if let Some(from) = query.from {
            conditions.push("start_time >= ?");
            bind_values.push(from.to_rfc3339());
        }
        
        if let Some(to) = query.to {
            conditions.push("start_time <= ?");
            bind_values.push(to.to_rfc3339());
        }
        
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        
        let sql = format!("SELECT session_id, camera_id, start_time, end_time, reason, status, COALESCE(keep_session, 0) as keep_session FROM {}{} ORDER BY start_time DESC", TABLE_RECORDING_SESSIONS, where_clause);

        tracing::debug!(
            "Executing SQL query for list_recordings:\n{}\nParameters: {:?}",
            sql, bind_values
        );

        let mut query_builder = sqlx::query(&sql);
        for value in &bind_values {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder.fetch_all(&self.pool).await?;

        let elapsed = start_time.elapsed();
        let row_count = rows.len();

        tracing::debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(RecordingSession {
                session_id: row.get("session_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
                keep_session: row.get("keep_session"),
            });
        }

        Ok(sessions)
    }

    async fn list_recordings_filtered(&self, camera_id: &str, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>, reason: Option<&str>) -> Result<Vec<RecordingSession>> {
        let start_time = std::time::Instant::now();
        
        let mut conditions = Vec::new();
        conditions.push("camera_id = ?".to_string());
        
        // Add time filters if provided
        if from.is_some() {
            conditions.push("start_time >= ?".to_string());
        }
        if to.is_some() {
            conditions.push("start_time <= ?".to_string());
        }
        
        // Add reason filter if provided (supports SQL wildcards)
        if reason.is_some() {
            conditions.push("reason LIKE ?".to_string());
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));
        
        let sql = format!(
            "SELECT session_id, camera_id, start_time, end_time, reason, status, COALESCE(keep_session, 0) as keep_session FROM {} {} ORDER BY start_time DESC",
            TABLE_RECORDING_SESSIONS, where_clause
        );

        tracing::debug!(
            "Executing SQL query for list_recordings_filtered:\n{}\nParameters: camera_id='{}', from='{:?}', to='{:?}', reason='{:?}'",
            sql, camera_id, from, to, reason
        );

        // Build the query with proper parameter binding
        let mut query = sqlx::query(&sql).bind(camera_id);

        if let Some(from_time) = from {
            query = query.bind(from_time);
        }
        if let Some(to_time) = to {
            query = query.bind(to_time);
        }
        if let Some(reason_filter) = reason {
            query = query.bind(reason_filter);
        }

        let rows = query.fetch_all(&self.pool).await?;

        let elapsed = start_time.elapsed();
        let row_count = rows.len();

        tracing::debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(RecordingSession {
                session_id: row.get("session_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
                keep_session: row.get("keep_session"),
            });
        }

        Ok(sessions)
    }

    async fn get_recorded_frames(
        &self,
        session_id: i64,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordedFrame>> {
        let start_time = std::time::Instant::now();
        
        let mut sql = format!("SELECT * FROM {} WHERE session_id = ?", TABLE_RECORDING_MJPEG);
        
        if from.is_some() {
            sql.push_str(" AND timestamp >= ?");
        }
        if to.is_some() {
            sql.push_str(" AND timestamp <= ?");
        }
        
        sql.push_str(" ORDER BY timestamp ASC");
        
        tracing::debug!(
            "Executing SQL query for get_recorded_frames:\n{}\nParameters: session_id={}, from={:?}, to={:?}",
            sql, session_id, from, to
        );

        let mut query = sqlx::query(&sql).bind(session_id);
        
        if let Some(from_time) = from {
            query = query.bind(from_time);
        }
        if let Some(to_time) = to {
            query = query.bind(to_time);
        }

        let rows = query.fetch_all(&self.pool).await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        tracing::debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut frames = Vec::new();
        for row in rows {
            frames.push(RecordedFrame {
                timestamp: row.get("timestamp"),
                frame_data: row.get("frame_data"),
            });
        }

        Ok(frames)
    }

    async fn delete_old_frames(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        let start_time = std::time::Instant::now();
        let cam_desc = camera_id.unwrap_or("all cameras");

        // Find sessions to clean up (oldest first, not marked as keep, ended before retention cutoff)
        let sessions_to_clean: Vec<(i64, String)> = if let Some(cam_id) = camera_id {
            let query = format!(
                r#"
                SELECT session_id, camera_id FROM {}
                WHERE camera_id = ?
                  AND end_time IS NOT NULL
                  AND end_time < ?
                  AND keep_session = 0
                ORDER BY end_time ASC
                "#,
                TABLE_RECORDING_SESSIONS
            );
            sqlx::query_as(&query)
                .bind(cam_id)
                .bind(older_than)
                .fetch_all(&self.pool).await?
        } else {
            let query = format!(
                r#"
                SELECT session_id, camera_id FROM {}
                WHERE end_time IS NOT NULL
                  AND end_time < ?
                  AND keep_session = 0
                ORDER BY end_time ASC
                "#,
                TABLE_RECORDING_SESSIONS
            );
            sqlx::query_as(&query)
                .bind(older_than)
                .fetch_all(&self.pool).await?
        };

        if sessions_to_clean.is_empty() {
            tracing::info!("No expired sessions to clean up for {}", cam_desc);
            return Ok(0);
        }

        tracing::info!(
            "Found {} expired sessions to clean up for {}, processing oldest first...",
            sessions_to_clean.len(),
            cam_desc
        );

        let mut total_deleted: u64 = 0;
        let mut sessions_processed = 0;

        for (session_id, session_camera_id) in &sessions_to_clean {
            let session_start = std::time::Instant::now();

            // Delete all frames for this session
            let delete_query = format!(
                "DELETE FROM {} WHERE session_id = ?",
                TABLE_RECORDING_MJPEG
            );
            let deleted = sqlx::query(&delete_query)
                .bind(session_id)
                .execute(&self.pool).await?
                .rows_affected();

            total_deleted += deleted;
            sessions_processed += 1;

            let session_elapsed = session_start.elapsed();

            if deleted > 0 {
                tracing::debug!(
                    "Session {}: deleted {} frames for camera '{}' in {:.2}s (total: {} frames, {}/{} sessions)",
                    session_id,
                    deleted,
                    session_camera_id,
                    session_elapsed.as_secs_f64(),
                    total_deleted,
                    sessions_processed,
                    sessions_to_clean.len()
                );
            }

            // Yield periodically to allow other tasks to run
            if sessions_processed % 10 == 0 {
                tokio::task::yield_now().await;
            }
        }

        let elapsed = start_time.elapsed();

        tracing::info!(
            "Completed frame cleanup: {} frames deleted from {} sessions in {:.1}s for {}",
            total_deleted,
            sessions_processed,
            elapsed.as_secs_f64(),
            cam_desc
        );

        Ok(total_deleted as usize)
    }
    
    async fn delete_unused_sessions(
        &self,
        camera_id: Option<&str>,
    ) -> Result<usize> {
        // Delete sessions that have:
        // 1. No frames in recording_mjpeg table
        // 2. No segments in recording_mp4 table
        // 3. Are not currently active (end_time is not NULL)
        // Uses EXISTS for efficient index lookups instead of NOT IN with full table scans

        let start_time = std::time::Instant::now();

        let result = if let Some(cam_id) = camera_id {
            // Delete unused sessions for a specific camera, but don't delete sessions marked to keep
            let query = format!(
                r#"
                DELETE FROM {sessions}
                WHERE camera_id = ?
                AND end_time IS NOT NULL
                AND keep_session = 0
                AND NOT EXISTS (
                    SELECT 1 FROM {mjpeg} WHERE session_id = {sessions}.session_id
                )
                AND NOT EXISTS (
                    SELECT 1 FROM {mp4} WHERE session_id = {sessions}.session_id
                )
                "#,
                sessions = TABLE_RECORDING_SESSIONS,
                mjpeg = TABLE_RECORDING_MJPEG,
                mp4 = TABLE_RECORDING_MP4
            );
            sqlx::query(&query)
                .bind(cam_id)
                .execute(&self.pool)
                .await?
        } else {
            // Delete unused sessions for all cameras, but don't delete sessions marked to keep
            let query = format!(
                r#"
                DELETE FROM {sessions}
                WHERE end_time IS NOT NULL
                AND keep_session = 0
                AND NOT EXISTS (
                    SELECT 1 FROM {mjpeg} WHERE session_id = {sessions}.session_id
                )
                AND NOT EXISTS (
                    SELECT 1 FROM {mp4} WHERE session_id = {sessions}.session_id
                )
                "#,
                sessions = TABLE_RECORDING_SESSIONS,
                mjpeg = TABLE_RECORDING_MJPEG,
                mp4 = TABLE_RECORDING_MP4
            );
            sqlx::query(&query)
                .execute(&self.pool)
                .await?
        };
        
        let deleted_sessions = result.rows_affected();
        let elapsed = start_time.elapsed();
        
        if deleted_sessions > 0 {
            tracing::info!(
                "Deleted {} unused sessions in {:.3}ms{}",
                deleted_sessions,
                elapsed.as_secs_f64() * 1000.0,
                if let Some(cam_id) = camera_id {
                    format!(" for camera '{}'", cam_id)
                } else {
                    String::new()
                }
            );
        } else {
            tracing::info!(
                "No unused sessions to delete (query took {:.3}ms)",
                elapsed.as_secs_f64() * 1000.0
            );
        }
        
        Ok(deleted_sessions as usize)
    }
    
    async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        tolerance_seconds: Option<i64>,
    ) -> Result<Option<RecordedFrame>> {
        let tolerance = tolerance_seconds.unwrap_or(0);
        
        if tolerance == 0 {
            // Exact timestamp match using idx_camera_timestamp index
            let query = format!(
                r#"
                SELECT timestamp, frame_data
                FROM {}
                WHERE camera_id = ? AND timestamp = ?
                LIMIT 1
                "#,
                TABLE_RECORDING_MJPEG
            );
            let row = sqlx::query(&query)
                .bind(camera_id)
                .bind(timestamp)
                .fetch_optional(&self.pool)
                .await?;

            if let Some(row) = row {
                return Ok(Some(RecordedFrame {
                    timestamp: row.get("timestamp"),
                    frame_data: row.get("frame_data"),
                }));
            }
        }

        // Find the closest frame within tolerance using idx_camera_timestamp index
        let tolerance_duration = chrono::Duration::seconds(tolerance);
        let time_before = timestamp - tolerance_duration;
        let time_after = timestamp + tolerance_duration;

        let query = format!(
            r#"
            SELECT timestamp, frame_data,
                   ABS(julianday(timestamp) - julianday(?)) as time_diff
            FROM {}
            WHERE camera_id = ?
              AND timestamp >= ?
              AND timestamp <= ?
            ORDER BY time_diff ASC
            LIMIT 1
            "#,
            TABLE_RECORDING_MJPEG
        );
        let row = sqlx::query(&query)
            .bind(timestamp)
            .bind(camera_id)
            .bind(time_before)
            .bind(time_after)
            .fetch_optional(&self.pool)
            .await?;
        
        if let Some(row) = row {
            Ok(Some(RecordedFrame {
                timestamp: row.get("timestamp"),
                frame_data: row.get("frame_data"),
            }))
        } else {
            Ok(None)
        }
    }
    
    async fn create_frame_stream(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Box<dyn FrameStream>> {
        let stream = SqliteFrameStream::new(&self.pool, camera_id.to_string(), from, to).await?;
        Ok(Box::new(stream))
    }
    
    async fn get_database_size(&self) -> Result<i64> {
        let row = sqlx::query(
            r#"
            SELECT (page_count * page_size) AS size_bytes
            FROM pragma_page_count(), pragma_page_size()
            "#
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(row.get("size_bytes"))
    }

    async fn add_video_segment(&self, segment: &VideoSegment) -> Result<i64> {
        // Acquire read lock - allows concurrent writes but blocks during cleanup
        let _lock = self.cleanup_lock.read().await;

        let query = format!(
            r#"
            INSERT INTO {} (camera_id, session_id, start_time, end_time, file_path, size_bytes, mp4_data)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
            TABLE_RECORDING_MP4
        );
        let result = sqlx::query(&query)
        .bind(&segment.camera_id)
        .bind(segment.session_id)
        .bind(segment.start_time)
        .bind(segment.end_time)
        .bind(&segment.file_path)
        .bind(segment.size_bytes)
        .bind(&segment.mp4_data)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
    }

    async fn list_video_segments(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<VideoSegment>> {
        let start_time = std::time::Instant::now();
        
        let query_str = format!(r#"
            SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes,
                   rs.reason as recording_reason, rs.camera_id
            FROM {} vs
            JOIN {} rs ON vs.session_id = rs.session_id
            WHERE rs.camera_id = ? AND vs.start_time < ? AND vs.end_time > ?
            ORDER BY vs.start_time ASC
            "#, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS);
        
        tracing::debug!(
            "Executing SQL query for list_video_segments:\n{}\nParameters: camera_id='{}', from='{}', to='{}'",
            query_str, camera_id, from, to
        );
        
        let rows = sqlx::query(&query_str)
        .bind(camera_id)
        .bind(to)
        .bind(from)
        .fetch_all(&self.pool)
        .await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        tracing::debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut segments = Vec::new();
        for row in rows {
            segments.push(VideoSegment {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                file_path: row.get("file_path"),
                size_bytes: row.get("size_bytes"),
                mp4_data: None,  // Not loaded for listing performance
                recording_reason: row.get("recording_reason"),
                camera_id: row.get("camera_id"),
            });
        }

        Ok(segments)
    }

    async fn list_video_segments_filtered(
        &self,
        camera_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        reason: Option<&str>,
        limit: i64,
        sort_order: &str,
    ) -> Result<Vec<VideoSegment>> {
        let start_time = std::time::Instant::now();
        
        let mut conditions = vec!["rs.camera_id = ?"];
        let mut bind_values: Vec<Box<dyn std::any::Any + Send>> = vec![Box::new(camera_id.to_string())];

        if let Some(from_time) = from {
            conditions.push("vs.end_time > ?");
            bind_values.push(Box::new(from_time));
        }

        if let Some(to_time) = to {
            conditions.push("vs.start_time < ?");
            bind_values.push(Box::new(to_time));
        }

        if let Some(reason_filter) = reason {
            conditions.push("rs.reason LIKE ?");
            bind_values.push(Box::new(format!("%{}%", reason_filter)));
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));
        
        let order_direction = match sort_order {
            "oldest" => "ASC",
            _ => "DESC", // default to newest first
        };

        let query_str = format!(r#"
            SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes,
                   rs.reason as recording_reason, rs.camera_id
            FROM {} vs
            JOIN {} rs ON vs.session_id = rs.session_id
            {}
            ORDER BY vs.start_time {}
            LIMIT ?
            "#, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS, where_clause, order_direction);
        
        tracing::debug!(
            "Executing SQL query for list_video_segments_filtered:\n{}\nParameters: camera_id='{}', from='{:?}', to='{:?}', reason='{:?}', limit={}, sort_order='{}'",
            query_str, camera_id, from, to, reason, limit, sort_order
        );
        
        let mut query = sqlx::query(&query_str);
        
        // Bind parameters in order
        query = query.bind(camera_id);
        if let Some(from_time) = from {
            query = query.bind(from_time);
        }
        if let Some(to_time) = to {
            query = query.bind(to_time);
        }
        if let Some(reason_filter) = reason {
            query = query.bind(format!("%{}%", reason_filter));
        }
        query = query.bind(limit);
        
        let rows = query.fetch_all(&self.pool).await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        tracing::debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut segments = Vec::new();
        for row in rows {
            segments.push(VideoSegment {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                file_path: row.get("file_path"),
                size_bytes: row.get("size_bytes"),
                mp4_data: None,  // Not loaded for listing performance
                recording_reason: row.get("recording_reason"),
                camera_id: row.get("camera_id"),
            });
        }

        Ok(segments)
    }

    async fn delete_old_video_segments(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        let start_time = std::time::Instant::now();
        
        // First, select the file paths of the segments to be deleted, excluding sessions marked to keep
        let (_query, segments_to_delete) = if let Some(cam_id) = camera_id {
            // Delete segments for a specific camera, but not for sessions marked to keep
            let query = format!(
                r#"
                SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes, vs.mp4_data
                FROM {} vs
                JOIN {} rs ON vs.session_id = rs.session_id
                WHERE rs.camera_id = ? AND vs.end_time < ? AND rs.keep_session = 0
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            let segments = sqlx::query_as::<_, VideoSegment>(&query)
                .bind(cam_id)
                .bind(older_than)
                .fetch_all(&self.pool)
                .await?;
            (query, segments)
        } else {
            // Delete segments for all cameras, but not for sessions marked to keep
            let query = format!(
                r#"
                SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes, vs.mp4_data
                FROM {} vs
                JOIN {} rs ON vs.session_id = rs.session_id
                WHERE vs.end_time < ? AND rs.keep_session = 0
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            let segments = sqlx::query_as::<_, VideoSegment>(&query)
                .bind(older_than)
                .fetch_all(&self.pool)
                .await?;
            (query, segments)
        };

        // Delete the files from the filesystem (only if they have file_path set)
        for segment in &segments_to_delete {
            if let Some(file_path) = &segment.file_path {
                if let Err(e) = tokio::fs::remove_file(file_path).await {
                    tracing::error!("Failed to delete video segment file {}: {}", file_path, e);
                }
            }
            // No action needed for database-stored segments - they'll be deleted with the record
        }

        // Then, delete the records from the database, but only for sessions not marked to keep
        let delete_result = if let Some(cam_id) = camera_id {
            let delete_query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT vs.session_id 
                    FROM {} vs
                    JOIN {} rs ON vs.session_id = rs.session_id
                    WHERE rs.camera_id = ? AND vs.end_time < ? AND rs.keep_session = 0
                )
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&delete_query)
                .bind(cam_id)
                .bind(older_than)
                .execute(&self.pool)
                .await?
        } else {
            let delete_query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT session_id FROM {} WHERE keep_session = 0
                ) AND end_time < ?
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&delete_query)
                .bind(older_than)
                .execute(&self.pool)
                .await?
        };

        let deleted_count = delete_result.rows_affected() as usize;
        let elapsed = start_time.elapsed();
        
        if deleted_count > 0 {
            tracing::info!(
                "Deleted {} video segments in {:.3}ms{}",
                deleted_count,
                elapsed.as_secs_f64() * 1000.0,
                if let Some(cam_id) = camera_id {
                    format!(" for camera '{}'", cam_id)
                } else {
                    String::new()
                }
            );
        } else {
            tracing::info!(
                "No video segments to delete (query took {:.3}ms)",
                elapsed.as_secs_f64() * 1000.0
            );
        }

        Ok(deleted_count)
    }

    async fn cleanup_database(
        &self,
        config: &crate::config::RecordingConfig,
        camera_configs: &std::collections::HashMap<String, crate::config::CameraConfig>,
    ) -> Result<()> {
        // Acquire write lock - blocks all frame writes during cleanup to prevent "database is locked" errors
        let _lock = self.cleanup_lock.write().await;
        tracing::debug!("Acquired cleanup write lock for SQLite database");

        // Extract camera_id from the database path if this is a per-camera database
        // The path format is typically "recordings/{camera_id}.db"
        let camera_id = if let Ok(mut connection) = self.pool.acquire().await {
            // Try to get the camera_id from the first recording session in this database
            let query = format!("SELECT DISTINCT camera_id FROM {} LIMIT 1", TABLE_RECORDING_SESSIONS);
            if let Ok(row) = sqlx::query(&query).fetch_optional(connection.as_mut()).await {
                row.and_then(|r| r.try_get::<String, _>("camera_id").ok())
            } else {
                None
            }
        } else {
            None
        };

        // Get camera-specific config or use global config
        let (frame_retention, video_retention, mp4_storage_type, hls_enabled, hls_retention) = if let Some(cam_id) = &camera_id {
            if let Some(camera_config) = camera_configs.get(cam_id) {
                // Use camera-specific retention settings if available, otherwise fall back to global
                let frame_retention = camera_config.get_frame_storage_retention()
                    .unwrap_or(&config.frame_storage_retention);
                let video_retention = camera_config.get_mp4_storage_retention()
                    .unwrap_or(&config.mp4_storage_retention);
                let video_type = camera_config.get_mp4_storage_type()
                    .unwrap_or(&config.mp4_storage_type);
                let hls_enabled = camera_config.get_hls_storage_enabled()
                    .unwrap_or(config.hls_storage_enabled);
                let hls_retention = camera_config.get_hls_storage_retention()
                    .unwrap_or(&config.hls_storage_retention);
                (frame_retention.clone(), video_retention.clone(), video_type.clone(), hls_enabled, hls_retention.clone())
            } else {
                // Camera not found in configs, use global settings
                (config.frame_storage_retention.clone(), 
                 config.mp4_storage_retention.clone(),
                 config.mp4_storage_type.clone(),
                 config.hls_storage_enabled,
                 config.hls_storage_retention.clone())
            }
        } else {
            // No camera_id found, use global settings
            (config.frame_storage_retention.clone(), 
             config.mp4_storage_retention.clone(),
             config.mp4_storage_type.clone(),
             config.hls_storage_enabled,
             config.hls_storage_retention.clone())
        };

        // Cleanup frames with camera-specific or global retention
        if config.frame_storage_enabled {
            // Check if retention is explicitly disabled with "0"
            if frame_retention != "0" {
                if let Ok(duration) = humantime::parse_duration(&frame_retention) {
                    if duration.as_secs() > 0 {
                        let older_than = Utc::now() - chrono::Duration::from_std(duration).unwrap();
                        tracing::info!("Starting frame cleanup (retention: {})", frame_retention);
                        if let Err(e) = self.delete_old_frames(camera_id.as_deref(), older_than).await {
                            tracing::error!("Error deleting old frames: {}", e);
                        }
                    }
                }
            } else {
                tracing::debug!("Frame retention disabled (0) for camera {:?}", camera_id);
            }
        }

        // Cleanup video segments with camera-specific or global retention
        if mp4_storage_type != crate::config::Mp4StorageType::Disabled {
            // Check if retention is explicitly disabled with "0"
            if video_retention != "0" {
                if let Ok(duration) = humantime::parse_duration(&video_retention) {
                    if duration.as_secs() > 0 {
                        let older_than = Utc::now() - chrono::Duration::from_std(duration).unwrap();
                        tracing::info!("Starting video segment cleanup (retention: {})", video_retention);
                        if let Err(e) = self.delete_old_video_segments(camera_id.as_deref(), older_than).await {
                            tracing::error!("Error deleting old video segments: {}", e);
                        }
                    }
                }
            } else {
                tracing::debug!("MP4 retention disabled (0) for camera {:?}", camera_id);
            }
        }

        // Cleanup HLS segments with camera-specific or global retention
        if hls_enabled {
            // Check if retention is explicitly disabled with "0"
            if hls_retention != "0" {
                if let Ok(duration) = humantime::parse_duration(&hls_retention) {
                    if duration.as_secs() > 0 {
                        tracing::info!("Starting HLS segment cleanup (retention: {})", hls_retention);
                        match self.delete_old_recording_hls_segments(&hls_retention, camera_id.as_deref()).await {
                            Ok(deleted_count) => {
                                tracing::info!("Deleted {} old HLS segments", deleted_count);
                            }
                        Err(e) => {
                            tracing::error!("Error deleting old HLS segments: {}", e);
                        }
                    }
                }
            }
            } else {
                tracing::debug!("HLS retention disabled (0) for camera {:?}", camera_id);
            }
        }

        // Finally, cleanup unused sessions (sessions with no frames or videos)
        // This should be done after deleting frames and videos to catch newly orphaned sessions
        tracing::info!("Starting unused session cleanup");
        if let Err(e) = self.delete_unused_sessions(camera_id.as_deref()).await {
            tracing::error!("Error deleting unused sessions: {}", e);
        }

        // Vacuum database to reclaim disk space after cleanup
        if let Err(e) = self.vacuum_tables().await {
            tracing::error!("Error vacuuming database: {}", e);
        }

        Ok(())
    }


    async fn get_video_segment_by_time(
        &self,
        camera_id: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<VideoSegment>> {
        let query = format!(r#"
            SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes, vs.mp4_data, rs.camera_id
            FROM {} vs
            JOIN {} rs ON vs.session_id = rs.session_id
            WHERE rs.camera_id = ? AND vs.start_time = ?
            "#, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS);
        
        debug!(
            "Executing SQLite query for get_video_segment_by_time:\n{}\nParameters: camera_id='{}', timestamp='{}'",
            query, camera_id, timestamp
        );
        
        let start_time = std::time::Instant::now();
        let row = sqlx::query(&query)
            .bind(camera_id)
            .bind(timestamp)
            .fetch_optional(&self.pool)
            .await?;
        
        let elapsed = start_time.elapsed();
        debug!(
            "SQLite query completed in {:.3}ms, found: {}",
            elapsed.as_secs_f64() * 1000.0,
            row.is_some()
        );

        if let Some(row) = row {
            Ok(Some(VideoSegment {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                file_path: row.get("file_path"),
                size_bytes: row.get("size_bytes"),
                mp4_data: row.get("mp4_data"),
                recording_reason: None, // Not needed for segment streaming
                camera_id: row.get("camera_id"),
            }))
        } else {
            Ok(None)
        }
    }

    // HLS-specific methods
    
    /// Store an HLS playlist in the database
    async fn store_hls_playlist(&self, playlist: &HlsPlaylist) -> Result<()> {
        let query = format!(
            r#"
            INSERT OR REPLACE INTO {} (playlist_id, camera_id, start_time, end_time, segment_duration, playlist_content, created_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&query)
            .bind(&playlist.playlist_id)
            .bind(&playlist.camera_id)
            .bind(playlist.start_time)
            .bind(playlist.end_time)
            .bind(playlist.segment_duration)
            .bind(&playlist.playlist_content)
            .bind(playlist.created_at)
            .bind(playlist.expires_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Store HLS playlist and segments in a transaction
    async fn store_hls_playlist_with_segments(&self, playlist: &HlsPlaylist, segments: &[HlsSegment]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // First, store the playlist
        let playlist_query = format!(
            r#"
            INSERT OR REPLACE INTO {} (playlist_id, camera_id, start_time, end_time, segment_duration, playlist_content, created_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&playlist_query)
            .bind(&playlist.playlist_id)
            .bind(&playlist.camera_id)
            .bind(playlist.start_time)
            .bind(playlist.end_time)
            .bind(playlist.segment_duration)
            .bind(&playlist.playlist_content)
            .bind(playlist.created_at)
            .bind(playlist.expires_at)
            .execute(&mut *tx)
            .await?;

        // Then, store all segments
        let segment_query = format!(
            r#"
            INSERT OR REPLACE INTO {} (playlist_id, segment_name, segment_index, segment_data, size_bytes, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
            TABLE_HLS_SEGMENTS
        );

        for segment in segments {
            sqlx::query(&segment_query)
                .bind(&segment.playlist_id)
                .bind(&segment.segment_name)
                .bind(segment.segment_index)
                .bind(&segment.segment_data)
                .bind(segment.size_bytes)
                .bind(segment.created_at)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Store an HLS segment in the database
    async fn store_hls_segment(&self, segment: &HlsSegment) -> Result<()> {
        let query = format!(
            r#"
            INSERT OR REPLACE INTO {} (playlist_id, segment_name, segment_index, segment_data, size_bytes, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
            TABLE_HLS_SEGMENTS
        );
        sqlx::query(&query)
            .bind(&segment.playlist_id)
            .bind(&segment.segment_name)
            .bind(segment.segment_index)
            .bind(&segment.segment_data)
            .bind(segment.size_bytes)
            .bind(segment.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get an HLS playlist by ID if it hasn't expired
    async fn get_hls_playlist(&self, playlist_id: &str) -> Result<Option<HlsPlaylist>> {
        let query = format!(
            r#"
            SELECT playlist_id, camera_id, start_time, end_time, segment_duration, playlist_content, created_at, expires_at
            FROM {} 
            WHERE playlist_id = ? AND expires_at > CURRENT_TIMESTAMP
            "#,
            TABLE_HLS_PLAYLISTS
        );
        let row = sqlx::query(&query)
            .bind(playlist_id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            Ok(Some(HlsPlaylist {
                playlist_id: row.get("playlist_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                segment_duration: row.get("segment_duration"),
                playlist_content: row.get("playlist_content"),
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
            }))
        } else {
            Ok(None)
        }
    }

    /// Get an HLS segment by playlist ID and segment name
    async fn get_hls_segment(&self, playlist_id: &str, segment_name: &str) -> Result<Option<HlsSegment>> {
        let query = format!(
            r#"
            SELECT playlist_id, segment_name, segment_index, segment_data, size_bytes, created_at
            FROM {} 
            WHERE playlist_id = ? AND segment_name = ?
            "#,
            TABLE_HLS_SEGMENTS
        );
        let row = sqlx::query(&query)
            .bind(playlist_id)
            .bind(segment_name)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            Ok(Some(HlsSegment {
                playlist_id: row.get("playlist_id"),
                segment_name: row.get("segment_name"),
                segment_index: row.get("segment_index"),
                segment_data: row.get("segment_data"),
                size_bytes: row.get("size_bytes"),
                created_at: row.get("created_at"),
            }))
        } else {
            Ok(None)
        }
    }

    /// Clean up expired HLS playlists and their segments
    async fn cleanup_expired_hls(&self) -> Result<usize> {
        let mut tx = self.pool.begin().await?;

        // Delete expired segments first (due to foreign key constraint)
        let delete_segments_query = format!(
            r#"
            DELETE FROM {} 
            WHERE playlist_id IN (
                SELECT playlist_id FROM {} 
                WHERE expires_at <= CURRENT_TIMESTAMP
            )
            "#,
            TABLE_HLS_SEGMENTS, TABLE_HLS_PLAYLISTS
        );
        let segments_result = sqlx::query(&delete_segments_query)
            .execute(&mut *tx)
            .await?;

        // Delete expired playlists
        let delete_playlists_query = format!(
            "DELETE FROM {} WHERE expires_at <= CURRENT_TIMESTAMP",
            TABLE_HLS_PLAYLISTS
        );
        let playlists_result = sqlx::query(&delete_playlists_query)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        tracing::info!(
            "Cleaned up expired HLS data: {} playlists, {} segments",
            playlists_result.rows_affected(),
            segments_result.rows_affected()
        );

        Ok(playlists_result.rows_affected() as usize)
    }

    async fn add_recording_hls_segment(&self, segment: &RecordingHlsSegment) -> Result<i64> {
        let query = format!(
            r#"
            INSERT INTO {} (camera_id, session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            TABLE_RECORDING_HLS
        );

        let result = sqlx::query(&query)
            .bind(&segment.camera_id)
            .bind(segment.session_id)
            .bind(segment.segment_index)
            .bind(segment.start_time)
            .bind(segment.end_time)
            .bind(segment.duration_seconds)
            .bind(&segment.segment_data)
            .bind(segment.size_bytes)
            .execute(&self.pool)
            .await?;

        Ok(result.last_insert_rowid())
    }

    async fn list_recording_hls_segments(
        &self,
        session_id: i64,
        from_time: Option<DateTime<Utc>>,
        to_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordingHlsSegment>> {
        match (from_time, to_time) {
            (None, None) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = ? ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
            (Some(from), None) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = ? AND start_time >= ? ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .bind(from)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
            (None, Some(to)) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = ? AND end_time <= ? ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .bind(to)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
            (Some(from), Some(to)) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = ? AND start_time >= ? AND end_time <= ? ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .bind(from)
                    .bind(to)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
        }
    }

    async fn get_recording_hls_segments_for_timerange(
        &self,
        camera_id: &str,
        from_time: DateTime<Utc>,
        to_time: DateTime<Utc>,
    ) -> Result<Vec<RecordingHlsSegment>> {
        // Query for segments that overlap with the requested time range
        // A segment overlaps if its start is before the range end AND its end is after the range start
        let query = format!(
            r#"
            SELECT rh.session_id, rh.segment_index, rh.start_time, rh.end_time, rh.duration_seconds, 
                   rh.segment_data, rh.size_bytes, rh.created_at
            FROM {} rh
            JOIN {} rs ON rh.session_id = rs.session_id
            WHERE rs.camera_id = ? 
            AND rh.start_time <= ?  -- segment starts before or at range end
            AND rh.end_time >= ?     -- segment ends after or at range start
            ORDER BY rh.start_time ASC, rh.segment_index ASC
            "#,
            TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
        );
        
        let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
            .bind(camera_id)
            .bind(to_time)
            .bind(from_time)
            .fetch_all(&self.pool)
            .await?;
            
        Ok(segments)
    }

    async fn delete_old_recording_hls_segments(
        &self,
        retention_duration: &str,
        camera_id: Option<&str>,
    ) -> Result<usize> {
        let duration = humantime::parse_duration(retention_duration)
            .map_err(|e| crate::errors::StreamError::config(&format!("Invalid retention duration '{}': {}", retention_duration, e)))?;
        
        let cutoff_time = Utc::now() - chrono::Duration::from_std(duration)
            .map_err(|e| crate::errors::StreamError::config(&format!("Invalid duration: {}", e)))?;
        
        let result = if let Some(cam_id) = camera_id {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT rs.session_id FROM {} rs 
                    WHERE rs.camera_id = ? AND rs.start_time < ? AND rs.keep_session = 0
                )
                "#,
                TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
                .bind(cam_id)
                .bind(cutoff_time)
                .execute(&self.pool)
                .await?
        } else {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT session_id FROM {} WHERE keep_session = 0
                ) AND created_at < ?
                "#,
                TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
                .bind(cutoff_time)
                .execute(&self.pool)
                .await?
        };
        
        Ok(result.rows_affected() as usize)
    }

    async fn get_recording_hls_segment_by_session_and_index(
        &self,
        session_id: i64,
        segment_index: i32,
    ) -> Result<Option<RecordingHlsSegment>> {
        let query = format!(
            r#"
            SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at
            FROM {} 
            WHERE session_id = ? AND segment_index = ?
            "#,
            TABLE_RECORDING_HLS
        );
        
        let segment = sqlx::query_as::<_, RecordingHlsSegment>(&query)
            .bind(session_id)
            .bind(segment_index)
            .fetch_optional(&self.pool)
            .await?;
        
        Ok(segment)
    }

    async fn get_last_hls_segment_index_for_session(
        &self,
        session_id: i64,
    ) -> Result<Option<i32>> {
        let query = format!(
            "SELECT MAX(segment_index) as max_index FROM {} WHERE session_id = ?",
            TABLE_RECORDING_HLS
        );
        
        let result: Option<(Option<i32>,)> = sqlx::query_as(&query)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        
        Ok(result.and_then(|(max_index,)| max_index))
    }

    async fn set_session_keep_flag(
        &self,
        session_id: i64,
        keep_session: bool,
    ) -> Result<()> {
        let query = format!(
            "UPDATE {} SET keep_session = ? WHERE session_id = ?",
            TABLE_RECORDING_SESSIONS
        );

        sqlx::query(&query)
            .bind(keep_session)
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn delete_recording_session(&self, session_id: i64) -> Result<DeletedRecordingStats> {
        // First check if session is stopped
        let session_query = format!(
            "SELECT status FROM {} WHERE session_id = ?",
            TABLE_RECORDING_SESSIONS
        );
        let status: String = sqlx::query_scalar(&session_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        if status == "active" {
            return Err(crate::errors::StreamError::database(
                "Cannot delete active recording session. Stop it first.".to_string()
            ));
        }

        // Count items before deletion
        let frames_count_query = format!("SELECT COUNT(*) FROM {} WHERE session_id = ?", TABLE_RECORDING_MJPEG);
        let frames_deleted: i64 = sqlx::query_scalar(&frames_count_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        let mp4_count_query = format!("SELECT COUNT(*) FROM {} WHERE session_id = ?", TABLE_RECORDING_MP4);
        let mp4_segments_deleted: i64 = sqlx::query_scalar(&mp4_count_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        let hls_count_query = format!("SELECT COUNT(*) FROM {} WHERE session_id = ?", TABLE_RECORDING_HLS);
        let hls_segments_deleted: i64 = sqlx::query_scalar(&hls_count_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        // Get MP4 file paths for filesystem deletion
        let mp4_files_query = format!(
            "SELECT file_path FROM {} WHERE session_id = ? AND file_path IS NOT NULL",
            TABLE_RECORDING_MP4
        );
        let file_paths: Vec<String> = sqlx::query_scalar(&mp4_files_query)
            .bind(session_id)
            .fetch_all(&self.pool)
            .await?;

        // Delete from database in correct order (due to foreign keys)
        let delete_frames = format!("DELETE FROM {} WHERE session_id = ?", TABLE_RECORDING_MJPEG);
        sqlx::query(&delete_frames).bind(session_id).execute(&self.pool).await?;

        let delete_mp4 = format!("DELETE FROM {} WHERE session_id = ?", TABLE_RECORDING_MP4);
        sqlx::query(&delete_mp4).bind(session_id).execute(&self.pool).await?;

        let delete_hls = format!("DELETE FROM {} WHERE session_id = ?", TABLE_RECORDING_HLS);
        sqlx::query(&delete_hls).bind(session_id).execute(&self.pool).await?;

        let delete_session = format!("DELETE FROM {} WHERE session_id = ?", TABLE_RECORDING_SESSIONS);
        sqlx::query(&delete_session).bind(session_id).execute(&self.pool).await?;

        // Delete MP4 files from filesystem
        for file_path in file_paths {
            if let Err(e) = tokio::fs::remove_file(&file_path).await {
                tracing::warn!("Failed to delete MP4 file {}: {}", file_path, e);
            }
        }

        info!("Deleted recording session {} (frames: {}, mp4: {}, hls: {})",
              session_id, frames_deleted, mp4_segments_deleted, hls_segments_deleted);

        Ok(DeletedRecordingStats {
            session_id,
            frames_deleted: frames_deleted as u64,
            mp4_segments_deleted: mp4_segments_deleted as u64,
            hls_segments_deleted: hls_segments_deleted as u64,
        })
    }

    async fn delete_mp4_segment_by_filename(&self, camera_id: &str, filename: &str) -> Result<i64> {
        // Get the segment info before deletion
        let query = format!(
            "SELECT s.file_path, s.size_bytes, s.session_id
             FROM {} s
             JOIN {} r ON s.session_id = r.session_id
             WHERE r.camera_id = ? AND s.file_path LIKE ?",
            TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
        );

        let pattern = format!("%/{}", filename);
        let row: Option<(Option<String>, i64, i64)> = sqlx::query_as(&query)
            .bind(camera_id)
            .bind(&pattern)
            .fetch_optional(&self.pool)
            .await?;

        if let Some((file_path, size_bytes, session_id)) = row {
            // Check if session is stopped
            let status_query = format!("SELECT status FROM {} WHERE session_id = ?", TABLE_RECORDING_SESSIONS);
            let status: String = sqlx::query_scalar(&status_query)
                .bind(session_id)
                .fetch_one(&self.pool)
                .await?;

            if status == "active" {
                return Err(crate::errors::StreamError::database(
                    "Cannot delete MP4 from active recording session".to_string()
                ));
            }

            // Delete from database
            let delete_query = format!(
                "DELETE FROM {} WHERE session_id = ? AND file_path LIKE ?",
                TABLE_RECORDING_MP4
            );
            sqlx::query(&delete_query)
                .bind(session_id)
                .bind(&pattern)
                .execute(&self.pool)
                .await?;

            // Delete file from filesystem if it exists
            if let Some(path) = file_path {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    tracing::warn!("Failed to delete MP4 file {}: {}", path, e);
                }
            }

            Ok(size_bytes)
        } else {
            Err(crate::errors::StreamError::database(format!("MP4 segment {} not found", filename)))
        }
    }

    async fn delete_mp4_segments_bulk(&self, camera_id: &str, filenames: Vec<String>) -> Result<BulkDeleteResult> {
        let mut deleted_count = 0;
        let mut failed = Vec::new();
        let mut total_size_bytes = 0i64;

        for filename in filenames {
            match self.delete_mp4_segment_by_filename(camera_id, &filename).await {
                Ok(size) => {
                    deleted_count += 1;
                    total_size_bytes += size;
                }
                Err(e) => {
                    tracing::error!("Failed to delete {}: {}", filename, e);
                    failed.push(filename);
                }
            }
        }

        Ok(BulkDeleteResult {
            deleted_count,
            failed,
            total_size_bytes,
        })
    }

    async fn delete_hls_segments_by_session(&self, session_id: i64) -> Result<u64> {
        // Check if session is stopped
        let status_query = format!("SELECT status FROM {} WHERE session_id = ?", TABLE_RECORDING_SESSIONS);
        let status: String = sqlx::query_scalar(&status_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        if status == "active" {
            return Err(crate::errors::StreamError::database(
                "Cannot delete HLS from active recording session".to_string()
            ));
        }

        let delete_query = format!("DELETE FROM {} WHERE session_id = ?", TABLE_RECORDING_HLS);
        let result = sqlx::query(&delete_query)
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn delete_hls_segments_by_timerange(&self, camera_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<u64> {
        let delete_query = format!(
            "DELETE FROM {}
             WHERE session_id IN (
                 SELECT session_id FROM {} WHERE camera_id = ? AND status != 'active'
             )
             AND start_time >= ? AND end_time <= ?",
            TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
        );

        let result = sqlx::query(&delete_query)
            .bind(camera_id)
            .bind(from)
            .bind(to)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn vacuum_tables(&self) -> Result<()> {
        let start_time = std::time::Instant::now();
        tracing::info!("Starting SQLite VACUUM to reclaim disk space...");

        // SQLite VACUUM rebuilds the entire database file
        // This reclaims unused space and defragments the database
        sqlx::query("VACUUM")
            .execute(&self.pool)
            .await?;

        let elapsed = start_time.elapsed();
        tracing::info!("SQLite VACUUM completed in {:.1}s", elapsed.as_secs_f64());

        Ok(())
    }

    async fn record_throughput_stats(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        bytes_per_second: i64,
        frame_count: i32,
        ffmpeg_fps: f32,
        connection_count: i32,
    ) -> Result<()> {
        let query = format!(
            r#"
            INSERT OR REPLACE INTO {} (camera_id, timestamp, bytes_per_second, frame_count, ffmpeg_fps, connection_count)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
            TABLE_THROUGHPUT_STATS
        );
        sqlx::query(&query)
            .bind(camera_id)
            .bind(timestamp)
            .bind(bytes_per_second)
            .bind(frame_count)
            .bind(ffmpeg_fps)
            .bind(connection_count)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn get_throughput_stats(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<ThroughputStats>> {
        let query = format!(
            r#"
            SELECT camera_id, timestamp, bytes_per_second, frame_count, ffmpeg_fps, connection_count
            FROM {} 
            WHERE camera_id = ? AND timestamp >= ? AND timestamp <= ?
            ORDER BY timestamp ASC
            "#,
            TABLE_THROUGHPUT_STATS
        );
        let rows = sqlx::query(&query)
            .bind(camera_id)
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(ThroughputStats {
                camera_id: row.get("camera_id"),
                timestamp: row.get("timestamp"),
                bytes_per_second: row.get("bytes_per_second"),
                frame_count: row.get("frame_count"),
                ffmpeg_fps: row.get("ffmpeg_fps"),
                connection_count: row.get("connection_count"),
            });
        }

        Ok(stats)
    }

    async fn cleanup_old_throughput_stats(&self, older_than: DateTime<Utc>) -> Result<u64> {
        let query = format!(
            "DELETE FROM {} WHERE timestamp < ?",
            TABLE_THROUGHPUT_STATS
        );
        let result = sqlx::query(&query)
            .bind(older_than)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn get_mp4_segments_in_range(&self, camera_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<crate::export_jobs::Mp4SegmentInfo>> {
        let query = format!(
            r#"
            SELECT session_id, start_time, end_time, file_path
            FROM {}
            WHERE camera_id = ? AND end_time >= ? AND start_time <= ?
            ORDER BY start_time ASC
            "#,
            TABLE_RECORDING_MP4
        );

        let rows = sqlx::query(&query)
            .bind(camera_id)
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?;

        let mut segments = Vec::new();
        for row in rows {
            segments.push(crate::export_jobs::Mp4SegmentInfo {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                storage_path: row.get("file_path"),
            });
        }

        Ok(segments)
    }

    async fn extract_mp4_segment_to_file(&self, segment_id: i64, output_path: &str) -> Result<()> {
        let query = format!(
            "SELECT mp4_data FROM {} WHERE session_id = ?",
            TABLE_RECORDING_MP4
        );

        let row = sqlx::query(&query)
            .bind(segment_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StreamError::not_found(format!("MP4 segment {} not found", segment_id)))?;

        let data: Vec<u8> = row.get("mp4_data");

        std::fs::write(output_path, data)
            .map_err(|e| StreamError::internal(format!("Failed to write MP4 segment to file: {}", e)))?;

        Ok(())
    }
}

// PostgreSQL Database Implementation
pub struct PostgreSqlDatabase {
    pool: PgPool,
    database_name: String,
    is_shared_database: bool, // True if all cameras share same DB
}

// PostgreSQL-specific frame streaming implementation
pub struct PostgreSqlFrameStream {
    connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
    camera_id: String,
    to: DateTime<Utc>,
    current_timestamp: Option<DateTime<Utc>>,
    batch_size: i64,
    current_batch: Vec<RecordedFrame>,
    batch_index: usize,
    finished: bool,
}

impl PostgreSqlFrameStream {
    async fn new(
        pool: &PgPool,
        camera_id: String,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Self> {
        let connection = pool.acquire().await?;
        Ok(Self {
            connection,
            camera_id,
            to,
            current_timestamp: Some(from),
            batch_size: 50, // Process 50 frames at a time for memory efficiency
            current_batch: Vec::with_capacity(50), // Pre-allocate for efficiency
            batch_index: 0,
            finished: false,
        })
    }
    
    async fn fetch_next_batch(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        
        let current_ts = match self.current_timestamp {
            Some(ts) => ts,
            None => {
                self.finished = true;
                return Ok(());
            }
        };
        
        // Use camera_id directly from recording_mjpeg table with idx_camera_timestamp index
        let query = format!(
            r#"
            SELECT timestamp, frame_data
            FROM {}
            WHERE camera_id = $1
              AND timestamp >= $2
              AND timestamp <= $3
            ORDER BY timestamp ASC
            LIMIT $4
            "#,
            TABLE_RECORDING_MJPEG
        );
        let rows = sqlx::query(&query)
        .bind(&self.camera_id)
        .bind(current_ts)
        .bind(self.to)
        .bind(self.batch_size)
        .fetch_all(self.connection.as_mut())
        .await?;

        self.current_batch.clear();
        self.batch_index = 0;

        for row in rows {
            let timestamp: DateTime<Utc> = row.get("timestamp");
            let frame_data: Vec<u8> = row.get("frame_data");

            self.current_batch.push(RecordedFrame {
                timestamp,
                frame_data,
            });

            // Update current timestamp for next batch
            self.current_timestamp = Some(timestamp + chrono::Duration::microseconds(1));
        }

        // If we got fewer rows than requested, we've reached the end
        if self.current_batch.len() < self.batch_size as usize {
            self.finished = true;
        }

        Ok(())
    }
}

#[async_trait]
impl FrameStream for PostgreSqlFrameStream {
    async fn next_frame(&mut self) -> Result<Option<RecordedFrame>> {
        // If we've consumed all frames in current batch, fetch the next batch
        if self.batch_index >= self.current_batch.len() {
            self.fetch_next_batch().await?;
            
            // If still no frames after fetching, we're done
            if self.current_batch.is_empty() {
                return Ok(None);
            }
        }
        
        // Double-check that batch_index is within bounds after fetch
        if self.batch_index >= self.current_batch.len() {
            // This shouldn't happen, but protect against it
            error!("Unexpected state: batch_index {} >= batch length {} after fetch", 
                   self.batch_index, self.current_batch.len());
            return Ok(None);
        }
        
        // Return the next frame from current batch - use safe indexing
        let frame = match self.current_batch.get(self.batch_index) {
            Some(frame) => frame.clone(),
            None => {
                error!("Failed to get frame at index {} from batch of length {}", 
                       self.batch_index, self.current_batch.len());
                return Ok(None);
            }
        };
        
        self.batch_index += 1;
        Ok(Some(frame))
    }
    
    async fn close(&mut self) -> Result<()> {
        // PostgreSQL connection will be dropped automatically
        self.finished = true;
        self.current_batch.clear();
        self.current_batch.shrink_to_fit(); // Release memory
        self.current_timestamp = None;
        Ok(())
    }
    
    fn estimated_frame_count(&self) -> Option<usize> {
        // Could implement a count query here if needed
        None
    }
}

impl PostgreSqlDatabase {
    pub async fn new(database_url: &str, camera_id: Option<&str>) -> Result<Self> {
        let (base_url, provided_db_name) = Self::parse_database_url(database_url)?;
        let is_shared_database = provided_db_name.is_some();
        
        let database_name = if let Some(db_name) = provided_db_name {
            // Use the provided database name for all cameras
            db_name
        } else if let Some(cam_id) = camera_id {
            // Create a camera-specific database name
            Self::sanitize_database_name(&format!("rtsp_{}", cam_id))
        } else {
            return Err(crate::errors::StreamError::config("Camera ID is required when no database is specified in URL"));
        };
        
        // Create the database if it doesn't exist (only for per-camera databases)
        if !is_shared_database {
            Self::create_database_if_not_exists(&base_url, &database_name).await?;
        }
        
        // Connect to the specific database
        let full_url = format!("{}/{}", base_url.trim_end_matches('/'), database_name);
        info!("Connecting to PostgreSQL database: {}", database_name);
        let pool = PgPool::connect(&full_url).await?;
        
        Ok(Self { 
            pool,
            database_name: database_name.to_string(),
            is_shared_database,
        })
    }
    
    fn parse_database_url(url: &str) -> Result<(String, Option<String>)> {
        // Parse URL like "postgres://user:pass@localhost/" or "postgres://user:pass@localhost/dbname"
        if let Some(last_slash_pos) = url.rfind('/') {
            let base_part = &url[..last_slash_pos];
            let db_part = &url[last_slash_pos + 1..];
            
            if db_part.is_empty() {
                // URL ends with '/' - no database specified
                Ok((base_part.to_string(), None))
            } else {
                // Database name provided
                Ok((base_part.to_string(), Some(db_part.to_string())))
            }
        } else {
            Err(crate::errors::StreamError::config("Invalid database URL format"))
        }
    }
    
    fn sanitize_database_name(name: &str) -> String {
        // PostgreSQL database names should be lowercase and contain only alphanumeric characters and underscores
        name.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
            .collect()
    }
    
    async fn create_database_if_not_exists(base_url: &str, database_name: &str) -> Result<()> {
        // Connect to the default 'postgres' database to create new databases
        let admin_url = format!("{}/postgres", base_url);
        debug!("Connecting to admin database to create {}: {}", database_name, admin_url);
        
        let admin_pool = PgPool::connect(&admin_url).await
            .map_err(|e| crate::errors::StreamError::database(format!("Failed to connect to admin database: {}", e)))?;
        
        // Check if database exists
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(database_name)
            .fetch_one(&admin_pool)
            .await?;
        
        if !exists {
            info!("Creating PostgreSQL database: {}", database_name);
            // Note: Cannot use parameterized query for CREATE DATABASE
            let create_query = format!("CREATE DATABASE {}", database_name);
            sqlx::query(&create_query)
                .execute(&admin_pool)
                .await
                .map_err(|e| crate::errors::StreamError::database(format!("Failed to create database {}: {}", database_name, e)))?;
            info!("Successfully created PostgreSQL database: {}", database_name);
        } else {
            debug!("PostgreSQL database already exists: {}", database_name);
        }
        
        admin_pool.close().await;
        Ok(())
    }
}

#[async_trait]
impl DatabaseProvider for PostgreSqlDatabase {
    async fn initialize(&self) -> Result<()> {
        let create_sessions_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                session_id BIGSERIAL PRIMARY KEY,
                camera_id TEXT NOT NULL,
                start_time TIMESTAMPTZ NOT NULL,
                end_time TIMESTAMPTZ,
                reason TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                keep_session BOOLEAN NOT NULL DEFAULT false
            )
            "#,
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_sessions_query)
            .execute(&self.pool)
            .await?;

        let create_mjpeg_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                session_id BIGINT NOT NULL,
                timestamp TIMESTAMPTZ NOT NULL,
                frame_data BYTEA NOT NULL,
                PRIMARY KEY (camera_id, timestamp),
                FOREIGN KEY (session_id) REFERENCES {}(session_id)
            )
            "#,
            TABLE_RECORDING_MJPEG, TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_mjpeg_query)
            .execute(&self.pool)
            .await?;

        let idx_camera_timestamp = format!(
            "CREATE INDEX IF NOT EXISTS idx_camera_timestamp ON {}(camera_id, timestamp)",
            TABLE_RECORDING_MJPEG
        );
        sqlx::query(&idx_camera_timestamp)
            .execute(&self.pool)
            .await?;

        // Add index on session_id for FK lookups
        let idx_mjpeg_session = format!(
            "CREATE INDEX IF NOT EXISTS idx_recording_mjpeg_session ON {}(session_id)",
            TABLE_RECORDING_MJPEG
        );
        sqlx::query(&idx_mjpeg_session)
            .execute(&self.pool)
            .await?;

        let create_mp4_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                session_id BIGINT NOT NULL,
                start_time TIMESTAMPTZ NOT NULL,
                end_time TIMESTAMPTZ NOT NULL,
                file_path TEXT,
                size_bytes BIGINT NOT NULL,
                mp4_data BYTEA,
                PRIMARY KEY (camera_id, start_time),
                FOREIGN KEY (session_id) REFERENCES {}(session_id) ON DELETE CASCADE
            )
            "#,
            TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_mp4_query)
            .execute(&self.pool)
            .await?;

        let idx_segment_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_segment_time ON {}(start_time, end_time)",
            TABLE_RECORDING_MP4
        );
        sqlx::query(&idx_segment_time)
            .execute(&self.pool)
            .await?;
        
        // Add index on session_id for the JOIN operation
        let idx_segment_session = format!(
            "CREATE INDEX IF NOT EXISTS idx_segment_session ON {}(session_id)",
            TABLE_RECORDING_MP4
        );
        sqlx::query(&idx_segment_session)
            .execute(&self.pool)
            .await?;

        // Add indexes on recording_sessions for common query patterns
        let idx_camera_start_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_camera_start_time ON {}(camera_id, start_time)",
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&idx_camera_start_time)
            .execute(&self.pool)
            .await?;

        // Create HLS playlists table
        let create_hls_playlists_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                playlist_id TEXT PRIMARY KEY,
                camera_id TEXT NOT NULL,
                start_time TIMESTAMPTZ NOT NULL,
                end_time TIMESTAMPTZ NOT NULL,
                segment_duration INTEGER NOT NULL,
                playlist_content TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                expires_at TIMESTAMPTZ NOT NULL
            )
            "#,
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&create_hls_playlists_query)
            .execute(&self.pool)
            .await?;

        // Create HLS segments table
        let create_hls_segments_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                playlist_id TEXT NOT NULL,
                segment_name TEXT NOT NULL,
                segment_index INTEGER NOT NULL,
                segment_data BYTEA NOT NULL,
                size_bytes BIGINT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (playlist_id, segment_name),
                FOREIGN KEY (playlist_id) REFERENCES {}(playlist_id) ON DELETE CASCADE
            )
            "#,
            TABLE_HLS_SEGMENTS, TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&create_hls_segments_query)
            .execute(&self.pool)
            .await?;

        // Create recording HLS segments table
        let create_recording_hls_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                session_id BIGINT NOT NULL,
                segment_index INTEGER NOT NULL,
                start_time TIMESTAMPTZ NOT NULL,
                end_time TIMESTAMPTZ NOT NULL,
                duration_seconds DOUBLE PRECISION NOT NULL,
                segment_data BYTEA NOT NULL,
                size_bytes BIGINT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (camera_id, segment_index),
                FOREIGN KEY (session_id) REFERENCES {}(session_id) ON DELETE CASCADE
            )
            "#,
            TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_recording_hls_query)
            .execute(&self.pool)
            .await?;

        // Add indexes for HLS tables
        let idx_hls_playlists_camera = format!(
            "CREATE INDEX IF NOT EXISTS idx_hls_playlists_camera ON {}(camera_id, start_time, end_time)",
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&idx_hls_playlists_camera)
            .execute(&self.pool)
            .await?;

        let idx_hls_playlists_expires = format!(
            "CREATE INDEX IF NOT EXISTS idx_hls_playlists_expires ON {}(expires_at)",
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&idx_hls_playlists_expires)
            .execute(&self.pool)
            .await?;

        let idx_hls_segments_playlist = format!(
            "CREATE INDEX IF NOT EXISTS idx_hls_segments_playlist ON {}(playlist_id, segment_index)",
            TABLE_HLS_SEGMENTS
        );
        sqlx::query(&idx_hls_segments_playlist)
            .execute(&self.pool)
            .await?;

        let idx_recording_hls_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_recording_hls_time ON {}(start_time, end_time)",
            TABLE_RECORDING_HLS
        );
        sqlx::query(&idx_recording_hls_time)
            .execute(&self.pool)
            .await?;

        // Add index on session_id for FK lookups
        let idx_recording_hls_session = format!(
            "CREATE INDEX IF NOT EXISTS idx_recording_hls_session ON {}(session_id)",
            TABLE_RECORDING_HLS
        );
        sqlx::query(&idx_recording_hls_session)
            .execute(&self.pool)
            .await?;

        let idx_camera_status = format!(
            "CREATE INDEX IF NOT EXISTS idx_camera_status ON {}(camera_id, status)",
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&idx_camera_status)
            .execute(&self.pool)
            .await?;

        // Create throughput stats table
        let create_throughput_stats_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                camera_id TEXT NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                bytes_per_second INTEGER NOT NULL,
                frame_count INTEGER NOT NULL,
                ffmpeg_fps REAL NOT NULL,
                connection_count INTEGER NOT NULL,
                PRIMARY KEY (camera_id, timestamp)
            )
            "#,
            TABLE_THROUGHPUT_STATS
        );
        sqlx::query(&create_throughput_stats_query)
            .execute(&self.pool)
            .await?;

        // Add index for throughput stats queries
        let idx_throughput_camera_time = format!(
            "CREATE INDEX IF NOT EXISTS idx_throughput_camera_time ON {}(camera_id, timestamp)",
            TABLE_THROUGHPUT_STATS
        );
        sqlx::query(&idx_throughput_camera_time)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn create_recording_session(
        &self,
        camera_id: &str,
        reason: Option<&str>,
        start_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64> {
        let query = format!(
            r#"
            INSERT INTO {} (camera_id, start_time, reason)
            VALUES ($1, $2, $3)
            RETURNING session_id
            "#,
            TABLE_RECORDING_SESSIONS
        );
        let row = sqlx::query(&query)
        .bind(camera_id)
        .bind(start_time)
        .bind(reason)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("session_id"))
    }

    async fn stop_recording_session(&self, session_id: i64) -> Result<()> {
        let query = format!(
            "UPDATE {} SET end_time = $1, status = 'stopped' WHERE session_id = $2",
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&query)
        .bind(Utc::now())
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_active_recordings(&self, camera_id: &str) -> Result<Vec<RecordingSession>> {
        let query = format!(
            "SELECT session_id, camera_id, start_time, end_time, reason, status, COALESCE(keep_session, false) as keep_session FROM {} WHERE camera_id = $1 AND status = 'active'",
            TABLE_RECORDING_SESSIONS
        );
        let rows = sqlx::query(&query)
        .bind(camera_id)
        .fetch_all(&self.pool)
        .await?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(RecordingSession {
                session_id: row.get("session_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
                keep_session: row.get("keep_session"),
            });
        }

        Ok(sessions)
    }

    async fn add_recorded_frame(
        &self,
        session_id: i64,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        _frame_number: i64,
        frame_data: &[u8],
    ) -> Result<i64> {
        let query = format!(
            r#"
            INSERT INTO {} (session_id, camera_id, timestamp, frame_data)
            VALUES ($1, $2, $3, $4)
            "#,
            TABLE_RECORDING_MJPEG
        );
        let result = sqlx::query(&query)
        .bind(session_id)
        .bind(camera_id)
        .bind(timestamp)
        .bind(frame_data)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
    }

    async fn add_recorded_frames_bulk(
        &self,
        session_id: i64,
        camera_id: &str,
        frames: &[(DateTime<Utc>, i64, Vec<u8>)],
    ) -> Result<u64> {
        if frames.is_empty() {
            return Ok(0);
        }

        debug!("PostgreSQL bulk insert: inserting {} frames for session {} camera {}", frames.len(), session_id, camera_id);
        let start_time = std::time::Instant::now();

        // PostgreSQL supports UNNEST for efficient bulk inserts
        let query = format!(
            r#"
            INSERT INTO {} (session_id, camera_id, timestamp, frame_data)
            SELECT $1, $2, * FROM UNNEST($3::timestamptz[], $4::bytea[])
            "#,
            TABLE_RECORDING_MJPEG
        );

        // Collect timestamps and frame data into arrays
        let timestamps: Vec<DateTime<Utc>> = frames.iter().map(|(ts, _, _)| *ts).collect();
        let frame_data: Vec<Vec<u8>> = frames.iter().map(|(_, _, data)| data.clone()).collect();

        let result = sqlx::query(&query)
            .bind(session_id)
            .bind(camera_id)
            .bind(timestamps)
            .bind(frame_data)
            .execute(&self.pool)
            .await?;
        
        let elapsed = start_time.elapsed();
        debug!(
            "PostgreSQL bulk insert completed in {:.3}ms, inserted {} frames",
            elapsed.as_secs_f64() * 1000.0,
            result.rows_affected()
        );
        
        Ok(result.rows_affected() as u64)
    }

    async fn list_recordings(&self, query: &RecordingQuery) -> Result<Vec<RecordingSession>> {
        let start_time = std::time::Instant::now();
        
        let mut conditions = Vec::new();
        let mut bind_count = 0;
        
        let mut sql = format!("SELECT session_id, camera_id, start_time, end_time, reason, status, COALESCE(keep_session, false) as keep_session FROM {}", TABLE_RECORDING_SESSIONS);
        
        if query.camera_id.is_some() || query.from.is_some() || query.to.is_some() {
            sql.push_str(" WHERE ");
            
            if query.camera_id.is_some() {
                bind_count += 1;
                conditions.push(format!("camera_id = ${}", bind_count));
            }
            
            if query.from.is_some() {
                bind_count += 1;
                conditions.push(format!("start_time >= ${}", bind_count));
            }
            
            if query.to.is_some() {
                bind_count += 1;
                conditions.push(format!("start_time <= ${}", bind_count));
            }
            
            sql.push_str(&conditions.join(" AND "));
        }
        
        sql.push_str(" ORDER BY start_time DESC");
        
        debug!("Executing PostgreSQL query for list_recordings: {}", sql);
        
        let mut db_query = sqlx::query(&sql);
        
        if let Some(ref camera_id) = query.camera_id {
            db_query = db_query.bind(camera_id);
        }
        if let Some(from) = query.from {
            db_query = db_query.bind(from);
        }
        if let Some(to) = query.to {
            db_query = db_query.bind(to);
        }
        
        let rows = db_query.fetch_all(&self.pool).await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(RecordingSession {
                session_id: row.get("session_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
                keep_session: row.get("keep_session"),
            });
        }

        Ok(sessions)
    }

    async fn list_recordings_filtered(&self, camera_id: &str, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>, reason: Option<&str>) -> Result<Vec<RecordingSession>> {
        let start_time = std::time::Instant::now();
        
        let mut conditions = vec!["camera_id = $1".to_string()];
        let mut bind_count = 1;
        
        // Add time filters if provided
        if from.is_some() {
            bind_count += 1;
            conditions.push(format!("start_time >= ${}", bind_count));
        }
        if to.is_some() {
            bind_count += 1;
            conditions.push(format!("start_time <= ${}", bind_count));
        }
        
        // Add reason filter if provided (supports SQL wildcards)
        if reason.is_some() {
            bind_count += 1;
            conditions.push(format!("reason LIKE ${}", bind_count));
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));
        
        let sql = format!(
            "SELECT session_id, camera_id, start_time, end_time, reason, status, COALESCE(keep_session, false) as keep_session FROM {} {} ORDER BY start_time DESC",
            TABLE_RECORDING_SESSIONS, where_clause
        );
        
        debug!(
            "Executing PostgreSQL query for list_recordings_filtered: {}",
            sql
        );

        // Build the query with proper parameter binding
        let mut query = sqlx::query(&sql).bind(camera_id);
        
        if let Some(from_time) = from {
            query = query.bind(from_time);
        }
        if let Some(to_time) = to {
            query = query.bind(to_time);
        }
        if let Some(reason_filter) = reason {
            query = query.bind(reason_filter);
        }
        
        let rows = query.fetch_all(&self.pool).await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(RecordingSession {
                session_id: row.get("session_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
                keep_session: row.get("keep_session"),
            });
        }

        Ok(sessions)
    }

    async fn get_recorded_frames(
        &self,
        session_id: i64,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordedFrame>> {
        let start_time = std::time::Instant::now();
        
        let mut sql = format!("SELECT * FROM {} WHERE session_id = $1", TABLE_RECORDING_MJPEG);
        let mut bind_count = 1;
        
        if from.is_some() {
            bind_count += 1;
            sql.push_str(&format!(" AND timestamp >= ${}", bind_count));
        }
        if to.is_some() {
            bind_count += 1;
            sql.push_str(&format!(" AND timestamp <= ${}", bind_count));
        }
        
        sql.push_str(" ORDER BY timestamp ASC");
        
        debug!(
            "Executing PostgreSQL query for get_recorded_frames: {}",
            sql
        );

        let mut query = sqlx::query(&sql).bind(session_id);
        
        if let Some(from_time) = from {
            query = query.bind(from_time);
        }
        if let Some(to_time) = to {
            query = query.bind(to_time);
        }

        let rows = query.fetch_all(&self.pool).await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut frames = Vec::new();
        for row in rows {
            frames.push(RecordedFrame {
                timestamp: row.get("timestamp"),
                frame_data: row.get("frame_data"),
            });
        }

        Ok(frames)
    }

    async fn delete_old_frames(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        let start_time = std::time::Instant::now();
        let cam_desc = camera_id.unwrap_or("all cameras");

        // Find sessions to clean up (oldest first, not marked as keep, ended before retention cutoff)
        let sessions_to_clean: Vec<(i64, String)> = if let Some(cam_id) = camera_id {
            let query = format!(
                r#"
                SELECT session_id, camera_id FROM {}
                WHERE camera_id = $1
                  AND end_time IS NOT NULL
                  AND end_time < $2
                  AND keep_session = false
                ORDER BY end_time ASC
                "#,
                TABLE_RECORDING_SESSIONS
            );
            sqlx::query_as(&query)
                .bind(cam_id)
                .bind(older_than)
                .fetch_all(&self.pool).await?
        } else {
            let query = format!(
                r#"
                SELECT session_id, camera_id FROM {}
                WHERE end_time IS NOT NULL
                  AND end_time < $1
                  AND keep_session = false
                ORDER BY end_time ASC
                "#,
                TABLE_RECORDING_SESSIONS
            );
            sqlx::query_as(&query)
                .bind(older_than)
                .fetch_all(&self.pool).await?
        };

        if sessions_to_clean.is_empty() {
            info!("No expired sessions to clean up for {}", cam_desc);
            return Ok(0);
        }

        info!(
            "Found {} expired sessions to clean up for {}, processing oldest first...",
            sessions_to_clean.len(),
            cam_desc
        );

        let mut total_deleted: u64 = 0;
        let mut sessions_processed = 0;

        for (session_id, session_camera_id) in &sessions_to_clean {
            let session_start = std::time::Instant::now();

            // Delete all frames for this session
            let delete_query = format!(
                "DELETE FROM {} WHERE session_id = $1",
                TABLE_RECORDING_MJPEG
            );
            let deleted = sqlx::query(&delete_query)
                .bind(session_id)
                .execute(&self.pool).await?
                .rows_affected();

            total_deleted += deleted;
            sessions_processed += 1;

            let session_elapsed = session_start.elapsed();

            if deleted > 0 {
                debug!(
                    "Session {}: deleted {} frames for camera '{}' in {:.2}s (total: {} frames, {}/{} sessions)",
                    session_id,
                    deleted,
                    session_camera_id,
                    session_elapsed.as_secs_f64(),
                    total_deleted,
                    sessions_processed,
                    sessions_to_clean.len()
                );
            }

            // Yield periodically to allow other tasks to run
            if sessions_processed % 10 == 0 {
                tokio::task::yield_now().await;
            }
        }

        let elapsed = start_time.elapsed();

        info!(
            "Completed frame cleanup: {} frames deleted from {} sessions in {:.1}s for {}",
            total_deleted,
            sessions_processed,
            elapsed.as_secs_f64(),
            cam_desc
        );

        Ok(total_deleted as usize)
    }

    async fn delete_unused_sessions(
        &self,
        camera_id: Option<&str>,
    ) -> Result<usize> {
        // Delete sessions that have:
        // 1. No frames in recording_mjpeg table
        // 2. No segments in recording_mp4 table
        // 3. Are not currently active (end_time is not NULL)
        // Uses EXISTS for efficient index lookups instead of NOT IN with full table scans

        let start_time = std::time::Instant::now();

        let result = if let Some(cam_id) = camera_id {
            // Delete unused sessions for a specific camera, but don't delete sessions marked to keep
            let query = format!(
                r#"
                DELETE FROM {sessions}
                WHERE camera_id = $1
                AND end_time IS NOT NULL
                AND keep_session = false
                AND NOT EXISTS (
                    SELECT 1 FROM {mjpeg} WHERE session_id = {sessions}.session_id
                )
                AND NOT EXISTS (
                    SELECT 1 FROM {mp4} WHERE session_id = {sessions}.session_id
                )
                "#,
                sessions = TABLE_RECORDING_SESSIONS,
                mjpeg = TABLE_RECORDING_MJPEG,
                mp4 = TABLE_RECORDING_MP4
            );
            sqlx::query(&query)
                .bind(cam_id)
                .execute(&self.pool)
                .await?
        } else {
            // Delete unused sessions for all cameras, but don't delete sessions marked to keep
            let query = format!(
                r#"
                DELETE FROM {sessions}
                WHERE end_time IS NOT NULL
                AND keep_session = false
                AND NOT EXISTS (
                    SELECT 1 FROM {mjpeg} WHERE session_id = {sessions}.session_id
                )
                AND NOT EXISTS (
                    SELECT 1 FROM {mp4} WHERE session_id = {sessions}.session_id
                )
                "#,
                sessions = TABLE_RECORDING_SESSIONS,
                mjpeg = TABLE_RECORDING_MJPEG,
                mp4 = TABLE_RECORDING_MP4
            );
            sqlx::query(&query)
                .execute(&self.pool)
                .await?
        };

        let deleted_sessions = result.rows_affected();
        let elapsed = start_time.elapsed();

        if deleted_sessions > 0 {
            info!(
                "Deleted {} unused sessions in {:.3}ms{}",
                deleted_sessions,
                elapsed.as_secs_f64() * 1000.0,
                if let Some(cam_id) = camera_id {
                    format!(" for camera '{}'", cam_id)
                } else {
                    String::new()
                }
            );
        } else {
            info!(
                "No unused sessions to delete (query took {:.3}ms)",
                elapsed.as_secs_f64() * 1000.0
            );
        }
        
        Ok(deleted_sessions as usize)
    }
    
    async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        tolerance_seconds: Option<i64>,
    ) -> Result<Option<RecordedFrame>> {
        let tolerance = tolerance_seconds.unwrap_or(0);
        
        if tolerance == 0 {
            // Exact timestamp match using idx_camera_timestamp index
            let query = format!(
                r#"
                SELECT timestamp, frame_data
                FROM {}
                WHERE camera_id = $1 AND timestamp = $2
                LIMIT 1
                "#,
                TABLE_RECORDING_MJPEG
            );
            let row = sqlx::query(&query)
                .bind(camera_id)
                .bind(timestamp)
                .fetch_optional(&self.pool)
                .await?;

            if let Some(row) = row {
                return Ok(Some(RecordedFrame {
                    timestamp: row.get("timestamp"),
                    frame_data: row.get("frame_data"),
                }));
            }
        }

        // Find the closest frame within tolerance using idx_camera_timestamp index
        let tolerance_duration = chrono::Duration::seconds(tolerance);
        let time_before = timestamp - tolerance_duration;
        let time_after = timestamp + tolerance_duration;

        let query = format!(
            r#"
            SELECT timestamp, frame_data,
                   ABS(EXTRACT(EPOCH FROM (timestamp - $1))) as time_diff
            FROM {}
            WHERE camera_id = $2
              AND timestamp >= $3
              AND timestamp <= $4
            ORDER BY time_diff ASC
            LIMIT 1
            "#,
            TABLE_RECORDING_MJPEG
        );
        let row = sqlx::query(&query)
            .bind(timestamp)
            .bind(camera_id)
            .bind(time_before)
            .bind(time_after)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            Ok(Some(RecordedFrame {
                timestamp: row.get("timestamp"),
                frame_data: row.get("frame_data"),
            }))
        } else {
            Ok(None)
        }
    }

    async fn create_frame_stream(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Box<dyn FrameStream>> {
        let stream = PostgreSqlFrameStream::new(&self.pool, camera_id.to_string(), from, to).await?;
        Ok(Box::new(stream))
    }
    
    async fn get_database_size(&self) -> Result<i64> {
        let row = sqlx::query(
            "SELECT pg_database_size(current_database()) AS size_bytes"
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(row.get("size_bytes"))
    }

    async fn add_video_segment(&self, segment: &VideoSegment) -> Result<i64> {
        let query = format!(
            r#"
            INSERT INTO {} (camera_id, session_id, start_time, end_time, file_path, size_bytes, mp4_data)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            TABLE_RECORDING_MP4
        );
        let result = sqlx::query(&query)
        .bind(&segment.camera_id)
        .bind(segment.session_id)
        .bind(segment.start_time)
        .bind(segment.end_time)
        .bind(&segment.file_path)
        .bind(segment.size_bytes)
        .bind(&segment.mp4_data)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
    }

    async fn list_video_segments(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<VideoSegment>> {
        let start_time = std::time::Instant::now();
        
        let query_str = format!(r#"
            SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes,
                   rs.reason as recording_reason, rs.camera_id
            FROM {} vs
            JOIN {} rs ON vs.session_id = rs.session_id
            WHERE rs.camera_id = $1 AND vs.start_time < $2 AND vs.end_time > $3
            ORDER BY vs.start_time ASC
            "#, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS);
        
        debug!(
            "Executing PostgreSQL query for list_video_segments: {}",
            query_str
        );
        
        let rows = sqlx::query(&query_str)
        .bind(camera_id)
        .bind(to)
        .bind(from)
        .fetch_all(&self.pool)
        .await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut segments = Vec::new();
        for row in rows {
            segments.push(VideoSegment {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                file_path: row.get("file_path"),
                size_bytes: row.get("size_bytes"),
                mp4_data: None,  // Not loaded for listing performance
                recording_reason: row.get("recording_reason"),
                camera_id: row.get("camera_id"),
            });
        }

        Ok(segments)
    }

    async fn list_video_segments_filtered(
        &self,
        camera_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        reason: Option<&str>,
        limit: i64,
        sort_order: &str,
    ) -> Result<Vec<VideoSegment>> {
        let start_time = std::time::Instant::now();
        
        let mut conditions = vec!["rs.camera_id = $1".to_string()];
        let mut bind_count = 1;

        if from.is_some() {
            bind_count += 1;
            conditions.push(format!("vs.end_time > ${}", bind_count));
        }

        if to.is_some() {
            bind_count += 1;
            conditions.push(format!("vs.start_time < ${}", bind_count));
        }

        if reason.is_some() {
            bind_count += 1;
            conditions.push(format!("rs.reason LIKE ${}", bind_count));
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));
        
        let order_direction = match sort_order {
            "oldest" => "ASC",
            _ => "DESC", // default to newest first
        };

        bind_count += 1;
        let query_str = format!(r#"
            SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes,
                   rs.reason as recording_reason, rs.camera_id
            FROM {} vs
            JOIN {} rs ON vs.session_id = rs.session_id
            {}
            ORDER BY vs.start_time {}
            LIMIT ${}
            "#, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS, where_clause, order_direction, bind_count);
        
        debug!(
            "Executing PostgreSQL query for list_video_segments_filtered: {}",
            query_str
        );
        
        let mut query = sqlx::query(&query_str);
        
        // Bind parameters in order
        query = query.bind(camera_id);
        if let Some(from_time) = from {
            query = query.bind(from_time);
        }
        if let Some(to_time) = to {
            query = query.bind(to_time);
        }
        if let Some(reason_filter) = reason {
            query = query.bind(format!("%{}%", reason_filter));
        }
        query = query.bind(limit);
        
        let rows = query.fetch_all(&self.pool).await?;
        
        let elapsed = start_time.elapsed();
        let row_count = rows.len();
        
        debug!(
            "Query completed in {:.3}ms, returned {} rows",
            elapsed.as_secs_f64() * 1000.0,
            row_count
        );

        let mut segments = Vec::new();
        for row in rows {
            segments.push(VideoSegment {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                file_path: row.get("file_path"),
                size_bytes: row.get("size_bytes"),
                mp4_data: None,  // Not loaded for listing performance
                recording_reason: row.get("recording_reason"),
                camera_id: row.get("camera_id"),
            });
        }

        Ok(segments)
    }

    async fn delete_old_video_segments(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        let start_time = std::time::Instant::now();
        
        // First, select the file paths of the segments to be deleted, excluding sessions marked to keep
        let segments_to_delete = if let Some(cam_id) = camera_id {
            // Delete segments for a specific camera, but not for sessions marked to keep
            let query = format!(
                r#"
                SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes, vs.mp4_data
                FROM {} vs
                JOIN {} rs ON vs.session_id = rs.session_id
                WHERE rs.camera_id = $1 AND vs.end_time < $2 AND rs.keep_session = false
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query_as::<_, VideoSegment>(&query)
                .bind(cam_id)
                .bind(older_than)
                .fetch_all(&self.pool)
                .await?
        } else {
            // Delete segments for all cameras, but not for sessions marked to keep
            let query = format!(
                r#"
                SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes, vs.mp4_data
                FROM {} vs
                JOIN {} rs ON vs.session_id = rs.session_id
                WHERE vs.end_time < $1 AND rs.keep_session = false
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query_as::<_, VideoSegment>(&query)
                .bind(older_than)
                .fetch_all(&self.pool)
                .await?
        };

        // Delete the files from the filesystem (only if they have file_path set)
        for segment in &segments_to_delete {
            if let Some(file_path) = &segment.file_path {
                if let Err(e) = tokio::fs::remove_file(file_path).await {
                    tracing::error!("Failed to delete video segment file {}: {}", file_path, e);
                }
            }
            // No action needed for database-stored segments - they'll be deleted with the record
        }

        // Then, delete the records from the database, but only for sessions not marked to keep
        let delete_result = if let Some(cam_id) = camera_id {
            let delete_query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT vs.session_id 
                    FROM {} vs
                    JOIN {} rs ON vs.session_id = rs.session_id
                    WHERE rs.camera_id = $1 AND vs.end_time < $2 AND rs.keep_session = false
                )
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&delete_query)
                .bind(cam_id)
                .bind(older_than)
                .execute(&self.pool)
                .await?
        } else {
            let delete_query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT session_id FROM {} WHERE keep_session = false
                ) AND end_time < $1
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&delete_query)
                .bind(older_than)
                .execute(&self.pool)
                .await?
        };

        let deleted_count = delete_result.rows_affected() as usize;
        let elapsed = start_time.elapsed();
        
        if deleted_count > 0 {
            info!(
                "Deleted {} video segments in {:.3}ms{}",
                deleted_count,
                elapsed.as_secs_f64() * 1000.0,
                if let Some(cam_id) = camera_id {
                    format!(" for camera '{}'", cam_id)
                } else {
                    String::new()
                }
            );
        } else {
            info!(
                "No video segments to delete (query took {:.3}ms)",
                elapsed.as_secs_f64() * 1000.0
            );
        }

        Ok(deleted_count)
    }

    async fn cleanup_database(
        &self,
        config: &crate::config::RecordingConfig,
        camera_configs: &std::collections::HashMap<String, crate::config::CameraConfig>,
    ) -> Result<()> {
        // For PostgreSQL, we need to determine which camera this database serves
        // This is more complex for shared databases
        let camera_id = if self.is_shared_database {
            // For shared databases, we can't determine a specific camera
            None
        } else {
            // For per-camera databases, try to get camera_id from sessions
            let mut connection = self.pool.acquire().await?;
            let query = format!("SELECT DISTINCT camera_id FROM {} LIMIT 1", TABLE_RECORDING_SESSIONS);
            if let Ok(row) = sqlx::query(&query).fetch_optional(connection.as_mut()).await {
                row.and_then(|r| r.try_get::<String, _>("camera_id").ok())
            } else {
                None
            }
        };

        // Get camera-specific config or use global config
        let (frame_retention, video_retention, mp4_storage_type, hls_enabled, hls_retention) = if let Some(cam_id) = &camera_id {
            if let Some(camera_config) = camera_configs.get(cam_id) {
                // Use camera-specific retention settings if available, otherwise fall back to global
                let frame_retention = camera_config.get_frame_storage_retention()
                    .unwrap_or(&config.frame_storage_retention);
                let video_retention = camera_config.get_mp4_storage_retention()
                    .unwrap_or(&config.mp4_storage_retention);
                let video_type = camera_config.get_mp4_storage_type()
                    .unwrap_or(&config.mp4_storage_type);
                let hls_enabled = camera_config.get_hls_storage_enabled()
                    .unwrap_or(config.hls_storage_enabled);
                let hls_retention = camera_config.get_hls_storage_retention()
                    .unwrap_or(&config.hls_storage_retention);
                (frame_retention.clone(), video_retention.clone(), video_type.clone(), hls_enabled, hls_retention.clone())
            } else {
                // Camera not found in configs, use global settings
                (config.frame_storage_retention.clone(), 
                 config.mp4_storage_retention.clone(),
                 config.mp4_storage_type.clone(),
                 config.hls_storage_enabled,
                 config.hls_storage_retention.clone())
            }
        } else {
            // No camera_id found, use global settings
            (config.frame_storage_retention.clone(), 
             config.mp4_storage_retention.clone(),
             config.mp4_storage_type.clone(),
             config.hls_storage_enabled,
             config.hls_storage_retention.clone())
        };

        // Cleanup frames with camera-specific or global retention
        if config.frame_storage_enabled {
            // Check if retention is explicitly disabled with "0"
            if frame_retention != "0" {
                if let Ok(duration) = humantime::parse_duration(&frame_retention) {
                    if duration.as_secs() > 0 {
                        let older_than = Utc::now() - chrono::Duration::from_std(duration).unwrap();
                        info!("Starting frame cleanup for database '{}' (retention: {})", self.database_name, frame_retention);
                        if let Err(e) = self.delete_old_frames(camera_id.as_deref(), older_than).await {
                            tracing::error!("Error deleting old frames: {}", e);
                        }
                    }
                }
            } else {
                tracing::debug!("Frame retention disabled (0) for database '{}', camera {:?}", self.database_name, camera_id);
            }
        }

        // Cleanup video segments with camera-specific or global retention
        if mp4_storage_type != crate::config::Mp4StorageType::Disabled {
            // Check if retention is explicitly disabled with "0"
            if video_retention != "0" {
                if let Ok(duration) = humantime::parse_duration(&video_retention) {
                    if duration.as_secs() > 0 {
                        let older_than = Utc::now() - chrono::Duration::from_std(duration).unwrap();
                        info!("Starting video segment cleanup for database '{}' (retention: {})", self.database_name, video_retention);
                        if let Err(e) = self.delete_old_video_segments(camera_id.as_deref(), older_than).await {
                            tracing::error!("Error deleting old video segments: {}", e);
                        }
                    }
                }
            } else {
                tracing::debug!("MP4 retention disabled (0) for database '{}', camera {:?}", self.database_name, camera_id);
            }
        }

        // Cleanup HLS segments with camera-specific or global retention
        if hls_enabled {
            // Check if retention is explicitly disabled with "0"
            if hls_retention != "0" {
                if let Ok(duration) = humantime::parse_duration(&hls_retention) {
                    if duration.as_secs() > 0 {
                        info!("Starting HLS segment cleanup (retention: {})", hls_retention);
                        match self.delete_old_recording_hls_segments(&hls_retention, camera_id.as_deref()).await {
                            Ok(deleted_count) => {
                                info!("Deleted {} old HLS segments", deleted_count);
                            }
                            Err(e) => {
                                tracing::error!("Error deleting old HLS segments: {}", e);
                            }
                        }
                    }
                }
            } else {
                tracing::debug!("HLS retention disabled (0) for database '{}', camera {:?}", self.database_name, camera_id);
            }
        }

        // Finally, cleanup unused sessions (sessions with no frames or videos)
        // This should be done after deleting frames and videos to catch newly orphaned sessions
        info!("Starting unused session cleanup");
        if let Err(e) = self.delete_unused_sessions(camera_id.as_deref()).await {
            tracing::error!("Error deleting unused sessions: {}", e);
        }

        // Vacuum tables to reclaim disk space after cleanup
        if let Err(e) = self.vacuum_tables().await {
            tracing::error!("Error vacuuming database: {}", e);
        }

        Ok(())
    }


    async fn get_video_segment_by_time(
        &self,
        camera_id: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<VideoSegment>> {
        let query = format!(r#"
            SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes, vs.mp4_data, rs.camera_id
            FROM {} vs
            JOIN {} rs ON vs.session_id = rs.session_id
            WHERE rs.camera_id = $1 AND vs.start_time = $2
            "#, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS);
        
        debug!(
            "Executing PostgreSQL query for get_video_segment_by_time:\n{}\nParameters: camera_id='{}', timestamp='{}'",
            query, camera_id, timestamp
        );
        
        let start_time = std::time::Instant::now();
        let row = sqlx::query(&query)
            .bind(camera_id)
            .bind(timestamp)
            .fetch_optional(&self.pool)
            .await?;
        
        let elapsed = start_time.elapsed();
        debug!(
            "PostgreSQL query completed in {:.3}ms, found: {}",
            elapsed.as_secs_f64() * 1000.0,
            row.is_some()
        );
            
        if let Some(row) = row {
            Ok(Some(VideoSegment {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                file_path: row.get("file_path"),
                size_bytes: row.get("size_bytes"),
                mp4_data: row.get("mp4_data"),
                recording_reason: None, // Not needed for segment streaming
                camera_id: row.get("camera_id"),
            }))
        } else {
            Ok(None)
        }
    }

    // HLS-specific methods implementation for PostgreSQL
    
    /// Store an HLS playlist in the database
    async fn store_hls_playlist(&self, playlist: &HlsPlaylist) -> Result<()> {
        let query = format!(
            r#"
            INSERT INTO {} (playlist_id, camera_id, start_time, end_time, segment_duration, playlist_content, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (playlist_id) DO UPDATE SET
                camera_id = EXCLUDED.camera_id,
                start_time = EXCLUDED.start_time,
                end_time = EXCLUDED.end_time,
                segment_duration = EXCLUDED.segment_duration,
                playlist_content = EXCLUDED.playlist_content,
                created_at = EXCLUDED.created_at,
                expires_at = EXCLUDED.expires_at
            "#,
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&query)
            .bind(&playlist.playlist_id)
            .bind(&playlist.camera_id)
            .bind(playlist.start_time)
            .bind(playlist.end_time)
            .bind(playlist.segment_duration)
            .bind(&playlist.playlist_content)
            .bind(playlist.created_at)
            .bind(playlist.expires_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Store HLS playlist and segments in a transaction
    async fn store_hls_playlist_with_segments(&self, playlist: &HlsPlaylist, segments: &[HlsSegment]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // First, store the playlist
        let playlist_query = format!(
            r#"
            INSERT INTO {} (playlist_id, camera_id, start_time, end_time, segment_duration, playlist_content, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (playlist_id) DO UPDATE SET
                camera_id = EXCLUDED.camera_id,
                start_time = EXCLUDED.start_time,
                end_time = EXCLUDED.end_time,
                segment_duration = EXCLUDED.segment_duration,
                playlist_content = EXCLUDED.playlist_content,
                created_at = EXCLUDED.created_at,
                expires_at = EXCLUDED.expires_at
            "#,
            TABLE_HLS_PLAYLISTS
        );
        sqlx::query(&playlist_query)
            .bind(&playlist.playlist_id)
            .bind(&playlist.camera_id)
            .bind(playlist.start_time)
            .bind(playlist.end_time)
            .bind(playlist.segment_duration)
            .bind(&playlist.playlist_content)
            .bind(playlist.created_at)
            .bind(playlist.expires_at)
            .execute(&mut *tx)
            .await?;

        // Then, store all segments
        let segment_query = format!(
            r#"
            INSERT INTO {} (playlist_id, segment_name, segment_index, segment_data, size_bytes, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (playlist_id, segment_name) DO UPDATE SET
                segment_index = EXCLUDED.segment_index,
                segment_data = EXCLUDED.segment_data,
                size_bytes = EXCLUDED.size_bytes,
                created_at = EXCLUDED.created_at
            "#,
            TABLE_HLS_SEGMENTS
        );

        for segment in segments {
            sqlx::query(&segment_query)
                .bind(&segment.playlist_id)
                .bind(&segment.segment_name)
                .bind(segment.segment_index)
                .bind(&segment.segment_data)
                .bind(segment.size_bytes)
                .bind(segment.created_at)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Store an HLS segment in the database
    async fn store_hls_segment(&self, segment: &HlsSegment) -> Result<()> {
        let query = format!(
            r#"
            INSERT INTO {} (playlist_id, segment_name, segment_index, segment_data, size_bytes, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (playlist_id, segment_name) DO UPDATE SET
                segment_index = EXCLUDED.segment_index,
                segment_data = EXCLUDED.segment_data,
                size_bytes = EXCLUDED.size_bytes,
                created_at = EXCLUDED.created_at
            "#,
            TABLE_HLS_SEGMENTS
        );
        sqlx::query(&query)
            .bind(&segment.playlist_id)
            .bind(&segment.segment_name)
            .bind(segment.segment_index)
            .bind(&segment.segment_data)
            .bind(segment.size_bytes)
            .bind(segment.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get an HLS playlist by ID if it hasn't expired
    async fn get_hls_playlist(&self, playlist_id: &str) -> Result<Option<HlsPlaylist>> {
        let query = format!(
            r#"
            SELECT playlist_id, camera_id, start_time, end_time, segment_duration, playlist_content, created_at, expires_at
            FROM {} 
            WHERE playlist_id = $1 AND expires_at > CURRENT_TIMESTAMP
            "#,
            TABLE_HLS_PLAYLISTS
        );
        let row = sqlx::query(&query)
            .bind(playlist_id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            Ok(Some(HlsPlaylist {
                playlist_id: row.get("playlist_id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                segment_duration: row.get("segment_duration"),
                playlist_content: row.get("playlist_content"),
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
            }))
        } else {
            Ok(None)
        }
    }

    /// Get an HLS segment by playlist ID and segment name
    async fn get_hls_segment(&self, playlist_id: &str, segment_name: &str) -> Result<Option<HlsSegment>> {
        let query = format!(
            r#"
            SELECT playlist_id, segment_name, segment_index, segment_data, size_bytes, created_at
            FROM {} 
            WHERE playlist_id = $1 AND segment_name = $2
            "#,
            TABLE_HLS_SEGMENTS
        );
        let row = sqlx::query(&query)
            .bind(playlist_id)
            .bind(segment_name)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            Ok(Some(HlsSegment {
                playlist_id: row.get("playlist_id"),
                segment_name: row.get("segment_name"),
                segment_index: row.get("segment_index"),
                segment_data: row.get("segment_data"),
                size_bytes: row.get("size_bytes"),
                created_at: row.get("created_at"),
            }))
        } else {
            Ok(None)
        }
    }

    /// Clean up expired HLS playlists and their segments
    async fn cleanup_expired_hls(&self) -> Result<usize> {
        let mut tx = self.pool.begin().await?;

        // Delete expired segments first (due to foreign key constraint)
        let delete_segments_query = format!(
            r#"
            DELETE FROM {} 
            WHERE playlist_id IN (
                SELECT playlist_id FROM {} 
                WHERE expires_at <= CURRENT_TIMESTAMP
            )
            "#,
            TABLE_HLS_SEGMENTS, TABLE_HLS_PLAYLISTS
        );
        let segments_result = sqlx::query(&delete_segments_query)
            .execute(&mut *tx)
            .await?;

        // Delete expired playlists
        let delete_playlists_query = format!(
            "DELETE FROM {} WHERE expires_at <= CURRENT_TIMESTAMP",
            TABLE_HLS_PLAYLISTS
        );
        let playlists_result = sqlx::query(&delete_playlists_query)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        info!(
            "Cleaned up expired HLS data: {} playlists, {} segments",
            playlists_result.rows_affected(),
            segments_result.rows_affected()
        );

        Ok(playlists_result.rows_affected() as usize)
    }

    async fn add_recording_hls_segment(&self, segment: &RecordingHlsSegment) -> Result<i64> {
        let query = format!(
            r#"
            INSERT INTO {} (camera_id, session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING session_id
            "#,
            TABLE_RECORDING_HLS
        );

        let row = sqlx::query(&query)
            .bind(&segment.camera_id)
            .bind(segment.session_id)
            .bind(segment.segment_index)
            .bind(segment.start_time)
            .bind(segment.end_time)
            .bind(segment.duration_seconds)
            .bind(&segment.segment_data)
            .bind(segment.size_bytes)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.get("session_id"))
    }

    async fn list_recording_hls_segments(
        &self,
        session_id: i64,
        from_time: Option<DateTime<Utc>>,
        to_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordingHlsSegment>> {
        match (from_time, to_time) {
            (None, None) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = $1 ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
            (Some(from), None) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = $1 AND start_time >= $2 ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .bind(from)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
            (None, Some(to)) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = $1 AND end_time <= $2 ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .bind(to)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
            (Some(from), Some(to)) => {
                let query = format!(
                    "SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at FROM {} WHERE session_id = $1 AND start_time >= $2 AND end_time <= $3 ORDER BY segment_index ASC",
                    TABLE_RECORDING_HLS
                );
                let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
                    .bind(session_id)
                    .bind(from)
                    .bind(to)
                    .fetch_all(&self.pool)
                    .await?;
                Ok(segments)
            }
        }
    }

    async fn get_recording_hls_segments_for_timerange(
        &self,
        camera_id: &str,
        from_time: DateTime<Utc>,
        to_time: DateTime<Utc>,
    ) -> Result<Vec<RecordingHlsSegment>> {
        // Query for segments that overlap with the requested time range
        // A segment overlaps if its start is before the range end AND its end is after the range start
        let query = format!(
            r#"
            SELECT rh.session_id, rh.segment_index, rh.start_time, rh.end_time, rh.duration_seconds, 
                   rh.segment_data, rh.size_bytes, rh.created_at
            FROM {} rh
            JOIN {} rs ON rh.session_id = rs.session_id
            WHERE rs.camera_id = $1 
            AND rh.start_time <= $2  -- segment starts before or at range end
            AND rh.end_time >= $3     -- segment ends after or at range start
            ORDER BY rh.start_time ASC, rh.segment_index ASC
            "#,
            TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
        );
        
        let segments = sqlx::query_as::<_, RecordingHlsSegment>(&query)
            .bind(camera_id)
            .bind(to_time)
            .bind(from_time)
            .fetch_all(&self.pool)
            .await?;
            
        Ok(segments)
    }

    async fn delete_old_recording_hls_segments(
        &self,
        retention_duration: &str,
        camera_id: Option<&str>,
    ) -> Result<usize> {
        let duration = humantime::parse_duration(retention_duration)
            .map_err(|e| crate::errors::StreamError::config(&format!("Invalid retention duration '{}': {}", retention_duration, e)))?;
        
        let cutoff_time = Utc::now() - chrono::Duration::from_std(duration)
            .map_err(|e| crate::errors::StreamError::config(&format!("Invalid duration: {}", e)))?;
        
        let result = if let Some(cam_id) = camera_id {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT rs.session_id FROM {} rs 
                    WHERE rs.camera_id = $1 AND rs.start_time < $2 AND rs.keep_session = false
                )
                "#,
                TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
                .bind(cam_id)
                .bind(cutoff_time)
                .execute(&self.pool)
                .await?
        } else {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT session_id FROM {} WHERE keep_session = false
                ) AND created_at < $1
                "#,
                TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
                .bind(cutoff_time)
                .execute(&self.pool)
                .await?
        };
        
        Ok(result.rows_affected() as usize)
    }

    async fn get_recording_hls_segment_by_session_and_index(
        &self,
        session_id: i64,
        segment_index: i32,
    ) -> Result<Option<RecordingHlsSegment>> {
        let query = format!(
            r#"
            SELECT session_id, segment_index, start_time, end_time, duration_seconds, segment_data, size_bytes, created_at
            FROM {} 
            WHERE session_id = $1 AND segment_index = $2
            "#,
            TABLE_RECORDING_HLS
        );
        
        let segment = sqlx::query_as::<_, RecordingHlsSegment>(&query)
            .bind(session_id)
            .bind(segment_index)
            .fetch_optional(&self.pool)
            .await?;
        
        Ok(segment)
    }

    async fn get_last_hls_segment_index_for_session(
        &self,
        session_id: i64,
    ) -> Result<Option<i32>> {
        let query = format!(
            "SELECT MAX(segment_index) as max_index FROM {} WHERE session_id = $1",
            TABLE_RECORDING_HLS
        );
        
        let result: Option<(Option<i32>,)> = sqlx::query_as(&query)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        
        Ok(result.and_then(|(max_index,)| max_index))
    }

    async fn set_session_keep_flag(
        &self,
        session_id: i64,
        keep_session: bool,
    ) -> Result<()> {
        let query = format!(
            "UPDATE {} SET keep_session = $1 WHERE session_id = $2",
            TABLE_RECORDING_SESSIONS
        );

        sqlx::query(&query)
            .bind(keep_session)
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn delete_recording_session(&self, session_id: i64) -> Result<DeletedRecordingStats> {
        // First check if session is stopped
        let session_query = format!(
            "SELECT status FROM {} WHERE session_id = $1",
            TABLE_RECORDING_SESSIONS
        );
        let status: String = sqlx::query_scalar(&session_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        if status == "active" {
            return Err(crate::errors::StreamError::database(
                "Cannot delete active recording session. Stop it first.".to_string()
            ));
        }

        // Count items before deletion
        let frames_count_query = format!("SELECT COUNT(*) FROM {} WHERE session_id = $1", TABLE_RECORDING_MJPEG);
        let frames_deleted: i64 = sqlx::query_scalar(&frames_count_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        let mp4_count_query = format!("SELECT COUNT(*) FROM {} WHERE session_id = $1", TABLE_RECORDING_MP4);
        let mp4_segments_deleted: i64 = sqlx::query_scalar(&mp4_count_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        let hls_count_query = format!("SELECT COUNT(*) FROM {} WHERE session_id = $1", TABLE_RECORDING_HLS);
        let hls_segments_deleted: i64 = sqlx::query_scalar(&hls_count_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        // Get MP4 file paths for filesystem deletion
        let mp4_files_query = format!(
            "SELECT file_path FROM {} WHERE session_id = $1 AND file_path IS NOT NULL",
            TABLE_RECORDING_MP4
        );
        let file_paths: Vec<String> = sqlx::query_scalar(&mp4_files_query)
            .bind(session_id)
            .fetch_all(&self.pool)
            .await?;

        // Delete from database in correct order (due to foreign keys)
        let delete_frames = format!("DELETE FROM {} WHERE session_id = $1", TABLE_RECORDING_MJPEG);
        sqlx::query(&delete_frames).bind(session_id).execute(&self.pool).await?;

        let delete_mp4 = format!("DELETE FROM {} WHERE session_id = $1", TABLE_RECORDING_MP4);
        sqlx::query(&delete_mp4).bind(session_id).execute(&self.pool).await?;

        let delete_hls = format!("DELETE FROM {} WHERE session_id = $1", TABLE_RECORDING_HLS);
        sqlx::query(&delete_hls).bind(session_id).execute(&self.pool).await?;

        let delete_session = format!("DELETE FROM {} WHERE session_id = $1", TABLE_RECORDING_SESSIONS);
        sqlx::query(&delete_session).bind(session_id).execute(&self.pool).await?;

        // Delete MP4 files from filesystem
        for file_path in file_paths {
            if let Err(e) = tokio::fs::remove_file(&file_path).await {
                tracing::warn!("Failed to delete MP4 file {}: {}", file_path, e);
            }
        }

        info!("Deleted recording session {} (frames: {}, mp4: {}, hls: {})",
              session_id, frames_deleted, mp4_segments_deleted, hls_segments_deleted);

        Ok(DeletedRecordingStats {
            session_id,
            frames_deleted: frames_deleted as u64,
            mp4_segments_deleted: mp4_segments_deleted as u64,
            hls_segments_deleted: hls_segments_deleted as u64,
        })
    }

    async fn delete_mp4_segment_by_filename(&self, camera_id: &str, filename: &str) -> Result<i64> {
        // Get the segment info before deletion
        let query = format!(
            "SELECT s.file_path, s.size_bytes, s.session_id
             FROM {} s
             JOIN {} r ON s.session_id = r.session_id
             WHERE r.camera_id = $1 AND s.file_path LIKE $2",
            TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
        );

        let pattern = format!("%/{}", filename);
        let row: Option<(Option<String>, i64, i64)> = sqlx::query_as(&query)
            .bind(camera_id)
            .bind(&pattern)
            .fetch_optional(&self.pool)
            .await?;

        if let Some((file_path, size_bytes, session_id)) = row {
            // Check if session is stopped
            let status_query = format!("SELECT status FROM {} WHERE session_id = $1", TABLE_RECORDING_SESSIONS);
            let status: String = sqlx::query_scalar(&status_query)
                .bind(session_id)
                .fetch_one(&self.pool)
                .await?;

            if status == "active" {
                return Err(crate::errors::StreamError::database(
                    "Cannot delete MP4 from active recording session".to_string()
                ));
            }

            // Delete from database
            let delete_query = format!(
                "DELETE FROM {} WHERE session_id = $1 AND file_path LIKE $2",
                TABLE_RECORDING_MP4
            );
            sqlx::query(&delete_query)
                .bind(session_id)
                .bind(&pattern)
                .execute(&self.pool)
                .await?;

            // Delete file from filesystem if it exists
            if let Some(path) = file_path {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    tracing::warn!("Failed to delete MP4 file {}: {}", path, e);
                }
            }

            Ok(size_bytes)
        } else {
            Err(crate::errors::StreamError::database(format!("MP4 segment {} not found", filename)))
        }
    }

    async fn delete_mp4_segments_bulk(&self, camera_id: &str, filenames: Vec<String>) -> Result<BulkDeleteResult> {
        let mut deleted_count = 0;
        let mut failed = Vec::new();
        let mut total_size_bytes = 0i64;

        for filename in filenames {
            match self.delete_mp4_segment_by_filename(camera_id, &filename).await {
                Ok(size) => {
                    deleted_count += 1;
                    total_size_bytes += size;
                }
                Err(e) => {
                    tracing::error!("Failed to delete {}: {}", filename, e);
                    failed.push(filename);
                }
            }
        }

        Ok(BulkDeleteResult {
            deleted_count,
            failed,
            total_size_bytes,
        })
    }

    async fn delete_hls_segments_by_session(&self, session_id: i64) -> Result<u64> {
        // Check if session is stopped
        let status_query = format!("SELECT status FROM {} WHERE session_id = $1", TABLE_RECORDING_SESSIONS);
        let status: String = sqlx::query_scalar(&status_query)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;

        if status == "active" {
            return Err(crate::errors::StreamError::database(
                "Cannot delete HLS from active recording session".to_string()
            ));
        }

        let delete_query = format!("DELETE FROM {} WHERE session_id = $1", TABLE_RECORDING_HLS);
        let result = sqlx::query(&delete_query)
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn delete_hls_segments_by_timerange(&self, camera_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<u64> {
        let delete_query = format!(
            "DELETE FROM {}
             WHERE session_id IN (
                 SELECT session_id FROM {} WHERE camera_id = $1 AND status != 'active'
             )
             AND start_time >= $2 AND end_time <= $3",
            TABLE_RECORDING_HLS, TABLE_RECORDING_SESSIONS
        );

        let result = sqlx::query(&delete_query)
            .bind(camera_id)
            .bind(from)
            .bind(to)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn vacuum_tables(&self) -> Result<()> {
        let start_time = std::time::Instant::now();
        info!("Starting PostgreSQL VACUUM FULL on recording tables for database '{}'...", self.database_name);

        // VACUUM FULL rewrites tables and reclaims disk space back to the OS
        // This is more thorough than regular VACUUM but requires exclusive lock
        // We vacuum each table that can have significant deletions

        let tables = [
            TABLE_RECORDING_MJPEG,
            TABLE_RECORDING_HLS,
            TABLE_RECORDING_MP4,
            TABLE_RECORDING_SESSIONS,
        ];

        for table in &tables {
            let table_start = std::time::Instant::now();
            let vacuum_query = format!("VACUUM FULL {}", table);

            if let Err(e) = sqlx::query(&vacuum_query)
                .execute(&self.pool)
                .await
            {
                // Log error but continue with other tables
                tracing::warn!("VACUUM FULL {} failed: {}", table, e);
            } else {
                let table_elapsed = table_start.elapsed();
                debug!("VACUUM FULL {} completed in {:.1}s", table, table_elapsed.as_secs_f64());
            }
        }

        let elapsed = start_time.elapsed();
        info!(
            "PostgreSQL VACUUM FULL completed for database '{}' in {:.1}s",
            self.database_name,
            elapsed.as_secs_f64()
        );

        Ok(())
    }

    async fn record_throughput_stats(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
        bytes_per_second: i64,
        frame_count: i32,
        ffmpeg_fps: f32,
        connection_count: i32,
    ) -> Result<()> {
        let query = format!(
            r#"
            INSERT INTO {} (camera_id, timestamp, bytes_per_second, frame_count, ffmpeg_fps, connection_count)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (camera_id, timestamp) DO UPDATE SET
                bytes_per_second = EXCLUDED.bytes_per_second,
                frame_count = EXCLUDED.frame_count,
                ffmpeg_fps = EXCLUDED.ffmpeg_fps,
                connection_count = EXCLUDED.connection_count
            "#,
            TABLE_THROUGHPUT_STATS
        );
        sqlx::query(&query)
            .bind(camera_id)
            .bind(timestamp)
            .bind(bytes_per_second)
            .bind(frame_count)
            .bind(ffmpeg_fps)
            .bind(connection_count)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn get_throughput_stats(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<ThroughputStats>> {
        let query = format!(
            r#"
            SELECT camera_id, timestamp, bytes_per_second, frame_count, ffmpeg_fps, connection_count
            FROM {} 
            WHERE camera_id = $1 AND timestamp >= $2 AND timestamp <= $3
            ORDER BY timestamp ASC
            "#,
            TABLE_THROUGHPUT_STATS
        );
        let rows = sqlx::query(&query)
            .bind(camera_id)
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(ThroughputStats {
                camera_id: row.get("camera_id"),
                timestamp: row.get("timestamp"),
                bytes_per_second: row.get("bytes_per_second"),
                frame_count: row.get("frame_count"),
                ffmpeg_fps: row.get("ffmpeg_fps"),
                connection_count: row.get("connection_count"),
            });
        }

        Ok(stats)
    }

    async fn cleanup_old_throughput_stats(&self, older_than: DateTime<Utc>) -> Result<u64> {
        let query = format!(
            "DELETE FROM {} WHERE timestamp < $1",
            TABLE_THROUGHPUT_STATS
        );
        let result = sqlx::query(&query)
            .bind(older_than)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn get_mp4_segments_in_range(&self, camera_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<crate::export_jobs::Mp4SegmentInfo>> {
        let query = format!(
            r#"
            SELECT session_id, start_time, end_time, file_path
            FROM {}
            WHERE camera_id = $1 AND end_time >= $2 AND start_time <= $3
            ORDER BY start_time ASC
            "#,
            TABLE_RECORDING_MP4
        );

        let rows = sqlx::query(&query)
            .bind(camera_id)
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?;

        let mut segments = Vec::new();
        for row in rows {
            segments.push(crate::export_jobs::Mp4SegmentInfo {
                session_id: row.get("session_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                storage_path: row.get("file_path"),
            });
        }

        Ok(segments)
    }

    async fn extract_mp4_segment_to_file(&self, segment_id: i64, output_path: &str) -> Result<()> {
        let query = format!(
            "SELECT mp4_data FROM {} WHERE session_id = $1",
            TABLE_RECORDING_MP4
        );

        let row = sqlx::query(&query)
            .bind(segment_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StreamError::not_found(format!("MP4 segment {} not found", segment_id)))?;

        let data: Vec<u8> = row.get("mp4_data");

        std::fs::write(output_path, data)
            .map_err(|e| StreamError::internal(format!("Failed to write MP4 segment to file: {}", e)))?;

        Ok(())
    }
}

// Database factory functions
pub async fn create_database_provider(
    config: &crate::config::RecordingConfig,
    camera_id: Option<&str>,
) -> Result<Arc<dyn DatabaseProvider>> {
    match config.database_type {
        crate::config::DatabaseType::SQLite => {
            // Use existing SQLite logic
            let db_path = if let Some(cam_id) = camera_id {
                format!("{}/{}.db", config.database_path, cam_id)
            } else {
                // Use default path for SQLite when no camera_id is provided
                format!("{}/recordings.db", config.database_path)
            };
            
            let database = SqliteDatabase::new(&db_path).await?;
            Ok(Arc::new(database))
        }
        crate::config::DatabaseType::PostgreSQL => {
            let database_url = config
                .database_url
                .as_ref()
                .ok_or_else(|| crate::errors::StreamError::config("database_url is required for PostgreSQL"))?;
            
            let database = PostgreSqlDatabase::new(database_url, camera_id).await?;
            Ok(Arc::new(database))
        }
    }
}