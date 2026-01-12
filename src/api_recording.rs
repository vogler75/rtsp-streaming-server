use std::sync::Arc;
use axum::response::IntoResponse;
use axum::extract::{Path as AxumPath, Query};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use bytes::Bytes;

use crate::config;
use crate::recording::RecordingManager;
use crate::mp4::HlsTimeRangeQuery;

#[derive(Debug, Deserialize)]
pub struct StartRecordingRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetKeepSessionQuery {
    #[serde(default = "default_true")]
    pub keep: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct GetRecordingsQuery {
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub reason: Option<String>, // Filter by recording reason using SQL wildcards (e.g., 'Manual' or '%alarm%')
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

#[derive(Debug, Deserialize)]
pub struct GetFrameByTimestampQuery {
    #[serde(default)]
    pub tolerance: Option<String>, // e.g., "30s", "5m", "1h" - default is no tolerance (exact match)
}

#[derive(Debug, Deserialize)]
pub struct GetMp4SegmentsQuery {
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub reason: Option<String>,
    #[serde(default = "default_segments_limit")]
    pub limit: i64,
    #[serde(default = "default_sort_order_recordings")]
    pub sort_order: String,
}

fn default_segments_limit() -> i64 {
    1000
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

pub fn check_api_auth(headers: &axum::http::HeaderMap, camera_config: &config::CameraConfig) -> std::result::Result<(), axum::response::Response> {
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
    pre_recording_buffer: Option<crate::pre_recording_buffer::PreRecordingBuffer>,
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
        pre_recording_buffer.as_ref(),
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

    match recording_manager.list_recordings_filtered(Some(&camera_id), query.from, query.to, query.reason.as_deref()).await {
        Ok(mut recordings) => {
            // Sort recordings based on sort_order parameter
            match query.sort_order.as_str() {
                "oldest" => recordings.sort_by(|a, b| a.start_time.cmp(&b.start_time)),
                _ => recordings.sort_by(|a, b| b.start_time.cmp(&a.start_time)), // "newest" (default)
            }
            
            let recordings_data: Vec<serde_json::Value> = recordings
                .into_iter()
                .map(|r| serde_json::json!({
                    "id": r.session_id,
                    "camera_id": r.camera_id,
                    "start_time": r.start_time,
                    "end_time": r.end_time,
                    "reason": r.reason,
                    "status": format!("{:?}", r.status).to_lowercase(),
                    "duration_seconds": r.end_time
                        .map(|end| end.signed_duration_since(r.start_time).num_seconds()),
                    "keep_session": r.keep_session
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
                "count": frames_data.len()
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

    // Get recording config to check HLS/MP4 status
    let recording_config = recording_manager.get_recording_config();
    let hls_enabled = camera_config.get_hls_storage_enabled()
        .unwrap_or(recording_config.hls_storage_enabled);
    let mp4_storage_type = camera_config.get_mp4_storage_type()
        .unwrap_or(&recording_config.mp4_storage_type);
    let mp4_enabled = mp4_storage_type != &config::Mp4StorageType::Disabled;
    let frame_storage_enabled = camera_config.get_frame_storage_enabled()
        .unwrap_or(recording_config.frame_storage_enabled);

    if let Some(active_recording) = recording_manager.get_active_recording(&camera_id).await {
        let data = serde_json::json!({
            "active": true,
            "session_id": active_recording.session_id,
            "start_time": active_recording.start_time,
            "frame_count": active_recording.frame_count,
            "camera_id": camera_id,
            "storage": {
                "hls_enabled": hls_enabled,
                "mp4_enabled": mp4_enabled,
                "frame_storage_enabled": frame_storage_enabled
            }
        });
        Json(ApiResponse::success(data)).into_response()
    } else {
        let data = serde_json::json!({
            "message": "No active recording found",
            "camera_id": camera_id,
            "active": false,
            "storage": {
                "hls_enabled": hls_enabled,
                "mp4_enabled": mp4_enabled,
                "frame_storage_enabled": frame_storage_enabled
            }
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

pub async fn api_list_mp4_segments(
    headers: axum::http::HeaderMap,
    Query(query): Query<GetMp4SegmentsQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    let camera_path = &camera_config.path;

    match recording_manager.list_video_segments_filtered(
        &camera_id,
        query.from,
        query.to,
        query.reason.as_deref(),
        query.limit,
        &query.sort_order,
    ).await {
        Ok(segments) => {
            let segments_data: Vec<serde_json::Value> = segments
                .into_iter()
                .map(|s| {
                    // Calculate duration from start and end times
                    let duration_seconds = s.end_time.signed_duration_since(s.start_time).num_seconds();
                    
                    // Generate filename from file_path if it exists, otherwise create one from timestamp
                    let filename = match &s.file_path {
                        Some(path) => {
                            std::path::Path::new(path)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    // Fallback to generating filename from exact timestamp
                                    s.start_time.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
                                })
                        },
                        None => {
                            // For database storage, generate a filename based on exact timestamp
                            s.start_time.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
                        }
                    };
                    
                    serde_json::json!({
                        "id": format!("{}_{}", s.session_id, s.start_time.timestamp()),
                        "session_id": s.session_id,
                        "start_time": s.start_time,
                        "end_time": s.end_time,
                        "duration_seconds": duration_seconds,
                        "url": format!("{}/control/recordings/mp4/segments/{}", camera_path, filename),
                        "size_bytes": s.size_bytes,
                        "recording_reason": s.recording_reason.unwrap_or_else(|| "Unknown".to_string()),
                        "camera_id": s.camera_id
                    })
                })
                .collect();

            let data = serde_json::json!({
                "segments": segments_data,
                "count": segments_data.len(),
                "camera_id": camera_id,
                "query": {
                    "from": query.from,
                    "to": query.to,
                    "reason": query.reason,
                    "limit": query.limit,
                    "sort_order": query.sort_order
                }
            });
            Json(ApiResponse::success(data)).into_response()
        }
        Err(_) => {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(ApiResponse::<()>::error("Failed to list MP4 segments", 500)))
             .into_response()
        }
    }
}

pub async fn api_stream_mp4_segment(
    headers: axum::http::HeaderMap,
    AxumPath(filename): AxumPath<String>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    // Parse Range header using the existing function
    let range = crate::mp4::parse_range_header(headers.get("range"));

    // Call the core logic in mp4.rs
    crate::mp4::stream_mp4_segment(&camera_id, &filename, range, &camera_config, &recording_manager).await
}

pub async fn api_serve_hls_timerange(
    headers: axum::http::HeaderMap,
    Query(query): Query<HlsTimeRangeQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    // Create a mock app state for the existing HLS function
    let app_state = crate::AppState {
        camera_streams: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        camera_configs: {
            let mut configs = std::collections::HashMap::new();
            configs.insert(camera_id.clone(), camera_config);
            Arc::new(tokio::sync::RwLock::new(configs))
        },
        mqtt_handle: None,
        recording_manager: Some(recording_manager),
        transcoding_config: Arc::new(crate::config::TranscodingConfig {
            output_format: "mjpeg".to_string(),
            capture_framerate: 30,
            output_framerate: None,
            channel_buffer_size: Some(1024),
            debug_capture: Some(false),
            debug_duplicate_frames: Some(false),
        }),
        recording_config: None,
        admin_token: None,
        cameras_directory: "cameras".to_string(),
        start_time: std::time::Instant::now(),
        server_config: Arc::new(crate::config::ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 8080,
            tls: None,
            cors_allow_origin: None,
            admin_token: None,
            cameras_directory: None,
            mp4_export_path: "exports".to_string(),
            mp4_export_max_jobs: 100,
        }),
        export_manager: None,
    };

    // Call the existing HLS playlist function
    crate::mp4::serve_hls_playlist(
        axum::extract::Path(camera_id),
        axum::extract::Query(query),
        axum::extract::State(app_state),
    ).await
}

pub async fn api_serve_hls_segment(
    headers: axum::http::HeaderMap,
    AxumPath((playlist_id, segment_name)): AxumPath<(String, String)>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    // Create a mock app state for the existing HLS function
    let app_state = crate::AppState {
        camera_streams: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        camera_configs: {
            let mut configs = std::collections::HashMap::new();
            configs.insert(camera_id.clone(), camera_config);
            Arc::new(tokio::sync::RwLock::new(configs))
        },
        mqtt_handle: None,
        recording_manager: Some(recording_manager),
        transcoding_config: Arc::new(crate::config::TranscodingConfig {
            output_format: "mjpeg".to_string(),
            capture_framerate: 30,
            output_framerate: None,
            channel_buffer_size: Some(1024),
            debug_capture: Some(false),
            debug_duplicate_frames: Some(false),
        }),
        recording_config: None,
        admin_token: None,
        cameras_directory: "cameras".to_string(),
        start_time: std::time::Instant::now(),
        server_config: Arc::new(crate::config::ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 8080,
            tls: None,
            cors_allow_origin: None,
            admin_token: None,
            cameras_directory: None,
            mp4_export_path: "exports".to_string(),
            mp4_export_max_jobs: 100,
        }),
        export_manager: None,
    };

    // Call the existing HLS segment function
    crate::mp4::serve_hls_segment(
        axum::extract::Path((camera_id, playlist_id, segment_name)),
        axum::extract::State(app_state),
    ).await
}

/// Parse tolerance string like "30s", "5m", "1h" into seconds
fn parse_tolerance_string(tolerance: &str) -> Result<i64, String> {
    if tolerance.is_empty() {
        return Ok(0);
    }
    
    let len = tolerance.len();
    if len < 2 {
        return Err(format!("Invalid tolerance format: {}", tolerance));
    }
    
    let (number_str, unit) = tolerance.split_at(len - 1);
    let number: i64 = number_str.parse().map_err(|_| format!("Invalid number in tolerance: {}", number_str))?;
    
    match unit {
        "s" => Ok(number),
        "m" => Ok(number * 60),
        "h" => Ok(number * 3600),
        _ => Err(format!("Invalid time unit: {}. Use 's', 'm', or 'h'", unit))
    }
}

pub async fn api_get_frame_by_timestamp(
    headers: axum::http::HeaderMap,
    AxumPath(timestamp_str): AxumPath<String>,
    Query(query): Query<GetFrameByTimestampQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    // Parse the timestamp from the path parameter
    let timestamp = match chrono::DateTime::parse_from_rfc3339(&timestamp_str) {
        Ok(ts) => ts.with_timezone(&chrono::Utc),
        Err(_) => {
            return Json(ApiResponse::<()>::error("Invalid timestamp format. Use ISO 8601 format (e.g., 2025-08-23T10:30:45.123Z)", 400)).into_response();
        }
    };

    // Parse tolerance parameter
    let tolerance_seconds = if let Some(tolerance_str) = query.tolerance {
        match parse_tolerance_string(&tolerance_str) {
            Ok(seconds) => Some(seconds),
            Err(err) => {
                return Json(ApiResponse::<()>::error(&format!("Invalid tolerance parameter: {}", err), 400)).into_response();
            }
        }
    } else {
        None // Default: no tolerance (exact match)
    };

    // Get the frame
    match recording_manager.get_frame_at_timestamp(&camera_id, timestamp, tolerance_seconds).await {
        Ok(Some(frame)) => {
            // Return raw JPEG data
            axum::response::Response::builder()
                .status(200)
                .header("Content-Type", "image/jpeg")
                .header("Content-Length", frame.frame_data.len())
                .header("X-Frame-Timestamp", frame.timestamp.to_rfc3339())
                .body(axum::body::Body::from(frame.frame_data))
                .unwrap_or_else(|_| {
                    Json(ApiResponse::<()>::error("Failed to build response", 500)).into_response()
                })
        }
        Ok(None) => {
            // No frame found
            axum::response::Response::builder()
                .status(404)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_string(&ApiResponse::<()>::error(&format!(
                        "No frame found for timestamp {} {}",
                        timestamp.to_rfc3339(),
                        if let Some(tol) = tolerance_seconds {
                            format!("within {}s tolerance", tol)
                        } else {
                            "(exact match)".to_string()
                        }
                    ), 404)).unwrap_or_default()
                ))
                .unwrap_or_else(|_| {
                    Json(ApiResponse::<()>::error("Failed to build 404 response", 500)).into_response()
                })
        }
        Err(e) => {
            Json(ApiResponse::<()>::error(&format!("Database error: {}", e), 500)).into_response()
        }
    }
}

pub async fn api_set_session_keep_flag(
    headers: axum::http::HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
    Query(query): Query<SetKeepSessionQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> axum::response::Response {
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    // Get the database for this specific camera
    let databases = recording_manager.databases.read().await;
    let database = match databases.get(&camera_id) {
        Some(db) => db,
        None => {
            return (axum::http::StatusCode::NOT_FOUND,
                    Json(ApiResponse::<()>::error(&format!("Database not found for camera {}", camera_id), 404)))
                    .into_response();
        }
    };
    
    match database.set_session_keep_flag(session_id, query.keep).await {
        Ok(_) => {
            let data = serde_json::json!({
                "session_id": session_id,
                "keep_session": query.keep,
                "message": format!("Session {} is now {}", session_id, if query.keep { "protected from purging" } else { "eligible for purging" })
            });
            Json(ApiResponse::success(data)).into_response()
        }
        Err(e) => {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             Json(ApiResponse::<()>::error(&format!("Database error: {}", e), 500)))
             .into_response()
        }
    }
}

// DELETE /cam1/control/recordings/sessions/:session_id
pub async fn api_delete_recording_session(
    headers: axum::http::HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> impl IntoResponse {
    // Check authentication
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    let databases = recording_manager.databases.read().await;

    if let Some(database) = databases.get(&camera_id) {
        match database.delete_recording_session(session_id).await {
            Ok(stats) => {
                let data = serde_json::json!({
                    "success": true,
                    "deleted": {
                        "session_id": stats.session_id,
                        "frames": stats.frames_deleted,
                        "mp4_segments": stats.mp4_segments_deleted,
                        "hls_segments": stats.hls_segments_deleted
                    }
                });
                Json(ApiResponse::success(data)).into_response()
            }
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                 Json(ApiResponse::<()>::error(&format!("Delete error: {}", e), 500)))
                    .into_response()
            }
        }
    } else {
        (axum::http::StatusCode::NOT_FOUND,
         Json(ApiResponse::<()>::error("Camera database not found", 404)))
            .into_response()
    }
}

// DELETE /cam1/control/recordings/mp4/segments/:filename
pub async fn api_delete_mp4_segment(
    headers: axum::http::HeaderMap,
    AxumPath(filename): AxumPath<String>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> impl IntoResponse {
    // Check authentication
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    let databases = recording_manager.databases.read().await;

    if let Some(database) = databases.get(&camera_id) {
        match database.delete_mp4_segment_by_filename(&camera_id, &filename).await {
            Ok(size_bytes) => {
                let data = serde_json::json!({
                    "success": true,
                    "deleted": {
                        "filename": filename,
                        "size_bytes": size_bytes
                    }
                });
                Json(ApiResponse::success(data)).into_response()
            }
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                 Json(ApiResponse::<()>::error(&format!("Delete error: {}", e), 500)))
                    .into_response()
            }
        }
    } else {
        (axum::http::StatusCode::NOT_FOUND,
         Json(ApiResponse::<()>::error("Camera database not found", 404)))
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
pub struct BulkDeleteMp4Request {
    pub filenames: Vec<String>,
}

// DELETE /cam1/control/recordings/mp4/segments (with JSON body)
pub async fn api_delete_mp4_segments_bulk(
    headers: axum::http::HeaderMap,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
    Json(request): Json<BulkDeleteMp4Request>,
) -> impl IntoResponse {
    // Check authentication
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    let databases = recording_manager.databases.read().await;

    if let Some(database) = databases.get(&camera_id) {
        match database.delete_mp4_segments_bulk(&camera_id, request.filenames).await {
            Ok(result) => {
                let data = serde_json::json!({
                    "success": true,
                    "deleted_count": result.deleted_count,
                    "total_size_bytes": result.total_size_bytes,
                    "failed": result.failed
                });
                Json(ApiResponse::success(data)).into_response()
            }
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                 Json(ApiResponse::<()>::error(&format!("Delete error: {}", e), 500)))
                    .into_response()
            }
        }
    } else {
        (axum::http::StatusCode::NOT_FOUND,
         Json(ApiResponse::<()>::error("Camera database not found", 404)))
            .into_response()
    }
}

// DELETE /cam1/control/recordings/hls/sessions/:session_id
pub async fn api_delete_hls_segments_by_session(
    headers: axum::http::HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> impl IntoResponse {
    // Check authentication
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    let databases = recording_manager.databases.read().await;

    if let Some(database) = databases.get(&camera_id) {
        match database.delete_hls_segments_by_session(session_id).await {
            Ok(deleted_count) => {
                let data = serde_json::json!({
                    "success": true,
                    "deleted_segments": deleted_count,
                    "session_id": session_id
                });
                Json(ApiResponse::success(data)).into_response()
            }
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                 Json(ApiResponse::<()>::error(&format!("Delete error: {}", e), 500)))
                    .into_response()
            }
        }
    } else {
        (axum::http::StatusCode::NOT_FOUND,
         Json(ApiResponse::<()>::error("Camera database not found", 404)))
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
pub struct DeleteHlsTimerangeQuery {
    pub from: chrono::DateTime<chrono::Utc>,
    pub to: chrono::DateTime<chrono::Utc>,
}

// DELETE /cam1/control/recordings/hls/timerange?from=...&to=...
pub async fn api_delete_hls_segments_by_timerange(
    headers: axum::http::HeaderMap,
    Query(query): Query<DeleteHlsTimerangeQuery>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
) -> impl IntoResponse {
    // Check authentication
    if let Err(response) = check_api_auth(&headers, &camera_config) {
        return response;
    }

    let databases = recording_manager.databases.read().await;

    if let Some(database) = databases.get(&camera_id) {
        match database.delete_hls_segments_by_timerange(&camera_id, query.from, query.to).await {
            Ok(deleted_count) => {
                let data = serde_json::json!({
                    "success": true,
                    "deleted_segments": deleted_count,
                    "from": query.from,
                    "to": query.to
                });
                Json(ApiResponse::success(data)).into_response()
            }
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                 Json(ApiResponse::<()>::error(&format!("Delete error: {}", e), 500)))
                    .into_response()
            }
        }
    } else {
        (axum::http::StatusCode::NOT_FOUND,
         Json(ApiResponse::<()>::error("Camera database not found", 404)))
            .into_response()
    }
}