use std::sync::Arc;
use tokio::sync::broadcast;
use axum::response::IntoResponse;
use axum::extract::{State, Query};
use tracing::trace;

use crate::{config, AppState};
use crate::websocket::websocket_handler;
use crate::control::handle_control_websocket;
use crate::recording::RecordingManager;
use crate::mqtt::MqttHandle;

pub async fn dashboard_handler() -> axum::response::Html<String> {
    trace!("Dashboard HTML requested");
    let html = include_str!("../static/dashboard.html").to_string();
    axum::response::Html(html)
}

pub async fn serve_control_page() -> axum::response::Html<String> {
    let html = include_str!("../static/control.html").to_string();
    axum::response::Html(html)
}

pub async fn serve_stream_page() -> axum::response::Html<String> {
    let html = include_str!("../static/stream.html").to_string();
    axum::response::Html(html)
}

pub async fn serve_test_page(
    query: Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let is_full_mode = query.contains_key("full");
    serve_test_with_mode(is_full_mode).await.into_response()
}

async fn serve_test_with_mode(is_full_mode: bool) -> axum::response::Html<String> {
    let mut html = include_str!("../static/test.html").to_string();
    
    if is_full_mode {
        let full_mode_script = r#"
        <script>
            // Enable full mode by setting a flag before the main script runs
            window.FULL_MODE = true;
        </script>"#;
        
        html = html.replace("</head>", &format!("{}</head>", full_mode_script));
    }
    
    axum::response::Html(html)
}

// Dynamic handlers that check current state instead of using captured state
pub async fn dynamic_camera_stream_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: Query<std::collections::HashMap<String, String>>,
    addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    camera_id: String,
    app_state: AppState,
) -> axum::response::Response {
    let camera_streams = app_state.camera_streams.read().await;
    if let Some(stream_info) = camera_streams.get(&camera_id) {
        let stream_info = stream_info.clone();
        drop(camera_streams);
        
        camera_stream_handler(
            ws, query, addr,
            stream_info.frame_sender,
            stream_info.camera_id,
            stream_info.mqtt_handle,
            stream_info.camera_config,
        ).await
    } else {
        (axum::http::StatusCode::NOT_FOUND, "Camera not found").into_response()
    }
}

pub async fn dynamic_camera_control_handler(
    headers: axum::http::HeaderMap,
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: Query<std::collections::HashMap<String, String>>,
    addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    camera_id: String,
    app_state: AppState,
) -> axum::response::Response {
    let camera_streams = app_state.camera_streams.read().await;
    if let Some(stream_info) = camera_streams.get(&camera_id) {
        let stream_info = stream_info.clone();
        drop(camera_streams);
        
        camera_control_handler(
            headers, ws, query, addr,
            stream_info.frame_sender,
            stream_info.camera_id,
            stream_info.mqtt_handle,
            stream_info.camera_config,
            stream_info.recording_manager,
        ).await
    } else {
        (axum::http::StatusCode::NOT_FOUND, "Camera not found").into_response()
    }
}

pub async fn dynamic_camera_live_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: Query<std::collections::HashMap<String, String>>,
    addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    camera_id: String,
    app_state: AppState,
) -> axum::response::Response {
    let camera_streams = app_state.camera_streams.read().await;
    if let Some(stream_info) = camera_streams.get(&camera_id) {
        let stream_info = stream_info.clone();
        drop(camera_streams);
        
        camera_live_handler(
            ws, query, addr,
            stream_info.frame_sender,
            stream_info.camera_id,
            stream_info.mqtt_handle,
            stream_info.camera_config,
        ).await
    } else {
        (axum::http::StatusCode::NOT_FOUND, "Camera not found").into_response()
    }
}

