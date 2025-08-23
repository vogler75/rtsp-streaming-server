use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use tracing::{error, info, warn, debug};
use serde::Deserialize;
use tokio::process::Command;

use crate::{config, recording::RecordingManager};
use crate::AppState;
use crate::database::{HlsPlaylist, HlsSegment};

/// Cleanup old HLS temporary directories on server startup
/// (Only needed for any leftover temp directories from database-based HLS generation)
pub async fn cleanup_old_hls_directories() {
    info!("Starting cleanup of old HLS temporary directories...");
    
    let tmp_dir = "/tmp";
    let mut entries = match tokio::fs::read_dir(tmp_dir).await {
        Ok(entries) => entries,
        Err(e) => {
            warn!("Failed to read /tmp directory for HLS cleanup: {}", e);
            return;
        }
    };
    
    let mut cleanup_count = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with("hls_") {
                let path = entry.path();
                if path.is_dir() {
                    if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                        warn!("Failed to remove old HLS temp directory {:?}: {}", path, e);
                    } else {
                        cleanup_count += 1;
                        info!("Removed old HLS temp directory: {:?}", path);
                    }
                }
            }
        }
    }
    
    info!("HLS temp directory cleanup completed: {} directories removed", cleanup_count);
}

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

/// Extract timestamp from MP4 filename (format: 2025-08-23T17:53:25.522501Z or 2025-08-23T14-30-00Z.mp4)
fn parse_timestamp_from_filename(filename: &str) -> Option<DateTime<Utc>> {
    // First try parsing as exact timestamp (new format without .mp4): 2025-08-23T17:53:25.522501Z
    match DateTime::parse_from_rfc3339(filename) {
        Ok(dt) => {
            debug!("Parsed timestamp from filename '{}' using direct RFC3339 format", filename);
            return Some(dt.with_timezone(&Utc));
        },
        Err(_) => {
            // Continue to try other formats
        }
    }
    
    // Try removing .mp4 extension for backward compatibility
    let timestamp_str = if let Some(stripped) = filename.strip_suffix(".mp4") {
        stripped
    } else {
        filename
    };
    
    // Try parsing the exact timestamp format: 2025-08-23T17:53:25.522501Z
    match DateTime::parse_from_rfc3339(timestamp_str) {
        Ok(dt) => {
            debug!("Parsed timestamp from filename '{}' using RFC3339 format after extension removal", filename);
            return Some(dt.with_timezone(&Utc));
        },
        Err(_) => {
            // Fallback to legacy format parsing: 2025-08-23T14-30-00Z
            debug!("Failed to parse as RFC3339, trying legacy format for filename '{}'", filename);
        }
    }
    
    // Legacy format: Parse the format: 2025-08-23T14-30-00Z
    let formatted_str = timestamp_str.replace('-', ":");
    // This gives us: 2025:08:23T14:30:00Z
    
    // Convert back to standard ISO format: 2025-08-23T14:30:00Z
    let iso_str = formatted_str.replacen(':', "-", 2);
    
    // Parse as RFC3339
    match DateTime::parse_from_rfc3339(&iso_str) {
        Ok(dt) => {
            debug!("Parsed timestamp from filename '{}' using legacy dash-separated format", filename);
            Some(dt.with_timezone(&Utc))
        },
        Err(e) => {
            debug!("Failed to parse timestamp from filename '{}' in all formats: {}", filename, e);
            None
        }
    }
}



// HLS-specific functionality

#[derive(Debug, Deserialize)]
pub struct HlsTimeRangeQuery {
    t1: DateTime<Utc>,
    t2: DateTime<Utc>,
    #[serde(default = "default_hls_segment_duration")]
    segment_duration: u32, // seconds per HLS segment
}

fn default_hls_segment_duration() -> u32 {
    10 // 10 second segments by default
}

