use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::broadcast;
use tracing::{info, warn, error};
use std::fs::File;
use std::io::BufReader;
use axum::response::IntoResponse;
use axum::extract::{State, Path, Query};
use axum::Json;
use clap::Parser;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

mod config;
mod errors;
mod builders;
mod rtsp_client;
mod websocket;
mod transcoder;
mod video_stream;
mod mqtt;
mod database;
mod recording;
mod control;
mod utils;

use config::Config;
use errors::{Result, StreamError};
use video_stream::VideoStream;
use websocket::websocket_handler;
use mqtt::{MqttPublisher, MqttHandle};
use database::SqliteDatabase;
use recording::{RecordingManager, RecordingConfig};
use control::handle_control_websocket;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: String,
}

#[derive(Clone)]
struct CameraStreamInfo {
    camera_id: String,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
    recording_manager: Option<Arc<RecordingManager>>,
}

#[derive(Clone)]
struct AppState {
    camera_streams: Arc<tokio::sync::RwLock<HashMap<String, CameraStreamInfo>>>,
    mqtt_handle: Option<MqttHandle>,
    start_time: std::time::Instant,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("rtsp_streaming_server=debug,info")
        .init();

    // Parse command line arguments
    let args = Args::parse();

    let config = match Config::load(&args.config) {
        Ok(cfg) => {
            info!("Loaded configuration from {}", args.config);
            cfg
        }
        Err(e) => {
            warn!("Could not load configuration from {}: {}", args.config, e);
            info!("Using default configuration");
            Config::default()
        }
    };

    info!("Starting RTSP streaming server on {}:{}", config.server.host, config.server.port);

