use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::path::PathBuf;
use crate::errors::{StreamError, Result};
use crate::database::DatabaseProvider;
use std::fs;
use tokio::process::Command;
use tracing::{info, error, warn, debug};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExportJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportJob {
    pub job_id: String,
    pub camera_id: String,
    pub from_time: DateTime<Utc>,
    pub to_time: DateTime<Utc>,
    pub status: ExportJobStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub output_filename: String,
    pub output_path: String,
    pub file_size_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub progress_percent: u8,
}

impl ExportJob {
    fn new(camera_id: String, from_time: DateTime<Utc>, to_time: DateTime<Utc>, export_path: &str) -> Self {
        let job_id = Uuid::new_v4().to_string();
        let output_filename = format!(
            "{}_{}_{}..mp4",
            camera_id,
            from_time.format("%Y-%m-%dT%H-%M-%S"),
            to_time.format("%Y-%m-%dT%H-%M-%S")
        );
        let output_path = PathBuf::from(export_path)
            .join(&output_filename)
            .to_string_lossy()
            .to_string();

        Self {
            job_id,
            camera_id,
            from_time,
            to_time,
            status: ExportJobStatus::Queued,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            output_filename,
            output_path,
            file_size_bytes: None,
            error_message: None,
            progress_percent: 0,
        }
    }
}

pub struct ExportJobManager {
    jobs: Arc<RwLock<VecDeque<ExportJob>>>,
    max_jobs: usize,
    export_path: String,
}

impl ExportJobManager {
    pub fn new(export_path: String, max_jobs: usize) -> Self {
        // Create export directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(&export_path) {
            error!("Failed to create export directory {}: {}", export_path, e);
        }

