use axum::response::IntoResponse;
use chrono::Datelike;
use tracing::error;

use crate::{config, recording::RecordingManager};
use crate::AppState;

pub fn parse_range_header(range_header: Option<&axum::http::HeaderValue>) -> Option<(u64, Option<u64>)> {
    if let Some(range_value) = range_header {
        if let Ok(range_str) = range_value.to_str() {
            if let Some(range_part) = range_str.strip_prefix("bytes=") {
                if let Some(dash_pos) = range_part.find('-') {
                    let start_str = &range_part[..dash_pos];
                    let end_str = &range_part[dash_pos + 1..];
                    if let Ok(start) = start_str.parse::<u64>() {
                        let end = if end_str.is_empty() { None } else { end_str.parse::<u64>().ok() };
                        return Some((start, end));
                    }
                }
            }
        }
    }
    None
}

pub fn calculate_range(range: Option<(u64, Option<u64>)>, file_size: u64) -> (u64, u64) {
    match range {
        Some((start, end)) => {
            let start = start.min(file_size.saturating_sub(1));
            let end = end.unwrap_or(file_size.saturating_sub(1)).min(file_size.saturating_sub(1));
            (start, end)
        }
        None => (0, file_size.saturating_sub(1)),
    }
}

pub async fn stream_mp4_recording(
    path: axum::extract::Path<(String, String)>, // camera_id, filename
    headers: axum::http::HeaderMap,
    axum::extract::State(app_state): axum::extract::State<AppState>,
) -> axum::response::Response {
    let (camera_id, filename) = path.0;
    tracing::info!("Streaming MP4 recording: camera_id={}, filename={}", camera_id, filename);
    let range = parse_range_header(headers.get("range"));

    let recording_manager = match app_state.recording_manager {
        Some(ref rm) => rm,
        None => {
            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Recording system not available").into_response();
        }
    };

    let camera_configs = app_state.camera_configs.read().await;
    let camera_config = match camera_configs.get(&camera_id) {
        Some(config) => config,
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "Camera not found").into_response();
        }
    };

    let storage_type = recording_manager.get_storage_type_for_camera(camera_config);

    match storage_type {
        config::Mp4StorageType::Database => {
            stream_from_database(&camera_id, &filename, range, recording_manager).await
        },
        config::Mp4StorageType::Filesystem => {
            stream_from_filesystem(&camera_id, &filename, range, app_state.recording_config.as_ref().unwrap()).await
        },
        config::Mp4StorageType::Disabled => {
            (axum::http::StatusCode::NOT_FOUND, "MP4 storage disabled for this camera").into_response()
        }
    }
}

async fn stream_from_database(
    camera_id: &str,
    filename: &str,
    range: Option<(u64, Option<u64>)>,
    recording_manager: &RecordingManager,
) -> axum::response::Response {
    let camera_streams = recording_manager.databases.read().await;
    let database = match camera_streams.get(camera_id) {
        Some(db) => db.clone(),
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "Camera database not found").into_response();
        }
    };
    drop(camera_streams);

    let segment = match database.get_video_segment_by_filename(camera_id, filename).await {
        Ok(Some(segment)) => {
            tracing::info!("Found segment: size_bytes={}, has_mp4_data={}", 
                segment.size_bytes, segment.mp4_data.is_some());
            segment
        },
        Ok(None) => {
            tracing::warn!("Recording not found for camera_id={}, filename={}", camera_id, filename);
            return (axum::http::StatusCode::NOT_FOUND, "Recording not found").into_response();
        }
        Err(e) => {
            error!("Failed to get segment info: {}", e);
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let file_size = segment.size_bytes as u64;
    let (start, end) = calculate_range(range, file_size);

    let data = match segment.mp4_data {
        Some(blob_data) => {
            tracing::info!("MP4 data retrieved from database: {} bytes", blob_data.len());
            // Log first few bytes to check if it's valid MP4
            if blob_data.len() >= 8 {
                let header = &blob_data[0..8];
                tracing::info!("MP4 header bytes: {:?}", header);
                // MP4 files should start with ftyp box after 4 bytes of size
                if header[4..8] == *b"ftyp" {
                    tracing::info!("Valid MP4 header detected");
                } else {
                    tracing::warn!("Invalid MP4 header - expected 'ftyp' at offset 4");
                }
            }
            blob_data
        },
        None => {
            tracing::error!("Segment data not found in database");
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Segment data not found in database").into_response();
        }
    };

    let chunk = if start == 0 && end == file_size.saturating_sub(1) {
        data
    } else {
        data.get(start as usize..=(end as usize)).unwrap_or(&data).to_vec()
    };

    let response = axum::response::Response::builder()
        .status(if range.is_some() { axum::http::StatusCode::PARTIAL_CONTENT } else { axum::http::StatusCode::OK })
        .header("Content-Type", "video/mp4")
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", chunk.len().to_string())
        .header("Cache-Control", "public, max-age=3600");

    let response = if range.is_some() {
        response.header("Content-Range", format!("bytes {}-{}/{}", start, end, file_size))
    } else {
        response
    };

    match response.body(axum::body::Body::from(chunk)) {
        Ok(response) => response,
        Err(e) => {
            error!("Failed to create response: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create response").into_response()
        }
    }
}

async fn stream_from_filesystem(
    camera_id: &str,
    filename: &str,
    range: Option<(u64, Option<u64>)>,
    recording_config: &config::RecordingConfig,
) -> axum::response::Response {
    let base_path = std::path::PathBuf::from(&recording_config.database_path);

    let mut potential_paths = vec![ base_path.join(camera_id).join(filename) ];

    let now = chrono::Utc::now();
    for year in (now.year()-1)..=(now.year()) {
        for month in 1..=12 {
            for day in 1..=31 {
                let path = base_path.join(camera_id)
                    .join(year.to_string())
                    .join(format!("{:02}", month))
                    .join(format!("{:02}", day))
                    .join(filename);
                potential_paths.push(path);
            }
        }
    }

    let mut file_path = None;
    for path in potential_paths {
        if path.exists() { file_path = Some(path); break; }
    }

    let file_path = match file_path { Some(path) => path, None => { return (axum::http::StatusCode::NOT_FOUND, "Recording file not found").into_response(); } };

    let metadata = match tokio::fs::metadata(&file_path).await {
        Ok(metadata) => metadata,
        Err(e) => { error!("Failed to get file metadata: {}", e); return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to access file").into_response(); }
    };

    let file_size = metadata.len();
    let (start, end) = calculate_range(range, file_size);

    let file_data = match tokio::fs::read(&file_path).await {
        Ok(data) => data,
        Err(e) => { error!("Failed to read file: {}", e); return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response(); }
    };

    let chunk = file_data.get(start as usize..=(end as usize)).unwrap_or(&file_data).to_vec();

    let response = axum::response::Response::builder()
        .status(if range.is_some() { axum::http::StatusCode::PARTIAL_CONTENT } else { axum::http::StatusCode::OK })
        .header("Content-Type", "video/mp4")
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", chunk.len().to_string())
        .header("Cache-Control", "public, max-age=3600");

    let response = if range.is_some() {
        response.header("Content-Range", format!("bytes {}-{}/{}", start, end, file_size))
    } else { response };

    match response.body(axum::body::Body::from(chunk)) {
        Ok(response) => response,
        Err(e) => { error!("Failed to create response: {}", e); (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create response").into_response() }
    }
}
