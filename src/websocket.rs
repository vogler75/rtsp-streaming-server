use std::sync::Arc;
use axum::{
    extract::{State, WebSocketUpgrade, ConnectInfo},
    response::Response,
};
use axum::extract::ws::{WebSocket, Message};
use tokio::sync::broadcast;
use futures_util::{stream::StreamExt, SinkExt};
use tracing::{info, error, debug};
use bytes::Bytes;
use crate::mqtt::{MqttHandle, ClientStatus};
use chrono::Utc;
use uuid::Uuid;
use std::net::SocketAddr;

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(frame_sender): State<Arc<broadcast::Sender<Bytes>>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, frame_sender, camera_id, mqtt_handle, addr))
}

async fn handle_socket(
    socket: WebSocket,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    client_addr: SocketAddr,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut frame_receiver = frame_sender.subscribe();
    
    let client_id = Uuid::new_v4().to_string();
    let client_ip = client_addr.ip().to_string();
    info!("New WebSocket client {} ({}) connected to camera {}", client_id, client_ip, camera_id);
    debug!("Frame sender has {} subscribers", frame_sender.receiver_count());
    
    // Register client with MQTT
    if let Some(ref mqtt) = mqtt_handle {
        let client_status = ClientStatus {
            id: client_id.clone(),
            camera_id: camera_id.clone(),
            connected_at: Utc::now().to_rfc3339(),
            frames_sent: 0,
            actual_fps: 0.0,
            ip_address: client_ip,
        };
        mqtt.add_client(client_status).await;
    }

    let mqtt_handle_clone = mqtt_handle.clone();
    let client_id_clone = client_id.clone();
    
    let send_task = tokio::spawn(async move {
        let mut frame_count = 0u64;
        let mut dropped_frames = 0u64;
        let mut total_frames_sent = 0u64;
        let mut last_stats_time = tokio::time::Instant::now();
        let mut fps_frame_count = 0u64;
        
        loop {
            match frame_receiver.recv().await {
                Ok(frame_data) => {
                    frame_count += 1;
                    fps_frame_count += 1;
                    
                    // Use timeout for non-blocking send - drop frame if it takes too long
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(10), // 10ms timeout for sending
                        sender.send(Message::Binary(frame_data.to_vec()))
                    ).await {
                        Ok(Ok(())) => {
                            // Frame sent successfully
                            total_frames_sent += 1;
                        }
                        Ok(Err(_)) => {
                            // Connection error
                            error!("WebSocket connection error");
                            break;
                        }
                        Err(_) => {
                            // Timeout - client is too slow, drop this frame
                            dropped_frames += 1;
                            if dropped_frames % 10 == 0 {
                                debug!("Dropped {} frames due to slow client", dropped_frames);
                            }
                            // Flush the sender to clear any pending data
                            let _ = sender.flush().await;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // We're too slow and frames were dropped to keep up
                    // This is expected behavior with channel_buffer_size=1
                    dropped_frames += skipped as u64;
                    debug!("WebSocket lagged, dropped {} old frames", skipped);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // Channel closed, exit
                    break;
                }
            }
            
            // Update client stats periodically
            let now = tokio::time::Instant::now();
            if now.duration_since(last_stats_time) >= std::time::Duration::from_secs(1) {
                let fps = fps_frame_count as f32;
                
                if let Some(ref mqtt) = mqtt_handle_clone {
                    mqtt.update_client_stats(&client_id_clone, total_frames_sent, fps).await;
                }
                
                fps_frame_count = 0;
                last_stats_time = now;
            }
        }
        info!("WebSocket send task ended (sent: {}, dropped: {})", frame_count, dropped_frames);
    });

    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    debug!("Received text message: {}", text);
                }
                Ok(Message::Binary(_)) => {
                    debug!("Received binary message");
                }
                Ok(Message::Close(_)) => {
                    info!("WebSocket client disconnected");
                    break;
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }
        info!("WebSocket receive task ended");
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!("WebSocket client {} disconnected", client_id);
    
    // Unregister client from MQTT
    if let Some(ref mqtt) = mqtt_handle {
        mqtt.remove_client(&client_id).await;
    }
}