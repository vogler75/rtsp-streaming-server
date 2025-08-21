use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{SqlitePool, Row, FromRow};
use tracing::error;
use crate::errors::Result;

// Table name constants for easy configuration
const TABLE_RECORDING_SESSIONS: &str = "recording_sessions";
const TABLE_RECORDING_MJPEG: &str = "recording_mjpeg";  // formerly recorded_frames
const TABLE_RECORDING_MP4: &str = "recording_mp4";      // formerly video_segments
const TABLE_HLS_PLAYLISTS: &str = "hls_playlists";
const TABLE_HLS_SEGMENTS: &str = "hls_segments";

#[derive(Debug, Clone)]
pub struct RecordingSession {
    pub id: i64,
    pub camera_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub status: RecordingStatus,
}

#[derive(Debug, Clone)]
pub struct RecordedFrame {
    pub timestamp: DateTime<Utc>,
    pub frame_data: Vec<u8>,  // Store actual frame data
}

#[derive(Debug, Clone, FromRow)]
pub struct VideoSegment {
    pub session_id: i64,  // Part of composite primary key with start_time
    pub start_time: DateTime<Utc>,  // Part of composite primary key with session_id
    pub end_time: DateTime<Utc>,
    pub file_path: Option<String>,  // Optional for database storage
    pub size_bytes: i64,
    pub mp4_data: Option<Vec<u8>>,  // Optional blob data for database storage
    #[sqlx(default)]  // This field might not exist when not joining with recording_sessions
    pub recording_reason: Option<String>,  // Recording reason from recording_sessions
    #[sqlx(default)]  // This field comes from the JOIN with recording_sessions
    #[allow(dead_code)]  // Available from JOIN but not always used
    pub camera_id: Option<String>,  // Camera ID from recording_sessions when needed
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

#[async_trait]
pub trait DatabaseProvider: Send + Sync {
    async fn initialize(&self) -> Result<()>;
    
    async fn create_recording_session(
        &self,
        camera_id: &str,
        reason: Option<&str>,
    ) -> Result<i64>;
    
    async fn stop_recording_session(&self, session_id: i64) -> Result<()>;
    
    async fn get_active_recordings(&self, camera_id: &str) -> Result<Vec<RecordingSession>>;
    
    async fn add_recorded_frame(
        &self,
        session_id: i64,
        timestamp: DateTime<Utc>,
        frame_number: i64,
        frame_data: &[u8],
    ) -> Result<i64>;
    
    async fn list_recordings(&self, query: &RecordingQuery) -> Result<Vec<RecordingSession>>;
    
    async fn get_recorded_frames(
        &self,
        session_id: i64,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordedFrame>>;
    
    async fn delete_old_frames_and_sessions(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize>;
    
    async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
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
        older_than: DateTime<Utc>,
    ) -> Result<usize>;

    async fn cleanup_database(&self, config: &crate::config::RecordingConfig) -> Result<()>;
    
    /// Get a specific video segment by filename (for HTTP streaming)
    async fn get_video_segment_by_filename(
        &self,
        camera_id: &str,
        filename: &str,
    ) -> Result<Option<VideoSegment>>;
        
    // HLS-specific methods
    async fn store_hls_playlist(&self, playlist: &HlsPlaylist) -> Result<()>;
    async fn store_hls_segment(&self, segment: &HlsSegment) -> Result<()>;
    async fn store_hls_playlist_with_segments(&self, playlist: &HlsPlaylist, segments: &[HlsSegment]) -> Result<()>;
    async fn get_hls_playlist(&self, playlist_id: &str) -> Result<Option<HlsPlaylist>>;
    async fn get_hls_segment(&self, playlist_id: &str, segment_name: &str) -> Result<Option<HlsSegment>>;
    async fn cleanup_expired_hls(&self) -> Result<usize>;
}

pub struct SqliteDatabase {
    pool: SqlitePool,
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
        
