use std::sync::Arc;
use axum::{
    extract::{State, WebSocketUpgrade},
    response::Response,
};
use axum::extract::ws::{WebSocket, Message};
use tokio::sync::broadcast;
use futures_util::{stream::StreamExt, SinkExt};
use tracing::{info, error, debug};
use bytes::Bytes;

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(frame_sender): State<Arc<broadcast::Sender<Bytes>>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, frame_sender))
}

async fn handle_socket(socket: WebSocket, frame_sender: Arc<broadcast::Sender<Bytes>>) {
    let (mut sender, mut receiver) = socket.split();
    let mut frame_receiver = frame_sender.subscribe();
    
    info!("New WebSocket client connected");
    debug!("Frame sender has {} subscribers", frame_sender.receiver_count());

    let send_task = tokio::spawn(async move {
        let mut frame_count = 0u64;
        let mut dropped_frames = 0u64;
        
        loop {
            match frame_receiver.recv().await {
                Ok(frame_data) => {
                    frame_count += 1;
                    
                    // Use timeout for non-blocking send - drop frame if it takes too long
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(10), // 10ms timeout for sending
                        sender.send(Message::Binary(frame_data.to_vec()))
                    ).await {
                        Ok(Ok(())) => {
                            // Frame sent successfully
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

    info!("WebSocket client disconnected");
}