pub async fn serve_hls_playlist(
    path: axum::extract::Path<String>, // camera_id
    axum::extract::Query(query): axum::extract::Query<HlsTimeRangeQuery>,
    axum::extract::State(app_state): axum::extract::State<AppState>,
) -> axum::response::Response {
    let camera_id = path.0;
    debug!("Serving HLS playlist: camera_id={}, from={}, to={}", camera_id, query.t1, query.t2);
    
    
    let recording_manager = match app_state.recording_manager {
        Some(ref rm) => rm,
        None => {
            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Recording system not available").into_response();
        }
    };

    // Create a unique playlist ID for this request
    let playlist_id = format!("{}_{}_{}_{}", camera_id, query.t1.timestamp(), query.t2.timestamp(), query.segment_duration);
    
    // First, check if we have a cached HLS playlist in the database
    let camera_streams = recording_manager.databases.read().await;
    let database = match camera_streams.get(&camera_id) {
        Some(db) => db.clone(),
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "Camera database not found").into_response();
        }
    };
    drop(camera_streams);

    // Check for existing cached playlist
    if let Ok(Some(cached_playlist)) = database.get_hls_playlist(&playlist_id).await {
        info!("Reusing cached HLS playlist from database for {}", playlist_id);
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header("Content-Type", "application/vnd.apple.mpegurl")
            .header("Cache-Control", "public, max-age=1800") // Cache for 30 minutes
            .header("Access-Control-Allow-Origin", "*")
            .body(axum::body::Body::from(cached_playlist.playlist_content))
            .unwrap_or_else(|e| {
                error!("Failed to create cached HLS response: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create playlist").into_response()
            });
    }

    // Get camera config to check if HLS storage is enabled
    let camera_configs = app_state.camera_configs.read().await;
    let camera_config = match camera_configs.get(&camera_id) {
        Some(config) => config.clone(),
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "Camera not found").into_response();
        }
    };
    drop(camera_configs);

    // Check if HLS storage is enabled for this camera
    let recording_config = recording_manager.get_recording_config();
    let hls_enabled = camera_config.get_hls_storage_enabled()
        .unwrap_or(recording_config.hls_storage_enabled);
    
    // Also check MP4 storage type to determine priority
    let mp4_storage_type = camera_config.get_mp4_storage_type()
        .unwrap_or(&recording_config.mp4_storage_type);
    let mp4_enabled = mp4_storage_type != &config::Mp4StorageType::Disabled;

    // When both HLS and MP4 are enabled, ALWAYS prefer HLS
    if hls_enabled {
        debug!("HLS storage enabled for camera '{}', checking for pre-generated segments", camera_id);
        
        // Try to find pre-generated HLS segments in database
        match database.get_recording_hls_segments_for_timerange(&camera_id, query.t1, query.t2).await {
            Ok(hls_segments) if !hls_segments.is_empty() => {
                debug!("Found {} pre-generated HLS segments for camera '{}' in time range", hls_segments.len(), camera_id);
                
                // Create HLS playlist from database-stored segments
                let mut playlist_content = String::new();
                playlist_content.push_str("#EXTM3U\n");
                playlist_content.push_str("#EXT-X-VERSION:3\n");
                playlist_content.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", query.segment_duration));
                playlist_content.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
                
                for segment in &hls_segments {
                    playlist_content.push_str(&format!("#EXTINF:{:.3},\n", segment.duration_seconds));
                    // Create segment URL that will be handled by serve_hls_segment_from_database
                    // Use "db" as a placeholder playlist_id for database-stored segments
                    let segment_url = format!("segments/db/recording_{}_{}_{}.ts", 
                                            segment.session_id, 
                                            segment.segment_index,
                                            segment.start_time.timestamp());
                    playlist_content.push_str(&format!("{}\n", segment_url));
                }
                
                playlist_content.push_str("#EXT-X-ENDLIST\n");
                
                debug!("Generated HLS playlist from {} database segments for camera '{}'", hls_segments.len(), camera_id);
                
                return axum::response::Response::builder()
                    .status(axum::http::StatusCode::OK)
                    .header("Content-Type", "application/vnd.apple.mpegurl")
                    .header("Cache-Control", "public, max-age=300") // Cache for 5 minutes
                    .header("Access-Control-Allow-Origin", "*")
                    .body(axum::body::Body::from(playlist_content))
                    .unwrap_or_else(|e| {
                        error!("Failed to create HLS response from database segments: {}", e);
                        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create playlist").into_response()
                    });
            }
            Ok(_) => {
                // When HLS is enabled but no segments found yet - NO FALLBACK
                // Get HLS segment duration to inform the user
                let hls_segment_seconds = camera_config.get_hls_segment_seconds()
                    .unwrap_or(recording_config.hls_segment_seconds);
                info!("No pre-generated HLS segments found for camera '{}' in time range (HLS-only mode, no MP4 fallback)", camera_id);
                let message = format!(
                    "No HLS segments available yet. Recording may have just started. Please wait at least {} seconds for the first segment to be generated, or check if recording is active.",
                    hls_segment_seconds
                );
                return (axum::http::StatusCode::NOT_FOUND, message).into_response();
            }
            Err(e) => {
                // When HLS is enabled and query failed - NO FALLBACK
                error!("Failed to query HLS segments for camera '{}' (HLS-only mode, no MP4 fallback): {}", camera_id, e);
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to retrieve HLS segments").into_response();
            }
        }
    }
    
    // Only proceed with MP4 conversion if HLS is DISABLED
    // When HLS is enabled, we NEVER fall back to MP4
    if hls_enabled {
        // This should never be reached because we return early above when HLS is enabled
        error!("Unexpected code path: HLS is enabled but reached MP4 conversion section for camera '{}'", camera_id);
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error: HLS-only mode but no segments available").into_response();
    }
    
    // HLS is disabled, check if MP4 is enabled
    if !mp4_enabled {
        info!("Neither HLS nor MP4 storage enabled for camera '{}'", camera_id);
        return (axum::http::StatusCode::NOT_FOUND, "No recording storage enabled for this camera").into_response();
    }
    
    info!("Using MP4 segments for camera '{}' (HLS disabled, MP4 enabled)", camera_id);

    // Get all video segments in the time range
    let segments = match recording_manager.list_video_segments_filtered(
        &camera_id,
        Some(query.t1),
        Some(query.t2),
        None, // no reason filter
        1000, // max segments
        "oldest", // chronological order
    ).await {
        Ok(segments) => segments,
        Err(e) => {
            error!("Failed to list video segments: {}", e);
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to list video segments").into_response();
        }
    };

    if segments.is_empty() {
        return (axum::http::StatusCode::NOT_FOUND, "No recordings found in the specified time range").into_response();
    }

    // Get camera config for storage type
    let camera_configs = app_state.camera_configs.read().await;
    let camera_config = match camera_configs.get(&camera_id) {
        Some(config) => config,
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "Camera not found").into_response();
        }
    };

    let storage_type = recording_manager.get_storage_type_for_camera(camera_config);
    drop(camera_configs);
    
    // Create temporary directory for FFmpeg processing
    let temp_dir = format!("/tmp/hls_temp_{}", playlist_id);
    if let Err(e) = tokio::fs::create_dir_all(&temp_dir).await {
        error!("Failed to create temp directory: {}", e);
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create temp directory").into_response();
    }

    // Prepare input files for FFmpeg
    let mut input_files = Vec::new();
    let mut temp_files = Vec::new();
    
    for (i, segment) in segments.iter().enumerate() {
        match storage_type {
            config::Mp4StorageType::Database => {
                // Get the full segment data by timestamp (more efficient)
                let db_segment = match database.get_video_segment_by_time(&camera_id, segment.start_time).await {
                    Ok(Some(seg)) => seg,
                    Ok(None) => {
                        debug!("No MP4 data found for segment at {}", segment.start_time);
                        continue;
                    },
                    Err(e) => {
                        error!("Failed to get segment by time: {}", e);
                        continue;
                    }
                };
                
                if let Some(mp4_data) = db_segment.mp4_data {
                    let temp_path = format!("{}/input_{:03}.mp4", temp_dir, i);
                    if let Err(e) = tokio::fs::write(&temp_path, &mp4_data).await {
                        error!("Failed to write temp file: {}", e);
                        continue;
                    }
                    input_files.push(temp_path.clone());
                    temp_files.push(temp_path);
                } else {
                    warn!("MP4 segment has no data for timestamp: {}", segment.start_time);
                }
            },
            config::Mp4StorageType::Filesystem => {
                if let Some(file_path) = &segment.file_path {
                    input_files.push(file_path.clone());
                }
            },
            config::Mp4StorageType::Disabled => {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return (axum::http::StatusCode::NOT_FOUND, "MP4 storage disabled").into_response();
            }
        }
    }

    if input_files.is_empty() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return (axum::http::StatusCode::NOT_FOUND, "No valid segments found").into_response();
    }

    // Create concat list for FFmpeg
    let concat_list_path = format!("{}/concat_list.txt", temp_dir);
    let concat_content = input_files.iter()
        .map(|path| format!("file '{}'", path))
        .collect::<Vec<String>>()
        .join("\n");
    
    if let Err(e) = tokio::fs::write(&concat_list_path, &concat_content).await {
        error!("Failed to write concat list: {}", e);
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to prepare HLS").into_response();
    }

    // Generate HLS segments using FFmpeg
    let playlist_path = format!("{}/playlist.m3u8", temp_dir);
    let mut hls_cmd = Command::new("ffmpeg");
    hls_cmd.args([
        "-f", "concat",
        "-safe", "0",
        "-i", &concat_list_path,
        "-c:v", "libx264",
        "-c:a", "aac",
        "-preset", "ultrafast",
        "-hls_time", &query.segment_duration.to_string(),
        "-hls_playlist_type", "vod",
        "-hls_segment_type", "mpegts", // Use MPEG-TS segments for better HLS compatibility
        "-hls_segment_filename", &format!("{}/segment_%03d.ts", temp_dir),
        "-start_number", "0",
        &playlist_path,
    ]);
    hls_cmd.stdout(std::process::Stdio::null());
    hls_cmd.stderr(std::process::Stdio::null());

    let ffmpeg_result = hls_cmd.status().await;
    match ffmpeg_result {
        Ok(status) if status.success() => {
            info!("HLS generation completed successfully");
        },
        Ok(status) => {
            error!("FFmpeg failed with exit code: {:?}", status.code());
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate HLS segments").into_response();
        },
        Err(e) => {
            error!("Failed to run FFmpeg: {}", e);
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to run FFmpeg").into_response();
        }
    }

    // Read and store the generated playlist and segments in database
    let playlist_content = match tokio::fs::read_to_string(&playlist_path).await {
        Ok(content) => content,
        Err(e) => {
            error!("Failed to read generated playlist: {}", e);
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to read playlist").into_response();
        }
    };

    // Read and prepare all HLS segments for atomic database storage
    let mut segments = Vec::new();
    let mut segment_index = 0;
    let mut final_playlist_content = String::new();
    
    for line in playlist_content.lines() {
        if line.starts_with("segment_") && line.ends_with(".ts") {
            // Read the segment file
            let segment_path = format!("{}/{}", temp_dir, line);
            match tokio::fs::read(&segment_path).await {
                Ok(segment_data) => {
                    let hls_segment = HlsSegment {
                        playlist_id: playlist_id.clone(),
                        segment_name: line.to_string(),
                        segment_index,
                        segment_data: segment_data.clone(),
                        size_bytes: segment_data.len() as i64,
                        created_at: Utc::now(),
                    };
                    
                    segments.push(hls_segment);
                    
                    // Use relative URLs in playlist for better compatibility with reverse proxies
                    final_playlist_content.push_str(&format!("segments/{}/{}\n", playlist_id, line));
                    segment_index += 1;
                },
                Err(e) => {
                    error!("Failed to read HLS segment file {}: {}", segment_path, e);
                    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                    return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to read HLS segment").into_response();
                }
            }
        } else {
            // Copy other playlist lines as-is
            final_playlist_content.push_str(&format!("{}\n", line));
        }
    }

    // Create the final playlist with complete content
    let expires_at = Utc::now() + chrono::Duration::minutes(30);
    let final_playlist = HlsPlaylist {
        playlist_id: playlist_id.clone(),
        camera_id: camera_id.clone(),
        start_time: query.t1,
        end_time: query.t2,
        segment_duration: query.segment_duration as i32,
        playlist_content: final_playlist_content.clone(),
        created_at: Utc::now(),
        expires_at,
    };

    // Store playlist and segments atomically in a transaction
    if let Err(e) = database.store_hls_playlist_with_segments(&final_playlist, &segments).await {
        error!("Failed to store HLS playlist and segments in database: {}", e);
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to store HLS data").into_response();
    }

    // Cleanup temp directory
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    
    info!("Generated and stored HLS playlist in database with {} segments", segment_index);

    // Schedule cleanup of expired HLS data from database
    let database_cleanup = database.clone();
    tokio::spawn(async move {
        // Wait for cache expiration time + 5 minutes buffer
        tokio::time::sleep(tokio::time::Duration::from_secs(35 * 60)).await;
        if let Err(e) = database_cleanup.cleanup_expired_hls().await {
            warn!("Failed to cleanup expired HLS data: {}", e);
        }
    });

    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("Content-Type", "application/vnd.apple.mpegurl")
        .header("Cache-Control", "public, max-age=1800") // Cache for 30 minutes
        .header("Access-Control-Allow-Origin", "*")
        .body(axum::body::Body::from(final_playlist_content))
        .unwrap_or_else(|e| {
            error!("Failed to create HLS response: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create playlist").into_response()
        })
}