        let query = format!(
            r#"
            SELECT rf.timestamp, rf.frame_data
            FROM {} rf
            JOIN {} rs ON rf.session_id = rs.id
            WHERE rs.camera_id = ? 
              AND rf.timestamp >= ? 
              AND rf.timestamp <= ?
            ORDER BY rf.timestamp ASC
            LIMIT ?
            "#,
            TABLE_RECORDING_MJPEG, TABLE_RECORDING_SESSIONS
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
        
        let database_url = format!("sqlite://{}?mode=rwc", database_path);
        let pool = SqlitePool::connect(&database_url).await?;
        
        Ok(Self { pool })
    }
}

#[async_trait]
impl DatabaseProvider for SqliteDatabase {
    async fn initialize(&self) -> Result<()> {
        let create_sessions_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                camera_id TEXT NOT NULL,
                start_time TIMESTAMP NOT NULL,
                end_time TIMESTAMP,
                reason TEXT,
                status TEXT NOT NULL DEFAULT 'active'
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
                session_id INTEGER NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                frame_data BLOB NOT NULL,
                PRIMARY KEY (session_id, timestamp),
                FOREIGN KEY (session_id) REFERENCES {}(id)
            )
            "#,
            TABLE_RECORDING_MJPEG, TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&create_mjpeg_query)
            .execute(&self.pool)
            .await?;

        let idx_session_timestamp = format!(
            "CREATE INDEX IF NOT EXISTS idx_session_timestamp ON {}(session_id, timestamp)",
            TABLE_RECORDING_MJPEG
        );
        sqlx::query(&idx_session_timestamp)
            .execute(&self.pool)
            .await?;

        let idx_timestamp = format!(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON {}(timestamp)",
            TABLE_RECORDING_MJPEG
        );
        sqlx::query(&idx_timestamp)
            .execute(&self.pool)
            .await?;

        let create_mp4_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                session_id INTEGER NOT NULL,
                start_time TIMESTAMP NOT NULL,
                end_time TIMESTAMP NOT NULL,
                file_path TEXT,
                size_bytes INTEGER NOT NULL,
                mp4_data BLOB,
                PRIMARY KEY (session_id, start_time),
                FOREIGN KEY (session_id) REFERENCES {}(id) ON DELETE CASCADE
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

