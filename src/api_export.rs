use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use tracing::{info, error};

use crate::config;
use crate::export_jobs::{ExportJobManager, ExportJobStatus};
use crate::api_recording::{ApiResponse, check_api_auth};

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ListJobsQuery {
    pub status: Option<String>,
}

/// Start an MP4 export job
pub async fn api_export_start(
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    export_manager: Arc<ExportJobManager>,
) -> Response {
    // Check authentication
    if let Err(e) = check_api_auth(&headers, &camera_config) {
        return e.into_response();
    }

    info!(
        "[{}] Starting export job from {} to {}",
        camera_id, query.from, query.to
    );

    // Create the export job
    let job_id = export_manager
        .create_job(camera_id.clone(), query.from, query.to)
        .await;

    let job = export_manager.get_job(&job_id).await;

    match job {
        Some(job) => {
            let response = ApiResponse::success(serde_json::json!({
                "job_id": job.job_id,
                "status": job.status,
                "output_filename": job.output_filename,
                "from_time": job.from_time,
                "to_time": job.to_time,
            }));

            (StatusCode::OK, Json(response)).into_response()
        }
        None => {
            let response = ApiResponse::<()>::error("Failed to create export job", 500);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
        }
    }
}

/// Get status of a specific export job
pub async fn api_export_get_job(
    headers: HeaderMap,
    Path(job_id): Path<String>,
    camera_id: String,
    camera_config: config::CameraConfig,
    export_manager: Arc<ExportJobManager>,
) -> Response {
    // Check authentication
    if let Err(e) = check_api_auth(&headers, &camera_config) {
        return e.into_response();
    }

    match export_manager.get_job(&job_id).await {
        Some(job) => {
            // Verify the job belongs to this camera
            if job.camera_id != camera_id {
                let response = ApiResponse::<()>::error("Job not found for this camera", 404);
                return (StatusCode::NOT_FOUND, Json(response)).into_response();
            }

            let response = ApiResponse::success(serde_json::json!(job));
            (StatusCode::OK, Json(response)).into_response()
        }
        None => {
            let response = ApiResponse::<()>::error(&format!("Export job {} not found", job_id), 404);
            (StatusCode::NOT_FOUND, Json(response)).into_response()
        }
    }
}

/// List all export jobs for a camera
pub async fn api_export_list_jobs(
    headers: HeaderMap,
    Query(query): Query<ListJobsQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    export_manager: Arc<ExportJobManager>,
) -> Response {
    // Check authentication
    if let Err(e) = check_api_auth(&headers, &camera_config) {
        return e.into_response();
    }

    // Parse status filter if provided
    let status_filter = query.status.as_ref().and_then(|s| {
        match s.to_lowercase().as_str() {
            "queued" => Some(ExportJobStatus::Queued),
            "running" => Some(ExportJobStatus::Running),
            "completed" => Some(ExportJobStatus::Completed),
            "failed" => Some(ExportJobStatus::Failed),
            _ => None,
        }
    });

    let jobs = export_manager
        .list_jobs(Some(&camera_id), status_filter)
        .await;

    let response = ApiResponse::success(serde_json::json!({
        "jobs": jobs,
        "total_count": jobs.len(),
        "camera_id": camera_id,
    }));

    (StatusCode::OK, Json(response)).into_response()
}

/// Download an exported MP4 file
pub async fn api_export_download(
    headers: HeaderMap,
    Path(job_id): Path<String>,
    camera_id: String,
    camera_config: config::CameraConfig,
    export_manager: Arc<ExportJobManager>,
) -> Response {
    // Check authentication
    if let Err(e) = check_api_auth(&headers, &camera_config) {
        return e.into_response();
    }

    match export_manager.get_job(&job_id).await {
        Some(job) => {
            // Verify the job belongs to this camera
            if job.camera_id != camera_id {
                let response = ApiResponse::<()>::error("Job not found for this camera", 404);
                return (StatusCode::NOT_FOUND, Json(response)).into_response();
            }

            // Check if job is completed
            if job.status != ExportJobStatus::Completed {
                let response = ApiResponse::<()>::error(&format!("Export job is not completed (status: {:?})", job.status), 400);
                return (StatusCode::BAD_REQUEST, Json(response)).into_response();
            }

            // Read the file
            match tokio::fs::read(&job.output_path).await {
                Ok(data) => {
                    let mut response_headers = HeaderMap::new();
                    response_headers.insert(
                        "Content-Type",
                        "video/mp4".parse().unwrap(),
                    );
                    response_headers.insert(
                        "Content-Disposition",
                        format!("attachment; filename=\"{}\"", job.output_filename)
                            .parse()
                            .unwrap(),
                    );
                    response_headers.insert(
                        "Content-Length",
                        data.len().to_string().parse().unwrap(),
                    );

                    (StatusCode::OK, response_headers, data).into_response()
                }
                Err(e) => {
                    error!("[{}] Failed to read export file: {}", camera_id, e);
                    let response = ApiResponse::<()>::error("Failed to read export file", 500);
                    (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
                }
            }
        }
        None => {
            let response = ApiResponse::<()>::error(&format!("Export job {} not found", job_id), 404);
            (StatusCode::NOT_FOUND, Json(response)).into_response()
        }
    }
}