pub async fn serve_hls_segment(
    path: axum::extract::Path<(String, String, String)>, // camera_id, playlist_id, segment_name
    axum::extract::State(app_state): axum::extract::State<AppState>,
) -> axum::response::Response {
    let (camera_id, playlist_id, segment_name) = path.0;
    debug!("Serving HLS segment: camera_id={}, playlist_id={}, segment={}", camera_id, playlist_id, segment_name);
    
    // Validate segment name to prevent path traversal
    if segment_name.contains("..") || segment_name.contains("/") || !segment_name.ends_with(".ts") {
        return (axum::http::StatusCode::BAD_REQUEST, "Invalid segment name").into_response();
    }
    
    let recording_manager = match app_state.recording_manager {
        Some(ref rm) => rm,
        None => {
            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Recording system not available").into_response();
        }
    };
    
    // Get database for this camera
    let camera_streams = recording_manager.databases.read().await;
    let database = match camera_streams.get(&camera_id) {
        Some(db) => db.clone(),
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "Camera database not found").into_response();
        }
    };
    drop(camera_streams);
    
    // Check if this is a database-stored HLS segment from recording
    // These use "db" as the playlist_id and segment names like "recording_1_8_timestamp.ts"
    if (playlist_id == "db" || segment_name.starts_with("recording_")) && segment_name.ends_with(".ts") {
        // Parse the segment name: recording_{session_id}_{segment_index}_{timestamp}.ts
        let parts: Vec<&str> = segment_name.trim_end_matches(".ts").split('_').collect();
        if parts.len() >= 4 && parts[0] == "recording" {
            if let (Ok(session_id), Ok(segment_index)) = (parts[1].parse::<i64>(), parts[2].parse::<i32>()) {
                debug!("Serving database-stored HLS segment from recording_hls table: session_id={}, segment_index={}", session_id, segment_index);
                
                match database.get_recording_hls_segment_by_session_and_index(session_id, segment_index).await {
                    Ok(Some(hls_segment)) => {
                        return axum::response::Response::builder()
                            .status(axum::http::StatusCode::OK)
                            .header("Content-Type", "video/mp2t") // MPEG-TS MIME type
                            .header("Cache-Control", "public, max-age=3600")
                            .header("Access-Control-Allow-Origin", "*")
                            .header("Content-Length", hls_segment.segment_data.len().to_string())
                            .body(axum::body::Body::from(hls_segment.segment_data))
                            .unwrap_or_else(|e| {
                                error!("Failed to create database HLS segment response: {}", e);
                                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create response").into_response()
                            });
                    }
                    Ok(None) => {
                        warn!("Database-stored HLS segment not found: session_id={}, segment_index={}", session_id, segment_index);
                        return (axum::http::StatusCode::NOT_FOUND, "HLS segment not found in database").into_response();
                    }
                    Err(e) => {
                        error!("Failed to get database-stored HLS segment: {}", e);
                        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
                    }
                }
            }
        }
    }
    
    // Only fall back to legacy HLS segment lookup if this is NOT a database-stored segment request
    if playlist_id == "db" {
        // If we get here with playlist_id "db", the segment wasn't found in recording_hls
        return (axum::http::StatusCode::NOT_FOUND, "HLS segment not found in recording_hls table").into_response();
    }
    
    // Fall back to legacy HLS segment lookup (for MP4-converted segments)
    let segment = match database.get_hls_segment(&playlist_id, &segment_name).await {
        Ok(Some(segment)) => segment,
        Ok(None) => {
            return (axum::http::StatusCode::NOT_FOUND, "HLS segment not found").into_response();
        }
        Err(e) => {
            error!("Failed to get HLS segment from database: {}", e);
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };
    
    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("Content-Type", "video/mp2t") // MPEG-TS MIME type
        .header("Cache-Control", "public, max-age=3600")
        .header("Access-Control-Allow-Origin", "*")
        .header("Content-Length", segment.segment_data.len().to_string())
        .body(axum::body::Body::from(segment.segment_data))
        .unwrap_or_else(|e| {
            error!("Failed to create segment response: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create response").into_response()
        })
}