        let idx_camera_status = format!(
            "CREATE INDEX IF NOT EXISTS idx_camera_status ON {}(camera_id, status)",
            TABLE_RECORDING_SESSIONS
        );
        sqlx::query(&idx_camera_status)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn create_recording_session(
        &self,
        camera_id: &str,
        reason: Option<&str>,
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
        .bind(Utc::now())
        .bind(reason)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    async fn stop_recording_session(&self, session_id: i64) -> Result<()> {
        let query = format!(
            "UPDATE {} SET end_time = ?, status = 'stopped' WHERE id = ?",
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
            "SELECT * FROM {} WHERE camera_id = ? AND status = 'active'",
            TABLE_RECORDING_SESSIONS
        );
        let rows = sqlx::query(&query)
        .bind(camera_id)
        .fetch_all(&self.pool)
        .await?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(RecordingSession {
                id: row.get("id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
            });
        }

        Ok(sessions)
    }

    async fn add_recorded_frame(
        &self,
        session_id: i64,
        timestamp: DateTime<Utc>,
        _frame_number: i64,
        frame_data: &[u8],
    ) -> Result<i64> {
        let query = format!(
            r#"
            INSERT INTO {} (session_id, timestamp, frame_data)
            VALUES (?, ?, ?)
            "#,
            TABLE_RECORDING_MJPEG
        );
        let result = sqlx::query(&query)
        .bind(session_id)
        .bind(timestamp)
        .bind(frame_data)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
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
        
        let sql = format!("SELECT * FROM {}{} ORDER BY start_time DESC", TABLE_RECORDING_SESSIONS, where_clause);
        
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
                id: row.get("id"),
                camera_id: row.get("camera_id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                reason: row.get("reason"),
                status: RecordingStatus::from(row.get::<String, _>("status")),
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

    async fn delete_old_frames_and_sessions(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        // Start a transaction
        let mut tx = self.pool.begin().await?;
        
        // Step 1: Delete old video segments that reference sessions we want to delete
        // This prevents foreign key constraint violations
        let delete_segments_result = if let Some(cam_id) = camera_id {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT id FROM {} 
                    WHERE camera_id = ? 
                    AND end_time IS NOT NULL 
                    AND end_time < ?
                )
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
            .bind(cam_id)
            .bind(older_than)
            .execute(&mut *tx).await?
        } else {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE session_id IN (
                    SELECT id FROM {} 
                    WHERE end_time IS NOT NULL 
                    AND end_time < ?
                )
                "#,
                TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
            .bind(older_than)
            .execute(&mut *tx).await?
        };
        let deleted_segments = delete_segments_result.rows_affected();
        
        // Step 2: Delete old frames based on their timestamp
        let frames_result = if let Some(cam_id) = camera_id {
            // Delete frames for a specific camera
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE timestamp < ? 
                AND session_id IN (
                    SELECT id FROM {} WHERE camera_id = ?
                )
                "#,
                TABLE_RECORDING_MJPEG, TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
            .bind(older_than)
            .bind(cam_id)
            .execute(&mut *tx).await?
        } else {
            // Delete frames for all cameras
            let query = format!("DELETE FROM {} WHERE timestamp < ?", TABLE_RECORDING_MJPEG);
            sqlx::query(&query)
                .bind(older_than)
                .execute(&mut *tx).await?
        };
        let deleted_frames = frames_result.rows_affected();
        
        // Step 3: Delete completed sessions based on end_time
        // Only delete sessions that have ended (end_time is not NULL) and ended before the cutoff
        let sessions_result = if let Some(cam_id) = camera_id {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE end_time IS NOT NULL 
                AND end_time < ? 
                AND camera_id = ?
                "#,
                TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
            .bind(older_than)
            .bind(cam_id)
            .execute(&mut *tx).await?
        } else {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE end_time IS NOT NULL 
                AND end_time < ?
                "#,
                TABLE_RECORDING_SESSIONS
            );
            sqlx::query(&query)
            .bind(older_than)
            .execute(&mut *tx).await?
        };
        let deleted_sessions = sessions_result.rows_affected();
        
        // Step 4: Clean up orphaned sessions (sessions with no frames left)
        // This handles cases where all frames of a session were deleted but session is still active
        let orphaned_result = if let Some(cam_id) = camera_id {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE camera_id = ?
                AND id NOT IN (
                    SELECT DISTINCT session_id FROM {}
                )
                AND end_time IS NOT NULL
                "#,
                TABLE_RECORDING_SESSIONS, TABLE_RECORDING_MJPEG
            );
            sqlx::query(&query)
            .bind(cam_id)
            .execute(&mut *tx).await?
        } else {
            let query = format!(
                r#"
                DELETE FROM {} 
                WHERE id NOT IN (
                    SELECT DISTINCT session_id FROM {}
                )
                AND end_time IS NOT NULL
                "#,
                TABLE_RECORDING_SESSIONS, TABLE_RECORDING_MJPEG
            );
            sqlx::query(&query)
            .execute(&mut *tx).await?
        };
        let deleted_orphaned = orphaned_result.rows_affected();
        
        // Commit the transaction
        tx.commit().await?;
        
        if deleted_segments > 0 || deleted_frames > 0 || deleted_sessions > 0 || deleted_orphaned > 0 {
            tracing::info!(
                "Cleanup complete: {} video segments deleted, {} frames deleted, {} completed sessions deleted, {} orphaned sessions deleted",
                deleted_segments, deleted_frames, deleted_sessions, deleted_orphaned
            );
        }
        
        // Return total number of sessions deleted
        Ok((deleted_sessions + deleted_orphaned) as usize)
    }
    
