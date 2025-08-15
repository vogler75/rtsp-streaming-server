use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn, error};
use std::fs::File;
use std::io::BufReader;
use axum::response::IntoResponse;
use axum::extract::State;
use clap::Parser;

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

use config::Config;
use errors::{Result, StreamError};
use video_stream::VideoStream;
use websocket::websocket_handler;
use std::collections::HashMap;
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
            info!("Initializing recording system with database: {}", recording_config.database_path);
            
            match SqliteDatabase::new(&recording_config.database_path).await {
                Ok(database) => {
                    let database: Arc<dyn database::DatabaseProvider> = Arc::new(database);
                    
                    let recording_config = RecordingConfig {
                        max_frame_size: recording_config.max_frame_size.unwrap_or(10 * 1024 * 1024),
                    };
                    
                    match RecordingManager::new(recording_config, database).await {
                        Ok(manager) => {
                            info!("Recording system initialized successfully");
                            Some(Arc::new(manager))
                        }
                        Err(e) => {
                            error!("Failed to initialize recording manager: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to initialize database: {}", e);
                    None
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
    #[derive(Clone)]
    struct CameraStreamInfo {
        camera_id: String,
        frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
        mqtt_handle: Option<MqttHandle>,
        camera_config: config::CameraConfig,
        recording_manager: Option<Arc<RecordingManager>>,
    }
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

    // Build router with camera paths
    let mut app = axum::Router::new()
        .route("/", axum::routing::get(index_handler))
        .nest_service("/static", tower_http::services::ServeDir::new("static"));
    
    // Add routes for each camera (both stream and control endpoints)
    for (path, stream_info) in camera_streams {
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

        // Live endpoint: /<camera_path>/live  
        let live_path = format!("{}/live", path);
        let live_info_clone = stream_info.clone();
        app = app.route(&live_path, axum::routing::get(
            move |ws, query, addr| camera_handler(
                ws, query, addr, 
                live_info_clone.frame_sender.clone(), 
                live_info_clone.camera_id.clone(), 
                live_info_clone.mqtt_handle.clone(), 
                live_info_clone.camera_config.clone()
            )
        ));

        // Keep legacy endpoint for backward compatibility (redirects to /live)
        let legacy_info_clone = stream_info.clone();
        app = app.route(&path, axum::routing::get(
            move |ws, query, addr| camera_handler(
                ws, query, addr, 
                legacy_info_clone.frame_sender.clone(), 
                legacy_info_clone.camera_id.clone(), 
                legacy_info_clone.mqtt_handle.clone(), 
                legacy_info_clone.camera_config.clone()
            )
        ));
    }
    
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

async fn serve_index_with_mode(is_full_mode: bool) -> axum::response::Html<String> {
    let mut html = include_str!("../static/index.html").to_string();
    
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


async fn index_handler(
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    // Check if 'full' parameter is present
    let is_full_mode = query.contains_key("full");
    serve_index_with_mode(is_full_mode).await.into_response()
}

async fn camera_handler(
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
            // Check if 'full' parameter is present
            let is_full_mode = query.contains_key("full");
            serve_index_with_mode(is_full_mode).await.into_response()
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
            // Check if 'full' parameter is present
            let is_full_mode = query.contains_key("full");
            serve_index_with_mode(is_full_mode).await.into_response()
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