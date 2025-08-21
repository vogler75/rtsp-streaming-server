use std::sync::Arc;
use axum::response::IntoResponse;
use axum::extract::{Path as AxumPath, Query};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use bytes::Bytes;

use crate::config;
use crate::recording::RecordingManager;

#[derive(Debug, Deserialize)]
pub struct StartRecordingRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetRecordingsQuery {
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_sort_order_recordings")]
    pub sort_order: String,
}

fn default_sort_order_recordings() -> String {
    "newest".to_string()
}

#[derive(Debug, Deserialize)]
pub struct GetFramesQuery {
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<u16>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            status: "success".to_string(),
            data: Some(data),
            error: None,
            code: None,
        }
    }

    pub fn error(message: &str, code: u16) -> ApiResponse<()> {
        ApiResponse {
            status: "error".to_string(),
            data: None,
            error: Some(message.to_string()),
            code: Some(code),
        }
    }
}

fn check_api_auth(headers: &axum::http::HeaderMap, camera_config: &config::CameraConfig) -> std::result::Result<(), axum::response::Response> {
    if let Some(expected_token) = &camera_config.token {
        if let Some(auth_header) = headers.get("authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    if token == expected_token {
                        return Ok(());
                    }
                }
            }
        }
        return Err((axum::http::StatusCode::UNAUTHORIZED, 
                   Json(ApiResponse::<()>::error("Invalid or missing Authorization header", 401)))
                   .into_response());
    }
    Ok(())
}

pub async fn api_start_recording(
    headers: axum::http::HeaderMap,
    Json(request): Json<StartRecordingRequest>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    // Check if already recording
    if recording_manager.is_recording(&camera_id).await {
        return (axum::http::StatusCode::CONFLICT, 
                Json(ApiResponse::<()>::error("Recording already in progress for this camera", 409)))
                .into_response();
    }

    match recording_manager.start_recording(
        &camera_id,
        "api_client",
        request.reason.as_deref(),
        None,
        frame_sender,
        &camera_config,
    ).await {
        Ok(session_id) => {
            let data = serde_json::json!({
                "session_id": session_id,
                "message": "Recording started",
                "camera_id": camera_id
            });
            Json(ApiResponse::success(data)).into_response()
        }
        Err(_) => {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(ApiResponse::<()>::error("Failed to start recording", 500)))
             .into_response()
        }
    }
}

pub async fn api_stop_recording(
    headers: axum::http::HeaderMap,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    match recording_manager.stop_recording(&camera_id).await {
        Ok(was_recording) => {
            if was_recording {
                let data = serde_json::json!({
                    "message": "Recording stopped",
                    "camera_id": camera_id
                });
                Json(ApiResponse::success(data)).into_response()
            } else {
                let data = serde_json::json!({
                    "message": "No active recording found",
                    "camera_id": camera_id
                });
                Json(ApiResponse::success(data)).into_response()
            }
        }
        Err(_) => {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(ApiResponse::<()>::error("Failed to stop recording", 500)))
             .into_response()
        }
    }
}

pub async fn api_list_recordings(
    headers: axum::http::HeaderMap,
    Query(query): Query<GetRecordingsQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    match recording_manager.list_recordings(Some(&camera_id), query.from, query.to).await {
        Ok(mut recordings) => {
            // Sort recordings based on sort_order parameter
            match query.sort_order.as_str() {
                "oldest" => recordings.sort_by(|a, b| a.start_time.cmp(&b.start_time)),
                _ => recordings.sort_by(|a, b| b.start_time.cmp(&a.start_time)), // "newest" (default)
            }
            
            let recordings_data: Vec<serde_json::Value> = recordings
                .into_iter()
                .map(|r| serde_json::json!({
                    "id": r.id,
                    "camera_id": r.camera_id,
                    "start_time": r.start_time,
                    "end_time": r.end_time,
                    "reason": r.reason,
                    "status": format!("{:?}", r.status).to_lowercase(),
                    "duration_seconds": r.end_time
                        .map(|end| end.signed_duration_since(r.start_time).num_seconds())
                }))
                .collect();

            let data = serde_json::json!({
                "recordings": recordings_data,
                "count": recordings_data.len(),
                "camera_id": camera_id
            });
            Json(ApiResponse::success(data)).into_response()
        }
        Err(_) => {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(ApiResponse::<()>::error("Failed to list recordings", 500)))
             .into_response()
        }
    }
}

pub async fn api_get_recorded_frames(
    headers: axum::http::HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
    Query(query): Query<GetFramesQuery>,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    match recording_manager.get_recorded_frames(session_id, query.from, query.to).await {
        Ok(frames) => {
            let frames_data: Vec<serde_json::Value> = frames
                .into_iter()
                .map(|f| serde_json::json!({
                    "timestamp": f.timestamp,
                    "frame_size": f.frame_data.len()
                    // Note: Not including actual frame_data in JSON response due to size
                }))
                .collect();

            let data = serde_json::json!({
                "session_id": session_id,
                "frames": frames_data,
                "count": frames_data.len(),
                "note": "Frame data not included in response due to size - use binary WebSocket for frame streaming"
            });
            Json(ApiResponse::success(data)).into_response()
        }
        Err(_) => {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(ApiResponse::<()>::error("Failed to get recorded frames", 500)))
             .into_response()
        }
    }
}

pub async fn api_get_active_recording(
    headers: axum::http::HeaderMap,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    if let Some(active_recording) = recording_manager.get_active_recording(&camera_id).await {
        let data = serde_json::json!({
            "active": true,
            "session_id": active_recording.session_id,
            "start_time": active_recording.start_time,
            "frame_count": active_recording.frame_count,
            "camera_id": camera_id
        });
        Json(ApiResponse::success(data)).into_response()
    } else {
        let data = serde_json::json!({
            "message": "No active recording found",
            "camera_id": camera_id,
            "active": false
        });
        Json(ApiResponse::success(data)).into_response()
    }
}

pub async fn api_get_recording_size(
    headers: axum::http::HeaderMap,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    match recording_manager.get_database_size(&camera_id).await {
        Ok(size_bytes) => {
            let data = serde_json::json!({
                "camera_id": camera_id,
                "size_bytes": size_bytes,
                "size_mb": (size_bytes as f64) / (1024.0 * 1024.0),
                "size_gb": (size_bytes as f64) / (1024.0 * 1024.0 * 1024.0)
            });
            Json(ApiResponse::success(data)).into_response()
        }
        Err(_) => {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(ApiResponse::<()>::error("Failed to get database size", 500)))
             .into_response()
        }
    }
}