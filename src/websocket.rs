use std::sync::Arc;
use axum::{
    extract::{State, WebSocketUpgrade, ConnectInfo},
    response::Response,
};
use axum::extract::ws::{WebSocket, Message};
use tokio::sync::broadcast;
use futures_util::{stream::StreamExt, SinkExt};
use tracing::{info, error, warn, trace};
use bytes::Bytes;
use crate::mqtt::{MqttHandle, ClientStatus};
use crate::config::CameraConfig;
use chrono::Utc;
use uuid::Uuid;
use std::net::SocketAddr;

// Rate limiting has been disabled to prevent blocking issues
// The code has been removed as it was causing dashboard access problems

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(frame_sender): State<Arc<broadcast::Sender<Bytes>>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    _camera_config: CameraConfig,
) -> Response {
    // Authentication is handled in camera_handler before this function is called
    let current_connections = frame_sender.receiver_count();
    info!("WebSocket upgrade for client {} on camera {} (current connections: {})", addr, camera_id, current_connections);
    
    // Verbose-only debugging for connection limits
    if current_connections >= 10 {
        trace!("High number of connections ({}) for camera {}, new client: {}", current_connections, camera_id, addr);
    }
    
    // Verbose-only system resource information when approaching limits
    if current_connections >= 12 {
        #[cfg(unix)]
        {
            use std::process::Command;
            if let Ok(output) = Command::new("sh").arg("-c").arg("ulimit -n").output() {
                if let Ok(limit_str) = String::from_utf8(output.stdout) {
                    trace!("System file descriptor limit: {}", limit_str.trim());
                }
            }
            if let Ok(output) = Command::new("lsof").arg("-p").arg(&std::process::id().to_string()).output() {
                let fd_count = String::from_utf8_lossy(&output.stdout).lines().count();
                trace!("Current process file descriptors in use: {}", fd_count);
            }
        }
    }
    
    ws.on_upgrade(move |socket| handle_socket(socket, frame_sender, camera_id, mqtt_handle, addr))
}

async fn handle_socket(
    socket: WebSocket,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    client_addr: SocketAddr,
) {
    let client_id = Uuid::new_v4().to_string();
    let client_ip = client_addr.ip().to_string();
    
    trace!("[{}] Starting WebSocket connection setup for camera {}", client_id, camera_id);
    
    // Wrap the entire socket handling in error handling
    if let Err(e) = handle_socket_inner(socket, frame_sender, camera_id, mqtt_handle, client_addr, client_id, client_ip).await {
        error!("WebSocket handling error: {}", e);
    }
}