    async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Option<RecordedFrame>> {
        // Find the nearest frame before or at the given timestamp, within 1 second
        let one_second_before = timestamp - chrono::Duration::seconds(1);
        
        let query = format!(
            r#"
            SELECT rf.timestamp, rf.frame_data
            FROM {} rf
            JOIN {} rs ON rf.session_id = rs.id
            WHERE rs.camera_id = ? 
              AND rf.timestamp <= ? 
              AND rf.timestamp >= ?
            ORDER BY rf.timestamp DESC
            LIMIT 1
            "#,
            TABLE_RECORDING_MJPEG, TABLE_RECORDING_SESSIONS
        );
        let row = sqlx::query(&query)
        .bind(camera_id)
        .bind(timestamp)
        .bind(one_second_before)
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
        let query = format!(
            r#"
            INSERT INTO {} (session_id, start_time, end_time, file_path, size_bytes, mp4_data)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
            TABLE_RECORDING_MP4
        );
        let result = sqlx::query(&query)
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
            LEFT JOIN {} rs ON vs.session_id = rs.id
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
            LEFT JOIN {} rs ON vs.session_id = rs.id
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
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        // First, select the file paths of the segments to be deleted
        let query = format!(
            "SELECT session_id, start_time, end_time, file_path, size_bytes, mp4_data FROM {} WHERE end_time < ?",
            TABLE_RECORDING_MP4
        );
        let segments_to_delete = sqlx::query_as::<_, VideoSegment>(&query)
        .bind(older_than)
        .fetch_all(&self.pool)
        .await?;

        // Delete the files from the filesystem (only if they have file_path set)
        for segment in &segments_to_delete {
            if let Some(file_path) = &segment.file_path {
                if let Err(e) = tokio::fs::remove_file(file_path).await {
                    tracing::error!("Failed to delete video segment file {}: {}", file_path, e);
                }
            }
            // No action needed for database-stored segments - they'll be deleted with the record
        }

        // Then, delete the records from the database
        let delete_query = format!("DELETE FROM {} WHERE end_time < ?", TABLE_RECORDING_MP4);
        let result = sqlx::query(&delete_query)
            .bind(older_than)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() as usize)
    }

    async fn cleanup_database(&self, config: &crate::config::RecordingConfig) -> Result<()> {
        // Cleanup frames
        if config.frame_storage_enabled {
            if let Ok(duration) = humantime::parse_duration(&config.frame_storage_retention) {
                if duration.as_secs() > 0 {
                    let older_than = Utc::now() - chrono::Duration::from_std(duration).unwrap();
                    match self.delete_old_frames_and_sessions(None, older_than).await {
                        Ok(deleted_count) => {
                            if deleted_count > 0 {
                                tracing::info!("Deleted {} old frame recording sessions.", deleted_count);
                            }
                        }
                        Err(e) => tracing::error!("Error deleting old frames: {}", e),
                    }
                }
            }
        }

        // Cleanup video segments
        if config.video_storage_type != crate::config::Mp4StorageType::Disabled {
            if let Ok(duration) = humantime::parse_duration(&config.video_storage_retention) {
                if duration.as_secs() > 0 {
                    let older_than = Utc::now() - chrono::Duration::from_std(duration).unwrap();
                    match self.delete_old_video_segments(older_than).await {
                        Ok(deleted_count) => {
                            if deleted_count > 0 {
                                tracing::info!("Deleted {} old video segments.", deleted_count);
                            }
                        }
                        Err(e) => tracing::error!("Error deleting old video segments: {}", e),
                    }
                }
            }
        }

        Ok(())
    }
    
    async fn get_video_segment_by_filename(
        &self,
        camera_id: &str,
        filename: &str,
    ) -> Result<Option<VideoSegment>> {
        let start_time = std::time::Instant::now();
        
        let query_str = format!(r#"
            SELECT vs.session_id, vs.start_time, vs.end_time, vs.file_path, vs.size_bytes, vs.mp4_data, rs.camera_id
            FROM {} vs
            JOIN {} rs ON vs.session_id = rs.id
            WHERE rs.camera_id = ? AND (
                vs.file_path LIKE '%' || ? || '%' OR
                ? LIKE '%' || strftime('%Y-%m-%dT%H-%M-%SZ', vs.start_time) || '%'
            )
            ORDER BY vs.start_time DESC
            LIMIT 1
            "#, TABLE_RECORDING_MP4, TABLE_RECORDING_SESSIONS);
        
        tracing::debug!(
            "Executing SQL query for get_video_segment_by_filename:\n{}\nParameters: camera_id='{}', filename='{}'",
            query_str, camera_id, filename
        );
        
        // Try to find by exact filename match first (for filesystem storage)
        let row = sqlx::query(&query_str)
        .bind(camera_id)
        .bind(filename)
        .bind(filename)
        .fetch_optional(&self.pool)
        .await?;
        
        let elapsed = start_time.elapsed();
        
        tracing::debug!(
            "Query completed in {:.3}ms, found: {}",
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
                recording_reason: None, // Not fetched in this query
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
}