    // Initialize MQTT if enabled
    let mqtt_handle: Option<MqttHandle> = if let Some(mqtt_config) = config.mqtt.clone() {
        if mqtt_config.enabled {
            info!("Initializing MQTT connection to {}", mqtt_config.broker_url);
            match MqttPublisher::new(mqtt_config).await {
                Ok(publisher) => {
                    match publisher.start().await {
                        Ok(handle) => {
                            info!("MQTT publisher started successfully");
                            Some(handle)
                        }
                        Err(e) => {
                            error!("Failed to start MQTT publisher: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to create MQTT publisher: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Initialize recording manager if enabled
    let recording_manager: Option<Arc<RecordingManager>> = if let Some(recording_config) = &config.recording {
        if recording_config.enabled {
            info!("Initializing recording system with database directory: {}", recording_config.database_path);
            
            // Ensure the database directory exists
            if let Err(e) = std::fs::create_dir_all(&recording_config.database_path) {
                error!("Failed to create database directory '{}': {}", recording_config.database_path, e);
                None
            } else {
                let recording_config_internal = RecordingConfig {
                    max_frame_size: recording_config.max_frame_size.unwrap_or(10 * 1024 * 1024),
                };
                
                match RecordingManager::new(recording_config_internal).await {
                    Ok(manager) => {
                        info!("Recording system initialized successfully");
                        let manager = Arc::new(manager);
                            
                            // Start cleanup task if max_recording_age is configured
                            if let Some(max_age_str) = &recording_config.max_recording_age {
                                if !max_age_str.is_empty() && max_age_str != "0" {
                                    match utils::parse_duration(max_age_str) {
                                        Ok(max_age_duration) => {
                                            let cleanup_interval_hours = recording_config.cleanup_interval_hours.unwrap_or(1);
                                            info!(
                                                "Starting recording cleanup task: removing recordings older than {} every {} hour(s)",
                                                max_age_str, cleanup_interval_hours
                                            );
                                            
                                            let manager_clone = manager.clone();
                                            let cameras_config = config.cameras.clone();
                                            let global_max_age = max_age_duration;
                                            
                                            tokio::spawn(async move {
                                                let mut interval = tokio::time::interval(
                                                    tokio::time::Duration::from_secs(cleanup_interval_hours * 3600)
                                                );
                                                
                                                loop {
                                                    interval.tick().await;
                                                    
                                                    // Process each camera's cleanup
                                                    for (camera_id, camera_config) in &cameras_config {
                                                        // Determine the max age for this camera
                                                        let max_age = if let Some(camera_max_age_str) = &camera_config.max_recording_age {
                                                            if !camera_max_age_str.is_empty() && camera_max_age_str != "0" {
                                                                match utils::parse_duration(camera_max_age_str) {
                                                                    Ok(duration) => duration,
                                                                    Err(e) => {
                                                                        error!("Invalid max_recording_age for camera '{}': {}", camera_id, e);
                                                                        continue;
                                                                    }
                                                                }
                                                            } else {
                                                                continue; // Skip if camera has explicit "0" or empty
                                                            }
                                                        } else {
                                                            global_max_age
                                                        };
                                                        
                                                        let older_than = chrono::Utc::now() - max_age;
                                                        
                                                        match manager_clone.cleanup_old_recordings(Some(camera_id), older_than).await {
                                                            Ok(deleted) => {
                                                                if deleted > 0 {
                                                                    info!("Cleaned up {} completed sessions and old frames for camera '{}'", deleted, camera_id);
                                                                }
                                                            }
                                                            Err(e) => {
                                                                error!("Failed to cleanup recordings for camera '{}': {}", camera_id, e);
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                        Err(e) => {
                                            error!("Invalid max_recording_age configuration: {}", e);
                                        }
                                    }
                                }
                            }
                            
                        Some(manager)
                    }
                    Err(e) => {
                        error!("Failed to initialize recording manager: {}", e);
                        None
                    }
                }
            }
        } else {
            info!("Recording system disabled in configuration");
            None
        }
    } else {
        None
    };

    // Create video streams for each camera
    let mut camera_streams: HashMap<String, CameraStreamInfo> = HashMap::new();
    
    for (camera_id, camera_config) in config.cameras.clone() {
        // Check if camera is enabled (default to true if not specified)
        let is_enabled = camera_config.enabled.unwrap_or(true);
        if !is_enabled {
            info!("Camera '{}' is disabled, skipping", camera_id);
            continue;
        }
        
        info!("Configuring camera '{}' on path '{}'...", camera_id, camera_config.path);
        
        match VideoStream::new(
            camera_id.clone(),
            camera_config.clone(),
            &config.transcoding,
            mqtt_handle.clone(),
        ).await {
            Ok(video_stream) => {
                // Create database for this camera if recording is enabled
                if let Some(ref recording_manager_ref) = recording_manager {
                    if let Some(recording_config) = &config.recording {
                        let camera_db_path = format!("{}/{}.db", recording_config.database_path, camera_id);
                        info!("Creating database for camera '{}' at '{}'", camera_id, camera_db_path);
                        
                        match SqliteDatabase::new(&camera_db_path).await {
                            Ok(database) => {
                                let database: Arc<dyn database::DatabaseProvider> = Arc::new(database);
                                if let Err(e) = recording_manager_ref.add_camera_database(&camera_id, database).await {
                                    error!("Failed to add database for camera '{}': {}", camera_id, e);
                                } else {
                                    info!("Database created successfully for camera '{}'", camera_id);
                                }
                            }
                            Err(e) => {
                                error!("Failed to create database for camera '{}': {}", camera_id, e);
                            }
                        }
                    }
                }

                // Store the camera stream info for this camera's path
                camera_streams.insert(camera_config.path.clone(), CameraStreamInfo {
                    camera_id: camera_id.clone(),
                    frame_sender: video_stream.frame_sender.clone(),
                    mqtt_handle: mqtt_handle.clone(),
                    camera_config: camera_config.clone(),
                    recording_manager: recording_manager.clone(),
                });
                
                // Start the video stream
                video_stream.start().await;
                info!("Started camera '{}' on path '{}'" , camera_id, camera_config.path);
            }
            Err(e) => {
                error!("Failed to create video stream for camera '{}': {}", camera_id, e);
            }
        }
    }
    
    if camera_streams.is_empty() {
        error!("No cameras configured or all failed to initialize");
        return Err(StreamError::config("No cameras available"));
    }

    // Restart active recordings if recording manager is available
    if let Some(ref recording_manager) = recording_manager {
        info!("Checking for active recordings to restart...");
        
        // Create a map of camera_id -> frame_sender for the restart method
        let mut camera_frame_senders: HashMap<String, Arc<broadcast::Sender<bytes::Bytes>>> = HashMap::new();
        
        for stream_info in camera_streams.values() {
            camera_frame_senders.insert(
                stream_info.camera_id.clone(),
                stream_info.frame_sender.clone()
            );
        }
        
        // Restart active recordings
        if let Err(e) = recording_manager.restart_active_recordings_at_startup(&camera_frame_senders).await {
            error!("Failed to restart active recordings at startup: {}", e);
        }
    }

    let cors_layer = if let Some(origin) = &config.server.cors_allow_origin {
        if origin == "*" {
            tower_http::cors::CorsLayer::permissive()
        } else {
            match origin.parse::<axum::http::HeaderValue>() {
                Ok(origin_header) => {
                    tower_http::cors::CorsLayer::new()
                        .allow_origin(origin_header)
                        .allow_methods(tower_http::cors::Any)
                        .allow_headers(tower_http::cors::Any)
                }
                Err(_) => {
                    warn!("Invalid CORS origin '{}', falling back to permissive", origin);
                    tower_http::cors::CorsLayer::permissive()
                }
            }
        }
    } else {
        tower_http::cors::CorsLayer::permissive()
    };

    // Collect camera streams for API access
    
    // Convert camera_streams to be accessible by camera_id
    let mut camera_streams_by_id: HashMap<String, CameraStreamInfo> = HashMap::new();
    let camera_streams_by_path = camera_streams.clone();
    for (_, stream_info) in &camera_streams {
        camera_streams_by_id.insert(stream_info.camera_id.clone(), stream_info.clone());
    }
    
    let app_state = AppState {
        camera_streams: Arc::new(tokio::sync::RwLock::new(camera_streams_by_id)),
        mqtt_handle: mqtt_handle.clone(),
        start_time: std::time::Instant::now(),
    };

    // Build router with camera paths
    let mut app = axum::Router::new()
        .route("/dashboard", axum::routing::get(dashboard_handler))
        .nest_service("/static", tower_http::services::ServeDir::new("static"));
    
    // Add routes for each camera (both stream and control endpoints)
    for (path, stream_info) in camera_streams_by_path {
        info!("Adding routes for camera at path: {}", path);
        
        // Stream endpoint: /<camera_path>/stream
        let stream_path = format!("{}/stream", path);
        let stream_info_clone = stream_info.clone();
        app = app.route(&stream_path, axum::routing::get(
            move |ws, query, addr| camera_stream_handler(
                ws, query, addr, 
                stream_info_clone.frame_sender.clone(), 
                stream_info_clone.camera_id.clone(), 
                stream_info_clone.mqtt_handle.clone(), 
                stream_info_clone.camera_config.clone()
            )
        ));

        // Control endpoint: /<camera_path>/control
        let control_path = format!("{}/control", path);
        let control_info_clone = stream_info.clone();
        app = app.route(&control_path, axum::routing::get(
            move |headers, ws, query, addr| camera_control_handler(
                headers, ws, query, addr,
                control_info_clone.frame_sender.clone(),
                control_info_clone.camera_id.clone(),
                control_info_clone.mqtt_handle.clone(),
                control_info_clone.camera_config.clone(),
                control_info_clone.recording_manager.clone()
            )
        ));

        // Live endpoint: /<camera_path>/live (WebSocket only)
        let live_path = format!("{}/live", path);
        let live_info_clone = stream_info.clone();
        app = app.route(&live_path, axum::routing::get(
            move |ws, query, addr| camera_live_handler(
                ws, query, addr, 
                live_info_clone.frame_sender.clone(), 
                live_info_clone.camera_id.clone(), 
                live_info_clone.mqtt_handle.clone(), 
                live_info_clone.camera_config.clone()
            )
        ));

        // Camera page endpoint: /<camera_path> serves test.html
        app = app.route(&path, axum::routing::get(serve_test_page));
        
        // Test endpoint: /<camera_path>/test serves test.html
        let test_path = format!("{}/test", path);
        app = app.route(&test_path, axum::routing::get(serve_test_page));

        // REST API endpoints: /<camera_path>/control/*
        if stream_info.recording_manager.is_some() {
            let api_info = stream_info.clone();
            
            // Start recording
            let start_recording_path = format!("{}/control/recording/start", path);
            let start_info = api_info.clone();
            app = app.route(&start_recording_path, axum::routing::post(
                move |headers, json| api_start_recording(
                    headers,
                    json,
                    start_info.camera_id.clone(),
                    start_info.camera_config.clone(),
                    start_info.recording_manager.clone().unwrap(),
                    start_info.frame_sender.clone()
                )
            ));

            // Stop recording
            let stop_recording_path = format!("{}/control/recording/stop", path);
            let stop_info = api_info.clone();
            app = app.route(&stop_recording_path, axum::routing::post(
                move |headers| api_stop_recording(
                    headers,
                    stop_info.camera_id.clone(),
                    stop_info.camera_config.clone(),
                    stop_info.recording_manager.clone().unwrap()
                )
            ));

            // List recordings
            let list_recordings_path = format!("{}/control/recordings", path);
            let list_info = api_info.clone();
            app = app.route(&list_recordings_path, axum::routing::get(
                move |headers, query| api_list_recordings(
                    headers,
                    query,
                    list_info.camera_id.clone(),
                    list_info.camera_config.clone(),
                    list_info.recording_manager.clone().unwrap()
                )
            ));

            // Get recorded frames
            let frames_path = format!("{}/control/recordings/:session_id/frames", path);
            let frames_info = api_info.clone();
            app = app.route(&frames_path, axum::routing::get(
                move |headers, path, query| api_get_recorded_frames(
                    headers,
                    path,
                    query,
                    frames_info.camera_config.clone(),
                    frames_info.recording_manager.clone().unwrap()
                )
            ));

            // Get active recording
            let active_recording_path = format!("{}/control/recording/active", path);
            let active_info = api_info.clone();
            app = app.route(&active_recording_path, axum::routing::get(
                move |headers| api_get_active_recording(
                    headers,
                    active_info.camera_id.clone(),
                    active_info.camera_config.clone(),
                    active_info.recording_manager.clone().unwrap()
                )
            ));

            // Get recording database size
            let size_recording_path = format!("{}/control/recording/size", path);
            let size_info = api_info.clone();
            app = app.route(&size_recording_path, axum::routing::get(
                move |headers| api_get_recording_size(
                    headers,
                    size_info.camera_id.clone(),
                    size_info.camera_config.clone(),
                    size_info.recording_manager.clone().unwrap()
                )
            ));
        }
    }
    
    // Add API endpoints with captured state
    let api_state = app_state.clone();
    app = app.route("/api/status", axum::routing::get(move || {
        let state = api_state.clone();
        async move {
            let uptime_secs = state.start_time.elapsed().as_secs();
            let camera_streams = state.camera_streams.read().await;
            let total_cameras = camera_streams.len();
            drop(camera_streams); // Release lock before await
            
            // Calculate total clients by summing clients from all cameras
            // Note: clients_connected includes WebSocket clients + internal systems (recording + control)
            // Each camera typically shows +2 clients at startup (recording=1, control=1)
            let mut total_clients = 0;
            if let Some(mqtt_handle) = &state.mqtt_handle {
                let all_camera_statuses = mqtt_handle.get_all_camera_status().await;
                total_clients = all_camera_statuses.values()
                    .map(|status| status.clients_connected)
                    .sum();
            }
            
            let status = serde_json::json!({
                "uptime_secs": uptime_secs,
                "total_clients": total_clients,
                "total_cameras": total_cameras
            });
            
            Json(ApiResponse::success(status)).into_response()
        }
    }));
    
    let api_state2 = app_state.clone();
    app = app.route("/api/cameras", axum::routing::get(move || {
        let state = api_state2.clone();
        async move {
            let camera_streams = state.camera_streams.read().await;
            
            let mut cameras = Vec::new();
            
            // Collect camera data first and sort by camera ID
            let mut camera_data: Vec<(String, String, bool)> = camera_streams.iter()
                .map(|(id, info)| (id.clone(), info.camera_config.path.clone(), info.camera_config.token.is_some()))
                .collect();
            camera_data.sort_by(|a, b| a.0.cmp(&b.0));
            drop(camera_streams); // Release lock before async operations
            
            // Get all camera statuses at once for efficiency
            let all_camera_statuses = if let Some(mqtt_handle) = &state.mqtt_handle {
                mqtt_handle.get_all_camera_status().await
            } else {
                HashMap::new()
            };
            
            for (camera_id, camera_path, token_required) in camera_data {
                let camera_status = if let Some(real_status) = all_camera_statuses.get(&camera_id) {
                    serde_json::json!({
                        "id": real_status.id,
                        "path": camera_path,
                        "connected": real_status.connected,
                        "capture_fps": real_status.capture_fps,
                        "clients_connected": real_status.clients_connected,
                        "last_frame_time": real_status.last_frame_time,
                        "ffmpeg_running": real_status.ffmpeg_running,
                        "duplicate_frames": real_status.duplicate_frames,
                        "token_required": token_required
                    })
                } else {
                    serde_json::json!({
                        "id": camera_id,
                        "path": camera_path,
                        "connected": false,
                        "capture_fps": 0.0,
                        "clients_connected": 0,
                        "last_frame_time": null,
                        "ffmpeg_running": false,
                        "duplicate_frames": 0,
                        "token_required": token_required
                    })
                };
                
                cameras.push(camera_status);
            }
            
            let response = serde_json::json!({
                "cameras": cameras,
                "count": cameras.len()
            });
            
            Json(ApiResponse::success(response)).into_response()
        }
    }));
    
    app = app.layer(cors_layer);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    
    // Check if TLS is enabled
    if let Some(tls_config) = &config.server.tls {
        if tls_config.enabled {
            info!("Starting HTTPS server on {}", addr);
            start_https_server(app, &addr, tls_config).await?;
        } else {
            info!("Starting HTTP server on {}", addr);
            start_http_server(app, &addr).await?;
        }
    } else {
        info!("Starting HTTP server on {}", addr);
        start_http_server(app, &addr).await?;
    }

    Ok(())
}

async fn serve_control_page() -> axum::response::Html<String> {
    let html = include_str!("../static/control.html").to_string();
    axum::response::Html(html)
}

async fn serve_stream_page() -> axum::response::Html<String> {
    let html = include_str!("../static/stream.html").to_string();
    axum::response::Html(html)
}

async fn serve_test_page(
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    // Check if 'full' parameter is present
    let is_full_mode = query.contains_key("full");
    serve_test_with_mode(is_full_mode).await.into_response()
}

async fn serve_test_with_mode(is_full_mode: bool) -> axum::response::Html<String> {
    let mut html = include_str!("../static/test.html").to_string();
    
    if is_full_mode {
        // Inject JavaScript to enable full mode
        let full_mode_script = r#"
        <script>
            // Enable full mode by setting a flag before the main script runs
            window.FULL_MODE = true;
        </script>"#;
        
        // Insert the script before the closing </head> tag
        html = html.replace("</head>", &format!("{}</head>", full_mode_script));
    }
    
    axum::response::Html(html)
}



async fn dashboard_handler() -> axum::response::Html<String> {
    let html = include_str!("../static/dashboard.html").to_string();
    axum::response::Html(html)
}


async fn camera_live_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
    addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
) -> axum::response::Response {
    match ws {
        Some(ws_upgrade) => {
            // Check token authentication before upgrading WebSocket
            if let Some(expected_token) = &camera_config.token {
                // Get token from query parameters
                if let Some(provided_token) = query.get("token") {
                    if provided_token == expected_token {
                        info!("Token authentication successful for camera {}", camera_id);
                    } else {
                        warn!("Invalid token provided for camera {}", camera_id);
                        return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response();
                    }
                } else {
                    warn!("Missing token for camera {} that requires authentication", camera_id);
                    return (axum::http::StatusCode::UNAUTHORIZED, "Missing token").into_response();
                }
            }
            
            if let Some(connect_info) = addr {
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            } else {
                // Fallback with unknown IP
                let fallback_addr = "127.0.0.1:0".parse().unwrap();
                let connect_info = axum::extract::ConnectInfo(fallback_addr);
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            }
        },
        None => {
            // No WebSocket upgrade - return error for /live endpoint
            (axum::http::StatusCode::BAD_REQUEST, "Live endpoint only accepts WebSocket connections").into_response()
        }
    }
}

async fn camera_stream_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
    addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
) -> axum::response::Response {
    match ws {
        Some(ws_upgrade) => {
            // Check token authentication before upgrading WebSocket
            if let Some(expected_token) = &camera_config.token {
                // Get token from query parameters
                if let Some(provided_token) = query.get("token") {
                    if provided_token == expected_token {
                        info!("Token authentication successful for camera {}", camera_id);
                    } else {
                        warn!("Invalid token provided for camera {}", camera_id);
                        return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response();
                    }
                } else {
                    warn!("Missing token for camera {} that requires authentication", camera_id);
                    return (axum::http::StatusCode::UNAUTHORIZED, "Missing token").into_response();
                }
            }
            
            if let Some(connect_info) = addr {
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            } else {
                // Fallback with unknown IP
                let fallback_addr = "127.0.0.1:0".parse().unwrap();
                let connect_info = axum::extract::ConnectInfo(fallback_addr);
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            }
        },
        None => {
            // Serve the dedicated video stream page
            serve_stream_page().await.into_response()
        }
    }
}

async fn camera_control_handler(
    headers: axum::http::HeaderMap,
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
    _addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    camera_id: String,
    _mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
    recording_manager: Option<Arc<RecordingManager>>,
) -> axum::response::Response {
    match ws {
        Some(ws_upgrade) => {
            // Check token authentication before upgrading WebSocket
            if let Some(expected_token) = &camera_config.token {
                let mut token_valid = false;
                
                // First try Authorization header for Bearer token
                if let Some(auth_header) = headers.get("authorization") {
                    if let Ok(auth_str) = auth_header.to_str() {
                        if let Some(token) = auth_str.strip_prefix("Bearer ") {
                            if token == expected_token {
                                info!("Bearer token authentication successful for camera {} control", camera_id);
                                token_valid = true;
                            } else {
                                warn!("Invalid Bearer token provided for camera {} control", camera_id);
                                return (axum::http::StatusCode::UNAUTHORIZED, "Invalid Bearer token").into_response();
                            }
                        } else {
                            warn!("Authorization header does not contain Bearer token for camera {} control", camera_id);
                            return (axum::http::StatusCode::UNAUTHORIZED, "Authorization header must contain Bearer token").into_response();
                        }
                    } else {
                        warn!("Invalid Authorization header format for camera {} control", camera_id);
                        return (axum::http::StatusCode::UNAUTHORIZED, "Invalid Authorization header format").into_response();
                    }
                }
                
                // If no valid Authorization header, try query parameter (fallback for WebSocket clients)
                if !token_valid {
                    if let Some(provided_token) = query.get("token") {
                        if provided_token == expected_token {
                            info!("Query parameter token authentication successful for camera {} control", camera_id);
                            token_valid = true;
                        } else {
                            warn!("Invalid query parameter token provided for camera {} control", camera_id);
                            return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response();
                        }
                    }
                }
                
                // If neither authentication method provided valid token
                if !token_valid {
                    warn!("Missing or invalid authentication for camera {} control that requires authentication", camera_id);
                    return (axum::http::StatusCode::UNAUTHORIZED, "Missing or invalid authentication - provide Bearer token in Authorization header or ?token= query parameter").into_response();
                }
            }

            // Check if recording is enabled
            if recording_manager.is_none() {
                warn!("Recording not enabled, control endpoint unavailable");
                return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Recording system not enabled").into_response();
            }
            
            let client_id = uuid::Uuid::new_v4().to_string();
            let socket = ws_upgrade.on_upgrade(move |socket| {
                handle_control_websocket(
                    socket,
                    camera_id,
                    client_id,
                    recording_manager.unwrap(),
                    frame_sender,
                )
            });
            socket.into_response()
        },
        None => {
            // Serve the control HTML page
            serve_control_page().await.into_response()
        }
    }
}

// API Request/Response structs
#[derive(Deserialize)]
struct StartRecordingRequest {
    reason: Option<String>,
}

#[derive(Deserialize)]
struct GetRecordingsQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct GetFramesQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct ApiResponse<T> {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<u16>,
}

impl<T> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            status: "success".to_string(),
            data: Some(data),
            error: None,
            code: None,
        }
    }

    fn error(message: &str, code: u16) -> ApiResponse<()> {
        ApiResponse {
            status: "error".to_string(),
            data: None,
            error: Some(message.to_string()),
            code: Some(code),
        }
    }
}

// Authentication helper
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

// API Handlers
async fn api_start_recording(
    headers: axum::http::HeaderMap,
    Json(request): Json<StartRecordingRequest>,
    camera_id: String,
    camera_config: config::CameraConfig,
    recording_manager: Arc<RecordingManager>,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
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

async fn api_stop_recording(
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

async fn api_list_recordings(
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
        Ok(recordings) => {
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

async fn api_get_recorded_frames(
    headers: axum::http::HeaderMap,
    Path(session_id): Path<i64>,
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

async fn api_get_active_recording(
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

async fn api_get_recording_size(
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

async fn start_http_server(app: axum::Router, addr: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("HTTP server listening on http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn start_https_server(app: axum::Router, addr: &str, tls_cfg: &config::TlsConfig) -> Result<()> {
    // Load TLS certificates
    let cert_file = File::open(&tls_cfg.cert_path)
        .map_err(|e| StreamError::server(format!("Failed to open certificate file '{}': {}", tls_cfg.cert_path, e)))?;
    let key_file = File::open(&tls_cfg.key_path)
        .map_err(|e| StreamError::server(format!("Failed to open private key file '{}': {}", tls_cfg.key_path, e)))?;

    let mut cert_reader = BufReader::new(cert_file);
    let mut key_reader = BufReader::new(key_file);

    // Parse certificate and key
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .map_err(|e| StreamError::server(format!("Failed to parse certificate: {}", e)))?
        .into_iter()
        .map(rustls::Certificate)
        .collect();
    
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
        .map_err(|e| StreamError::server(format!("Failed to parse private key: {}", e)))?;
    
    if keys.is_empty() {
        // Try RSA private keys if PKCS8 fails
        let mut key_reader = BufReader::new(File::open(&tls_cfg.key_path)?);
        keys = rustls_pemfile::rsa_private_keys(&mut key_reader)
            .map_err(|e| StreamError::server(format!("Failed to parse RSA private key: {}", e)))?;
    }
    
    let private_key = keys.into_iter().next()
        .ok_or_else(|| StreamError::server("No private key found in key file"))?;

    // Create TLS configuration
    let rustls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, rustls::PrivateKey(private_key))
        .map_err(|e| StreamError::server(format!("Failed to create TLS config: {}", e)))?;

    info!("HTTPS server listening on https://{}", addr);
    info!("Certificate: {}", tls_cfg.cert_path);
    info!("Private key: {}", tls_cfg.key_path);

    // Start HTTPS server
    let tls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(rustls_config));
    axum_server::bind_rustls(addr.parse()?, tls_config)
        .serve(app.into_make_service())
        .await
        .map_err(|e| StreamError::server(format!("HTTPS server error: {}", e)))?;

    Ok(())
}