pub async fn camera_live_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: Query<std::collections::HashMap<String, String>>,
    addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
) -> axum::response::Response {
    use tracing::{trace, info, debug, warn};
    
    let current_connections = frame_sender.receiver_count();
    trace!("Live handler called for camera {} (connections: {}), WS upgrade: {}", 
          camera_id, current_connections, ws.is_some());
    match ws {
        Some(ws_upgrade) => {
            if let Some(expected_token) = &camera_config.token {
                if let Some(provided_token) = query.get("token") {
                    if provided_token == expected_token {
                        info!("Token authentication successful for camera {}", camera_id);
                    } else {
                        debug!("Invalid token provided for camera {}", camera_id);
                        return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response();
                    }
                } else {
                    warn!("Missing token for camera {} that requires authentication", camera_id);
                    return (axum::http::StatusCode::UNAUTHORIZED, "Missing token").into_response();
                }
            }
            
            if let Some(connect_info) = addr {
                trace!("Starting live WebSocket handler for camera {} from {}", camera_id, connect_info.0);
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            } else {
                let fallback_addr = "127.0.0.1:0".parse().unwrap();
                let connect_info = axum::extract::ConnectInfo(fallback_addr);
                trace!("Starting live WebSocket handler for camera {} (fallback addr)", camera_id);
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            }
        },
        None => {
            (axum::http::StatusCode::BAD_REQUEST, "Live endpoint only accepts WebSocket connections").into_response()
        }
    }
}

pub async fn camera_stream_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: Query<std::collections::HashMap<String, String>>,
    addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
) -> axum::response::Response {
    use tracing::{trace, info, debug, warn};
    
    match ws {
        Some(ws_upgrade) => {
            if let Some(expected_token) = &camera_config.token {
                if let Some(provided_token) = query.get("token") {
                    if provided_token == expected_token {
                        info!("Token authentication successful for camera {}", camera_id);
                    } else {
                        debug!("Invalid token provided for camera {}", camera_id);
                        return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response();
                    }
                } else {
                    warn!("Missing token for camera {} that requires authentication", camera_id);
                    return (axum::http::StatusCode::UNAUTHORIZED, "Missing token").into_response();
                }
            }
            
            if let Some(connect_info) = addr {
                trace!("Starting stream WebSocket handler for camera {} from {}", camera_id, connect_info.0);
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            } else {
                let fallback_addr = "127.0.0.1:0".parse().unwrap();
                let connect_info = axum::extract::ConnectInfo(fallback_addr);
                trace!("Starting stream WebSocket handler for camera {} (fallback addr)", camera_id);
                websocket_handler(ws_upgrade, State(frame_sender), connect_info, camera_id, mqtt_handle, camera_config).await
            }
        },
        None => {
            serve_stream_page().await.into_response()
        }
    }
}

pub async fn camera_control_handler(
    headers: axum::http::HeaderMap,
    ws: Option<axum::extract::WebSocketUpgrade>,
    query: Query<std::collections::HashMap<String, String>>,
    _addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    camera_id: String,
    _mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
    recording_manager: Option<Arc<RecordingManager>>,
) -> axum::response::Response {
    use tracing::{trace, info, warn, debug};
    
    match ws {
        Some(ws_upgrade) => {
            if let Some(expected_token) = &camera_config.token {
                let mut token_valid = false;
                
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
                
                if !token_valid {
                    debug!("Missing or invalid authentication for camera {} control that requires authentication", camera_id);
                    return (axum::http::StatusCode::UNAUTHORIZED, "Missing or invalid authentication - provide Bearer token in Authorization header or ?token= query parameter").into_response();
                }
            }

            if recording_manager.is_none() {
                warn!("Recording not enabled, control endpoint unavailable");
                return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Recording system not enabled").into_response();
            }
            
            let client_id = uuid::Uuid::new_v4().to_string();
            trace!("[CONTROL] Starting control WebSocket upgrade for camera {} with client {}", camera_id, client_id);
            let camera_id_clone = camera_id.clone();
            let client_id_clone = client_id.clone();
            let socket = ws_upgrade.on_upgrade(move |socket| async move {
                trace!("[CONTROL] WebSocket upgraded for camera {} client {}", camera_id_clone, client_id_clone);
                let camera_id_task = camera_id.clone();
                let client_id_task = client_id.clone();
                let _handle = tokio::spawn(async move {
                    trace!("[CONTROL] Spawned control handler task for camera {} client {}", camera_id_task, client_id_task);
                    handle_control_websocket(
                        socket,
                        camera_id,
                        client_id,
                        recording_manager.unwrap(),
                        frame_sender,
                    ).await;
                    trace!("[CONTROL] Control handler task completed for camera {} client {}", camera_id_task, client_id_task);
                });
                trace!("[CONTROL] Control handler spawned, waiting for completion");
            });
            socket.into_response()
        },
        None => {
            serve_control_page().await.into_response()
        }
    }
}