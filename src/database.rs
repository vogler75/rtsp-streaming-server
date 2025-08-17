use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{SqlitePool, Row};
use crate::errors::Result;

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
    
    async fn get_frames_in_range(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<RecordedFrame>>;
    
    async fn delete_old_recordings(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize>;
    
    async fn get_frame_at_timestamp(
        &self,
        camera_id: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Option<RecordedFrame>>;
    
    async fn get_database_size(&self) -> Result<i64>;
}

pub struct SqliteDatabase {
    pool: SqlitePool,
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
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS recording_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                camera_id TEXT NOT NULL,
                start_time TIMESTAMP NOT NULL,
                end_time TIMESTAMP,
                reason TEXT,
                status TEXT NOT NULL DEFAULT 'active'
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS recorded_frames (
                session_id INTEGER NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                frame_data BLOB NOT NULL,
                FOREIGN KEY (session_id) REFERENCES recording_sessions(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_session_timestamp ON recorded_frames(session_id, timestamp)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_timestamp ON recorded_frames(timestamp)")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn create_recording_session(
        &self,
        camera_id: &str,
        reason: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO recording_sessions (camera_id, start_time, reason)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(camera_id)
        .bind(Utc::now())
        .bind(reason)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    async fn stop_recording_session(&self, session_id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE recording_sessions SET end_time = ?, status = 'stopped' WHERE id = ?",
        )
        .bind(Utc::now())
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_active_recordings(&self, camera_id: &str) -> Result<Vec<RecordingSession>> {
        let rows = sqlx::query(
            "SELECT * FROM recording_sessions WHERE camera_id = ? AND status = 'active'",
        )
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
        let result = sqlx::query(
            r#"
            INSERT INTO recorded_frames (session_id, timestamp, frame_data)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(session_id)
        .bind(timestamp)
        .bind(frame_data)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
    }

    async fn list_recordings(&self, query: &RecordingQuery) -> Result<Vec<RecordingSession>> {
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
        
        let sql = format!("SELECT * FROM recording_sessions{} ORDER BY start_time DESC", where_clause);
        
        let mut query_builder = sqlx::query(&sql);
        for value in &bind_values {
            query_builder = query_builder.bind(value);
        }
        
        let rows = query_builder.fetch_all(&self.pool).await?;

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
        let mut sql = "SELECT * FROM recorded_frames WHERE session_id = ?".to_string();
        
        if from.is_some() {
            sql.push_str(" AND timestamp >= ?");
        }
        if to.is_some() {
            sql.push_str(" AND timestamp <= ?");
        }
        
        sql.push_str(" ORDER BY timestamp ASC");

        let mut query = sqlx::query(&sql).bind(session_id);
        
        if let Some(from_time) = from {
            query = query.bind(from_time);
        }
        if let Some(to_time) = to {
            query = query.bind(to_time);
        }

        let rows = query.fetch_all(&self.pool).await?;

        let mut frames = Vec::new();
        for row in rows {
            frames.push(RecordedFrame {
                timestamp: row.get("timestamp"),
                frame_data: row.get("frame_data"),
            });
        }

        Ok(frames)
    }

    async fn get_frames_in_range(
        &self,
        camera_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<RecordedFrame>> {
        let rows = sqlx::query(
            r#"
            SELECT rf.* FROM recorded_frames rf
            JOIN recording_sessions rs ON rf.session_id = rs.id
            WHERE rs.camera_id = ? AND rf.timestamp >= ? AND rf.timestamp <= ?
            ORDER BY rf.timestamp ASC
            "#,
        )
        .bind(camera_id)
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await?;

        let mut frames = Vec::new();
        for row in rows {
            frames.push(RecordedFrame {
                timestamp: row.get("timestamp"),
                frame_data: row.get("frame_data"),
            });
        }

        Ok(frames)
    }
    
    async fn delete_old_recordings(
        &self,
        camera_id: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        // Start a transaction
        let mut tx = self.pool.begin().await?;
        
        // Step 1: Delete old frames based on their timestamp
        let delete_frames_query = if let Some(cam_id) = camera_id {
            // Delete frames for a specific camera
            sqlx::query(
                r#"
                DELETE FROM recorded_frames 
                WHERE timestamp < ? 
                AND session_id IN (
                    SELECT id FROM recording_sessions WHERE camera_id = ?
                )
                "#
            )
            .bind(older_than)
            .bind(cam_id)
        } else {
            // Delete frames for all cameras
            sqlx::query("DELETE FROM recorded_frames WHERE timestamp < ?")
                .bind(older_than)
        };
        
        let frames_result = delete_frames_query.execute(&mut *tx).await?;
        let deleted_frames = frames_result.rows_affected();
        
        // Step 2: Delete completed sessions based on end_time
        // Only delete sessions that have ended (end_time is not NULL) and ended before the cutoff
        let delete_sessions_query = if let Some(cam_id) = camera_id {
            sqlx::query(
                r#"
                DELETE FROM recording_sessions 
                WHERE end_time IS NOT NULL 
                AND end_time < ? 
                AND camera_id = ?
                "#
            )
            .bind(older_than)
            .bind(cam_id)
        } else {
            sqlx::query(
                r#"
                DELETE FROM recording_sessions 
                WHERE end_time IS NOT NULL 
                AND end_time < ?
                "#
            )
            .bind(older_than)
        };
        
        let sessions_result = delete_sessions_query.execute(&mut *tx).await?;
        let deleted_sessions = sessions_result.rows_affected();
        
        // Step 3: Clean up orphaned sessions (sessions with no frames left)
        // This handles cases where all frames of a session were deleted but session is still active
        let cleanup_orphaned_query = if let Some(cam_id) = camera_id {
            sqlx::query(
                r#"
                DELETE FROM recording_sessions 
                WHERE camera_id = ?
                AND id NOT IN (
                    SELECT DISTINCT session_id FROM recorded_frames
                )
                AND end_time IS NOT NULL
                "#
            )
            .bind(cam_id)
        } else {
            sqlx::query(
                r#"
                DELETE FROM recording_sessions 
                WHERE id NOT IN (
                    SELECT DISTINCT session_id FROM recorded_frames
                )
                AND end_time IS NOT NULL
                "#
            )
        };
        
        let orphaned_result = cleanup_orphaned_query.execute(&mut *tx).await?;
        let deleted_orphaned = orphaned_result.rows_affected();
        
        // Commit the transaction
        tx.commit().await?;
        
        if deleted_frames > 0 || deleted_sessions > 0 || deleted_orphaned > 0 {
            tracing::info!(
                "Cleanup complete: {} frames deleted, {} completed sessions deleted, {} orphaned sessions deleted",
                deleted_frames, deleted_sessions, deleted_orphaned
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
        
        let row = sqlx::query(
            r#"
            SELECT rf.timestamp, rf.frame_data
            FROM recorded_frames rf
            JOIN recording_sessions rs ON rf.session_id = rs.id
            WHERE rs.camera_id = ? 
              AND rf.timestamp <= ? 
              AND rf.timestamp >= ?
            ORDER BY rf.timestamp DESC
            LIMIT 1
            "#
        )
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
}