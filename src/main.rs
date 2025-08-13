use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn, error};
use anyhow::Result;
use std::fs::File;
use std::io::BufReader;
use axum::response::IntoResponse;
use axum::extract::State;

mod config;
mod rtsp_client;
mod websocket;
mod transcoder;
mod video_stream;

use config::Config;
use video_stream::VideoStream;
use websocket::websocket_handler;
use std::collections::HashMap;


#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("rtsp_streaming_server=debug,info")
        .init();

    let config = Config::load("config.toml").unwrap_or_else(|_| {
        warn!("Could not load config.toml, using default configuration");
        Config::default()
    });

    info!("Starting RTSP streaming server on {}:{}", config.server.host, config.server.port);

    // Create video streams for each camera
    let mut camera_streams: HashMap<String, Arc<broadcast::Sender<bytes::Bytes>>> = HashMap::new();
    
    for (camera_id, camera_config) in config.cameras.clone() {
        info!("Configuring camera '{}' on path '{}'...", camera_id, camera_config.path);
        
        match VideoStream::new(
            camera_id.clone(),
            camera_config.clone(),
            &config.transcoding,
        ).await {
            Ok(video_stream) => {
                // Store the frame sender for this camera's path
                camera_streams.insert(camera_config.path.clone(), video_stream.frame_sender.clone());
                
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
        return Err(anyhow::anyhow!("No cameras available"));
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
    
    // Add a route for each camera
    for (path, frame_sender) in camera_streams {
        info!("Adding route for camera at path: {}", path);
        let sender = frame_sender.clone();
        app = app.route(&path, axum::routing::get(
            move |ws, query| camera_handler(ws, query, sender.clone())
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
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
) -> axum::response::Response {
    match ws {
        Some(ws_upgrade) => websocket_handler(ws_upgrade, State(frame_sender)).await,
        None => {
            // Check if 'full' parameter is present
            let is_full_mode = query.contains_key("full");
            serve_index_with_mode(is_full_mode).await.into_response()
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
        .map_err(|e| anyhow::anyhow!("Failed to open certificate file '{}': {}", tls_cfg.cert_path, e))?;
    let key_file = File::open(&tls_cfg.key_path)
        .map_err(|e| anyhow::anyhow!("Failed to open private key file '{}': {}", tls_cfg.key_path, e))?;

    let mut cert_reader = BufReader::new(cert_file);
    let mut key_reader = BufReader::new(key_file);

    // Parse certificate and key
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .map_err(|e| anyhow::anyhow!("Failed to parse certificate: {}", e))?
        .into_iter()
        .map(rustls::Certificate)
        .collect();
    
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
        .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?;
    
    if keys.is_empty() {
        // Try RSA private keys if PKCS8 fails
        let mut key_reader = BufReader::new(File::open(&tls_cfg.key_path)?);
        keys = rustls_pemfile::rsa_private_keys(&mut key_reader)
            .map_err(|e| anyhow::anyhow!("Failed to parse RSA private key: {}", e))?;
    }
    
    let private_key = keys.into_iter().next()
        .ok_or_else(|| anyhow::anyhow!("No private key found in key file"))?;

    // Create TLS configuration
    let rustls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, rustls::PrivateKey(private_key))
        .map_err(|e| anyhow::anyhow!("Failed to create TLS config: {}", e))?;

    info!("HTTPS server listening on https://{}", addr);
    info!("Certificate: {}", tls_cfg.cert_path);
    info!("Private key: {}", tls_cfg.key_path);

    // Start HTTPS server
    let tls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(rustls_config));
    axum_server::bind_rustls(addr.parse()?, tls_config)
        .serve(app.into_make_service())
        .await
        .map_err(|e| anyhow::anyhow!("HTTPS server error: {}", e))?;

    Ok(())
}