        Self {
            jobs: Arc::new(RwLock::new(VecDeque::new())),
            max_jobs,
            export_path,
        }
    }

    /// Create a new export job
    pub async fn create_job(
        &self,
        camera_id: String,
        from_time: DateTime<Utc>,
        to_time: DateTime<Utc>,
    ) -> String {
        let job = ExportJob::new(camera_id, from_time, to_time, &self.export_path);
        let job_id = job.job_id.clone();

        let mut jobs = self.jobs.write().await;
        jobs.push_back(job);

        // Cleanup if we exceed max jobs
        self.cleanup_old_jobs_internal(&mut jobs).await;

        info!("Created export job {}", job_id);
        job_id
    }

    /// Get a specific job by ID
    pub async fn get_job(&self, job_id: &str) -> Option<ExportJob> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.job_id == job_id).cloned()
    }

    /// List all jobs, optionally filtered by camera_id and/or status
    pub async fn list_jobs(
        &self,
        camera_id: Option<&str>,
        status: Option<ExportJobStatus>,
    ) -> Vec<ExportJob> {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .filter(|j| {
                let camera_match = camera_id.map_or(true, |cid| j.camera_id == cid);
                let status_match = status.as_ref().map_or(true, |s| &j.status == s);
                camera_match && status_match
            })
            .cloned()
            .collect()
    }

    /// Update job status and metadata
    async fn update_job<F>(&self, job_id: &str, update_fn: F) -> Result<()>
    where
        F: FnOnce(&mut ExportJob),
    {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.job_id == job_id) {
            update_fn(job);
            Ok(())
        } else {
            Err(StreamError::not_found(format!("Export job {} not found", job_id)))
        }
    }

    /// Cleanup old jobs (keep only last max_jobs)
    async fn cleanup_old_jobs_internal(&self, jobs: &mut VecDeque<ExportJob>) {
        while jobs.len() > self.max_jobs {
            if let Some(old_job) = jobs.pop_front() {
                debug!("Removed old export job {} from queue (cleanup)", old_job.job_id);
            }
        }
    }

    /// Cleanup old jobs (public interface)
    pub async fn cleanup_old_jobs(&self) {
        let mut jobs = self.jobs.write().await;
        self.cleanup_old_jobs_internal(&mut jobs).await;
    }

    /// Get the next queued job for a specific camera
    pub async fn get_next_queued_job(&self, camera_id: &str) -> Option<ExportJob> {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .find(|j| j.camera_id == camera_id && j.status == ExportJobStatus::Queued)
            .cloned()
    }

    /// Check if there's a running job for a specific camera
    pub async fn has_running_job(&self, camera_id: &str) -> bool {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .any(|j| j.camera_id == camera_id && j.status == ExportJobStatus::Running)
    }

    /// Process an export job
    pub async fn process_job(
        &self,
        job_id: &str,
        database: Arc<dyn DatabaseProvider>,
        recording_base_path: &str,
    ) -> Result<()> {
        // Mark as running
        self.update_job(job_id, |job| {
            job.status = ExportJobStatus::Running;
            job.started_at = Some(Utc::now());
            job.progress_percent = 5;
        })
        .await?;

        let job = self
            .get_job(job_id)
            .await
            .ok_or_else(|| StreamError::not_found(format!("Job {} not found", job_id)))?;

        info!(
            "[{}] Starting export job {} from {} to {}",
            job.camera_id, job_id, job.from_time, job.to_time
        );

        // Execute the export
        match self
            .execute_export(&job, database, recording_base_path)
            .await
        {
            Ok(file_size) => {
                info!("[{}] Export job {} completed successfully", job.camera_id, job_id);
                self.update_job(job_id, |job| {
                    job.status = ExportJobStatus::Completed;
                    job.completed_at = Some(Utc::now());
                    job.file_size_bytes = Some(file_size);
                    job.progress_percent = 100;
                })
                .await?;
                Ok(())
            }
            Err(e) => {
                error!("[{}] Export job {} failed: {}", job.camera_id, job_id, e);
                self.update_job(job_id, |job| {
                    job.status = ExportJobStatus::Failed;
                    job.completed_at = Some(Utc::now());
                    job.error_message = Some(e.to_string());
                })
                .await?;
                Err(e)
            }
        }
    }

    /// Execute the actual export using FFmpeg
    async fn execute_export(
        &self,
        job: &ExportJob,
        database: Arc<dyn DatabaseProvider>,
        recording_base_path: &str,
    ) -> Result<i64> {
        // Get MP4 segments in the time range
        let segments = database
            .get_mp4_segments_in_range(&job.camera_id, job.from_time, job.to_time)
            .await?;

        if segments.is_empty() {
            return Err(StreamError::not_found(format!(
                "No MP4 segments found for camera {} in time range {} to {}",
                job.camera_id, job.from_time, job.to_time
            )));
        }

        info!(
            "[{}] Found {} MP4 segments to concatenate",
            job.camera_id,
            segments.len()
        );

        // Update progress
        self.update_job(&job.job_id, |j| j.progress_percent = 10)
            .await?;

        // Create temp directory for concat file
        let temp_dir = PathBuf::from(&self.export_path).join("temp");
        fs::create_dir_all(&temp_dir).map_err(|e| {
            StreamError::internal(format!("Failed to create temp directory: {}", e))
        })?;

        // Create FFmpeg concat file
        let concat_file_path = temp_dir.join(format!("concat_{}.txt", job.job_id));
        let mut concat_content = String::new();

        for segment in &segments {
            // Resolve actual file path
            let file_path = if segment.storage_path.is_some() {
                // Filesystem storage
                PathBuf::from(recording_base_path)
                    .join(&job.camera_id)
                    .join(segment.storage_path.as_ref().unwrap())
            } else {
                // Database storage - extract to temp file
                let temp_file_path = temp_dir.join(format!("segment_{}_{}.mp4", job.job_id, segment.session_id));
                database
                    .extract_mp4_segment_to_file(segment.session_id, &temp_file_path.to_string_lossy())
                    .await?;
                temp_file_path
            };

            // Add to concat list
            concat_content.push_str(&format!(
                "file '{}'\n",
                file_path.to_string_lossy().replace("'", "'\\''")
            ));
        }

        fs::write(&concat_file_path, concat_content).map_err(|e| {
            StreamError::internal(format!("Failed to write concat file: {}", e))
        })?;

        info!("[{}] Created concat file with {} segments", job.camera_id, segments.len());

        // Update progress
        self.update_job(&job.job_id, |j| j.progress_percent = 20)
            .await?;

        // Run FFmpeg concat
        let output = Command::new("ffmpeg")
            .args(&[
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                &concat_file_path.to_string_lossy(),
                "-c",
                "copy",
                "-y",
                &job.output_path,
            ])
            .output()
            .await
            .map_err(|e| StreamError::internal(format!("Failed to execute FFmpeg: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(StreamError::internal(format!("FFmpeg failed: {}", stderr)));
        }

        // Update progress
        self.update_job(&job.job_id, |j| j.progress_percent = 90)
            .await?;

        // Get file size
        let file_size = fs::metadata(&job.output_path)
            .map_err(|e| StreamError::internal(format!("Failed to get file metadata: {}", e)))?
            .len() as i64;

        // Cleanup temp files
        if let Err(e) = fs::remove_file(&concat_file_path) {
            warn!("Failed to remove concat file: {}", e);
        }

        // Remove temp segment files (database-stored segments)
        for segment in &segments {
            if segment.storage_path.is_none() {
                let temp_file_path = temp_dir.join(format!("segment_{}_{}.mp4", job.job_id, segment.session_id));
                if let Err(e) = fs::remove_file(&temp_file_path) {
                    warn!("Failed to remove temp segment file: {}", e);
                }
            }
        }

        info!(
            "[{}] Export completed: {} ({} bytes)",
            job.camera_id, job.output_filename, file_size
        );

        Ok(file_size)
    }
}

// Struct to hold MP4 segment information
#[derive(Debug, Clone)]
pub struct Mp4SegmentInfo {
    pub session_id: i64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub storage_path: Option<String>, // None = database storage, Some = filesystem storage
}
