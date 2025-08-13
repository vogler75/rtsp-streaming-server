use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn, error};
use anyhow::Result;

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
        .with_env_filter("debug")
        .init();

    let config = Config::load("config.toml").unwrap_or_else(|_| {
        warn!("Could not load config.toml, using default configuration");
        Config::default()
    });

    info!("Starting RTSP streaming server on {}:{}", config.server.host, config.server.port);

    let (frame_tx, _) = broadcast::channel(100);
    let frame_tx = Arc::new(frame_tx);

    let rtsp_client = RtspClient::new(config.rtsp.clone(), frame_tx.clone());
    
    tokio::spawn(async move {
        if let Err(e) = rtsp_client.start().await {
            error!("RTSP client error: {}", e);
        }
    });

    let app = axum::Router::new()
        .route("/ws", axum::routing::get(websocket_handler))
        .route("/", axum::routing::get(serve_index))
        .nest_service("/static", tower_http::services::ServeDir::new("static"))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(frame_tx);

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", config.server.host, config.server.port)).await?;
    
    info!("Server listening on http://{}:{}", config.server.host, config.server.port);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn serve_index() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/index.html"))
}