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

use config::Config;
use rtsp_client::RtspClient;
use websocket::websocket_handler;

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

    // Use configured channel buffer size or default to 1 for only latest frame
    let channel_buffer_size = config.transcoding.channel_buffer_size.unwrap_or(1);
    info!("Using channel buffer size: {} frames", channel_buffer_size);
    let (frame_tx, _) = broadcast::channel(channel_buffer_size);
    let frame_tx = Arc::new(frame_tx);

    let rtsp_client = RtspClient::new(
        config.rtsp.clone(), 
        frame_tx.clone(),
        config.transcoding.quality,
        config.transcoding.capture_framerate,
        config.transcoding.send_framerate,
        config.transcoding.allow_duplicate_frames.unwrap_or(false)
    ).await;
    
    tokio::spawn(async move {
        if let Err(e) = rtsp_client.start().await {
            error!("RTSP client error: {}", e);
        }
    });

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

    let app = axum::Router::new()
        .route("/", axum::routing::get(root_handler))
        .nest_service("/static", tower_http::services::ServeDir::new("static"))
        .layer(cors_layer)
        .with_state(frame_tx);

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

async fn serve_index() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/index.html"))
}

async fn root_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    State(frame_sender): axum::extract::State<Arc<broadcast::Sender<bytes::Bytes>>>,
) -> axum::response::Response {
    match ws {
        Some(ws_upgrade) => websocket_handler(ws_upgrade, State(frame_sender)).await,
        None => serve_index().await.into_response(),
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