async fn handle_socket_inner(
    socket: WebSocket,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    camera_id: String,
    mqtt_handle: Option<MqttHandle>,
    _client_addr: SocketAddr,
    client_id: String,
    client_ip: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    
    // Rate limiting has been disabled to prevent blocking issues
    
    let (mut sender, mut receiver) = socket.split();
    trace!("[{}] WebSocket split completed", client_id);
    
    let subscriber_count_before = frame_sender.receiver_count();
    trace!("[{}] Subscriber count before subscribe: {}", client_id, subscriber_count_before);
    
    // Subscribe to frame stream
    trace!("[{}] About to call frame_sender.subscribe()", client_id);
    let subscription_start = std::time::Instant::now();
    
    let frame_receiver = frame_sender.subscribe();
    
    let subscription_duration = subscription_start.elapsed();
    trace!("[{}] Subscribe completed in {:?}", client_id, subscription_duration);
    
    let subscriber_count_after = frame_sender.receiver_count();
    trace!("[{}] Subscriber count after subscribe: {} (delta: +{})", 
           client_id, subscriber_count_after, subscriber_count_after.saturating_sub(subscriber_count_before));
    
    // Check if we're approaching the channel limit (verbose only)
    if subscriber_count_after > 8 {
        trace!("[{}] High subscriber count ({}) for camera {} - may cause performance issues", 
              client_id, subscriber_count_after, camera_id);
    }
    
    info!("New WebSocket client {} ({}) connected to camera {}", client_id, client_ip, camera_id);
    trace!("Frame sender has {} subscribers", frame_sender.receiver_count());
    
    // Register client with MQTT (OUTSIDE mutex to prevent blocking)
    if let Some(ref mqtt) = mqtt_handle {
        let client_status = ClientStatus {
            id: client_id.clone(),
            camera_id: camera_id.clone(),
            connected_at: Utc::now().to_rfc3339(),
            frames_sent: 0,
            actual_fps: 0.0,
            ip_address: client_ip,
        };
        // This is now outside the mutex so it won't block other connections
        mqtt.add_client(client_status).await;
    }

    let mqtt_handle_clone = mqtt_handle.clone();
    let client_id_clone = client_id.clone();
    
    trace!("[{}] About to spawn send_task", client_id);
    let task_spawn_start = std::time::Instant::now();
    
    let mut send_task = tokio::spawn(async move {
        trace!("[{}] Send_task started", client_id_clone);
        let task_start_time = std::time::Instant::now();
        let mut frame_count = 0u64;
        let mut dropped_frames = 0u64;
        let mut total_frames_sent = 0u64;
        let mut last_stats_time = tokio::time::Instant::now();
        let mut fps_frame_count = 0u64;
        let mut frame_receiver = frame_receiver; // Move the frame_receiver into the task
        
        trace!("[{}] Starting frame receive loop", client_id_clone);
        
        loop {
            match frame_receiver.recv().await {
                Ok(frame_data) => {
                    frame_count += 1;
                    
                    // Log first frame received
                    if frame_count == 1 {
                        trace!("[{}] First frame received at {:?}", client_id_clone, task_start_time.elapsed());
                    }
                    fps_frame_count += 1;
                    
                    // Use timeout for non-blocking send - drop frame if it takes too long
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(5), // Reduced timeout for faster dropping
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
                                trace!("Dropped {} frames due to slow client", dropped_frames);
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
                    trace!("WebSocket lagged, dropped {} old frames", skipped);
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

    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    trace!("Received text message: {}", text);
                }
                Ok(Message::Binary(_)) => {
                    trace!("Received binary message");
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
    
    let spawn_duration = task_spawn_start.elapsed();
    trace!("[{}] Both tasks spawned in {:?}", client_id, spawn_duration);

    // Use tokio::select! to wait for either task to complete, then abort the other
    tokio::select! {
        send_result = &mut send_task => {
            trace!("[{}] Send task completed with result: {:?}", client_id, send_result);
            // Abort the receive task if send task completes first
            recv_task.abort();
            // Wait for abort with timeout to prevent hanging
            match tokio::time::timeout(std::time::Duration::from_millis(100), recv_task).await {
                Ok(_) => trace!("[{}] Receive task aborted after send task completion", client_id),
                Err(_) => warn!("[{}] Timeout waiting for receive task abort", client_id),
            }
        },
        recv_result = &mut recv_task => {
            trace!("[{}] Recv task completed with result: {:?}", client_id, recv_result);
            // Abort the send task if receive task completes first
            send_task.abort();
            // Wait for abort with timeout to prevent hanging
            match tokio::time::timeout(std::time::Duration::from_millis(100), send_task).await {
                Ok(_) => trace!("[{}] Send task aborted after receive task completion", client_id),
                Err(_) => warn!("[{}] Timeout waiting for send task abort", client_id),
            }
        },
    }

    info!("WebSocket client {} disconnected", client_id);
    
    // Unregister client from MQTT (with timeout to prevent blocking)
    if let Some(ref mqtt) = mqtt_handle {
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            mqtt.remove_client(&client_id)
        ).await {
            Ok(_) => trace!("[{}] Client unregistered from MQTT", client_id),
            Err(_) => error!("[{}] Timeout unregistering client from MQTT", client_id),
        }
    }
    
    Ok(())
}