// New reusable MP4 streaming functions for camera-specific endpoints

pub async fn stream_mp4_segment(
    camera_id: &str,
    filename: &str,
    range: Option<(u64, Option<u64>)>,
    camera_config: &config::CameraConfig,
    recording_manager: &RecordingManager,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    
    // Get the storage type for this camera
    let storage_type = recording_manager.get_storage_type_for_camera(camera_config);

    match storage_type {
        config::Mp4StorageType::Database => {
            stream_segment_from_database(camera_id, filename, range, recording_manager).await
        },
        config::Mp4StorageType::Filesystem => {
            let recording_config = recording_manager.get_recording_config();
            stream_segment_from_filesystem(camera_id, filename, range, recording_config).await
        },
        config::Mp4StorageType::Disabled => {
            (axum::http::StatusCode::NOT_FOUND, "MP4 storage disabled for this camera").into_response()
        }
    }
}

async fn stream_segment_from_database(
    camera_id: &str,
    filename: &str,
    range: Option<(u64, Option<u64>)>,
    recording_manager: &RecordingManager,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    
    let camera_streams = recording_manager.databases.read().await;
    let database = match camera_streams.get(camera_id) {
        Some(db) => db.clone(),
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "Camera database not found").into_response();
        }
    };
    drop(camera_streams);

    // Extract timestamp from filename and use efficient time-based lookup
    let segment = if let Some(timestamp) = parse_timestamp_from_filename(filename) {
        match database.get_video_segment_by_time(camera_id, timestamp).await {
            Ok(Some(segment)) => segment,
            Ok(None) => {
                return (axum::http::StatusCode::NOT_FOUND, "Recording not found").into_response();
            }
            Err(e) => {
                error!("Failed to get segment by time: {}", e);
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }
        }
    } else {
        error!("Invalid filename format: {}. Expected format: YYYY-MM-DDTHH:MM:SS.ffffffZ or YYYY-MM-DDTHH-MM-SSZ.mp4", filename);
        return (axum::http::StatusCode::BAD_REQUEST, "Invalid filename format").into_response();
    };

    let file_size = segment.size_bytes as u64;
    let (start, end) = calculate_range(range, file_size);

    let data = match segment.mp4_data {
        Some(blob_data) => blob_data,
        None => {
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

async fn stream_segment_from_filesystem(
    camera_id: &str,
    filename: &str,
    range: Option<(u64, Option<u64>)>,
    recording_config: &config::RecordingConfig,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use chrono::Datelike;
    
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

    let file_path = match file_path { 
        Some(path) => path, 
        None => { 
            return (axum::http::StatusCode::NOT_FOUND, "Recording file not found").into_response(); 
        } 
    };

    let metadata = match tokio::fs::metadata(&file_path).await {
        Ok(metadata) => metadata,
        Err(e) => { 
            error!("Failed to get file metadata: {}", e); 
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to access file").into_response(); 
        }
    };

    let file_size = metadata.len();
    let (start, end) = calculate_range(range, file_size);

    let file_data = match tokio::fs::read(&file_path).await {
        Ok(data) => data,
        Err(e) => { 
            error!("Failed to read file: {}", e); 
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response(); 
        }
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
