use crate::errors::{Result, StreamError};
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::config::MqttConfig;
use chrono::Utc;

#[derive(Debug, Clone, Serialize)]
pub struct CameraStatus {
    pub id: String,
    pub connected: bool,
    pub capture_fps: f32,
    pub clients_connected: usize, // Total subscribers: WebSocket clients + internal systems (recording=1, control=1)
    pub last_frame_time: Option<String>,
    pub ffmpeg_running: bool,
    pub duplicate_frames: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PictureArrival {
    pub t: u128,  // Timestamp in milliseconds since epoch
    pub d: u128,  // Time difference from previous picture in milliseconds
    pub s: usize, // Frame size in bytes
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientStatus {
    pub id: String,
    pub camera_id: String,
    pub connected_at: String,
    pub frames_sent: u64,
    pub actual_fps: f32,
    pub ip_address: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientEvent {
    pub client_id: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThroughputStats {
    pub bytes_per_second: i64,
    pub frame_count: i32,
    pub ffmpeg_fps: f32,
    pub connection_count: i32,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub uptime_secs: u64,
    pub total_clients: usize,
    pub total_cameras: usize,
}

pub struct MqttPublisher {
    client: AsyncClient,
    eventloop: EventLoop,
    config: MqttConfig,
    camera_status: Arc<RwLock<HashMap<String, CameraStatus>>>,
    client_status: Arc<RwLock<Vec<ClientStatus>>>,
    start_time: std::time::Instant,
}

impl MqttPublisher {
    pub async fn new(config: MqttConfig) -> Result<Self> {
        // Parse the broker URL to extract host and port
        let url = url::Url::parse(&config.broker_url)
            .map_err(|e| StreamError::mqtt(format!("Invalid MQTT broker URL '{}': {}", config.broker_url, e)))?;
        
        let host = url.host_str()
            .ok_or_else(|| StreamError::mqtt(format!("No host found in MQTT broker URL: {}", config.broker_url)))?;
        
        let port = url.port().unwrap_or(1883);
        
        info!("Connecting to MQTT broker at {}:{}", host, port);
        
        let mut mqtt_options = MqttOptions::new(
            &config.client_id,
            host,
            port,
        );
        
        mqtt_options.set_keep_alive(Duration::from_secs(config.keep_alive_secs));
        
        // Set maximum packet size (default to 256MB if not specified)
        let max_packet_size = config.max_packet_size.unwrap_or(268435455); // 256MB - 1 byte
        mqtt_options.set_max_packet_size(max_packet_size, max_packet_size);
        
        if let Some(username) = &config.username {
            if let Some(password) = &config.password {
                mqtt_options.set_credentials(username, password);
            }
        }
        
        let (client, eventloop) = AsyncClient::new(mqtt_options, 100);
        
        Ok(Self {
            client,
            eventloop,
            config,
            camera_status: Arc::new(RwLock::new(HashMap::new())),
            client_status: Arc::new(RwLock::new(Vec::new())),
            start_time: std::time::Instant::now(),
        })
    }
    
    pub async fn start(mut self) -> Result<MqttHandle> {
        let client = self.client.clone();
        let config = self.config.clone();
        let camera_status = self.camera_status.clone();
        let client_status = self.client_status.clone();
        
        // Spawn event loop handler
        let _eventloop_handle = tokio::spawn(async move {
            loop {
                match self.eventloop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        info!("Connected to MQTT broker");
                    }
                    Ok(Event::Incoming(Packet::Disconnect)) => {
                        warn!("Disconnected from MQTT broker");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!("MQTT connection error: {}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
        
        // Spawn status publisher
        let client_clone = client.clone();
        let config_clone = config.clone();
        let camera_status_clone = camera_status.clone();
        let client_status_clone = client_status.clone();
        let start_time = self.start_time;
        
        let _publisher_handle = tokio::spawn(async move {
            let mut publish_interval = interval(Duration::from_secs(config_clone.publish_interval_secs));
            
            loop {
                publish_interval.tick().await;
                
                let cameras = camera_status_clone.read().await.clone();
                let clients = client_status_clone.read().await.clone();
                
                // Publish server status
                let status = ServerStatus {
                    uptime_secs: start_time.elapsed().as_secs(),
                    total_clients: clients.len(),
                    total_cameras: cameras.len(),
                };
                
                if let Ok(payload) = serde_json::to_string(&status) {
                    let topic = format!("{}/status", config_clone.base_topic);
                    let qos = match config_clone.qos {
                        0 => QoS::AtMostOnce,
                        1 => QoS::AtLeastOnce,
                        _ => QoS::ExactlyOnce,
                    };
                    
                    if let Err(e) = client_clone.publish(
                        topic,
                        qos,
                        config_clone.retain,
                        payload.as_bytes(),
                    ).await {
                        error!("Failed to publish server status: {}", e);
                    }
                }
                
                // Also publish individual camera status updates at the same interval
                for (camera_id, camera_status) in &cameras {
                    if let Ok(payload) = serde_json::to_string(&camera_status) {
                        let topic = format!("{}/cameras/{}/status", config_clone.base_topic, camera_id);
                        let qos = match config_clone.qos {
                            0 => QoS::AtMostOnce,
                            1 => QoS::AtLeastOnce,
                            _ => QoS::ExactlyOnce,
                        };
                        
                        if let Err(e) = client_clone.publish(
                            topic,
                            qos,
                            config_clone.retain,
                            payload.as_bytes(),
                        ).await {
                            error!("Failed to publish camera status for {}: {}", camera_id, e);
                        }
                    }
                }
            }
        });
        
        Ok(MqttHandle {
            client,
            camera_status,
            client_status,
            config,
        })
    }
}

#[derive(Clone)]
pub struct MqttHandle {
    client: AsyncClient,
    camera_status: Arc<RwLock<HashMap<String, CameraStatus>>>,
    client_status: Arc<RwLock<Vec<ClientStatus>>>,
    config: MqttConfig,
}

impl MqttHandle {
    pub async fn update_camera_status(&self, camera_id: String, status: CameraStatus) {
        let mut cameras = self.camera_status.write().await;
        cameras.insert(camera_id.clone(), status.clone());
        
        // Only store the status - publishing will be handled by the interval timer
        // This respects the configured publish_interval_secs for all status updates
    }
    
    pub async fn add_client(&self, client: ClientStatus) {
        let mut clients = self.client_status.write().await;
        clients.push(client.clone());
        
        // Publish client status to individual client topic
        let topic = format!("{}/clients/{}/status", self.config.base_topic, client.id);
        if let Ok(payload) = serde_json::to_string(&client) {
            let qos = match self.config.qos {
                0 => QoS::AtMostOnce,
                1 => QoS::AtLeastOnce,
                _ => QoS::ExactlyOnce,
            };
            
            if let Err(e) = self.client.publish(
                topic,
                qos,
                self.config.retain,
                payload.as_bytes(),
            ).await {
                error!("Failed to publish client status: {}", e);
            }
        }
        
        // Also publish connection event to global connected topic
        let event_topic = format!("{}/clients/connected", self.config.base_topic);
        let event = ClientEvent {
            client_id: client.id.clone(),
            timestamp: client.connected_at.clone(),
        };
        if let Ok(payload) = serde_json::to_string(&event) {
            let qos = match self.config.qos {
                0 => QoS::AtMostOnce,
                1 => QoS::AtLeastOnce,
                _ => QoS::ExactlyOnce,
            };
            
            if let Err(e) = self.client.publish(
                event_topic,
                qos,
                false, // Don't retain events
                payload.as_bytes(),
            ).await {
                error!("Failed to publish client connection event: {}", e);
            }
        }
    }
    
    pub async fn remove_client(&self, client_id: &str) {
        let mut clients = self.client_status.write().await;
        if let Some(pos) = clients.iter().position(|c| c.id == client_id) {
            let client = clients.remove(pos);
            
            // Remove client status from individual client topic (publish empty retained message)
            let topic = format!("{}/clients/{}/status", self.config.base_topic, client_id);
            let qos = match self.config.qos {
                0 => QoS::AtMostOnce,
                1 => QoS::AtLeastOnce,
                _ => QoS::ExactlyOnce,
            };
            
            if let Err(e) = self.client.publish(
                topic,
                qos,
                true, // Retain empty message to clear the topic
                &[],  // Empty payload
            ).await {
                error!("Failed to clear client status topic: {}", e);
            }
            
            // Publish client disconnection event to global disconnected topic
            let event_topic = format!("{}/clients/disconnected", self.config.base_topic);
            let event = ClientEvent {
                client_id: client.id.clone(),
                timestamp: Utc::now().to_rfc3339(),
            };
            if let Ok(payload) = serde_json::to_string(&event) {
                if let Err(e) = self.client.publish(
                    event_topic,
                    qos,
                    false, // Don't retain events
                    payload.as_bytes(),
                ).await {
                    error!("Failed to publish client disconnection event: {}", e);
                }
            }
        }
    }
    
    pub async fn update_client_stats(&self, client_id: &str, frames_sent: u64, actual_fps: f32) {
        let mut clients = self.client_status.write().await;
        if let Some(client) = clients.iter_mut().find(|c| c.id == client_id) {
            client.frames_sent = frames_sent;
            client.actual_fps = actual_fps;
            
            // Publish updated client status to individual client topic
            let topic = format!("{}/clients/{}/status", self.config.base_topic, client_id);
            if let Ok(payload) = serde_json::to_string(&client) {
                let qos = match self.config.qos {
                    0 => QoS::AtMostOnce,
                    1 => QoS::AtLeastOnce,
                    _ => QoS::ExactlyOnce,
                };
                
                if let Err(e) = self.client.publish(
                    topic,
                    qos,
                    self.config.retain,
                    payload.as_bytes(),
                ).await {
                    error!("Failed to publish client stats update: {}", e);
                }
            }
        }
    }
    
    #[allow(dead_code)]
    pub async fn publish_custom(&self, topic_suffix: &str, payload: &str) -> Result<()> {
        let topic = format!("{}/{}", self.config.base_topic, topic_suffix);
        let qos = match self.config.qos {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            _ => QoS::ExactlyOnce,
        };
        
        self.client.publish(
            topic,
            qos,
            self.config.retain,
            payload.as_bytes(),
        ).await?;
        
        Ok(())
    }
    
    pub async fn publish_picture_arrival(&self, camera_id: &str, arrival_time: u128, time_diff: u128, frame_size: usize) {
        // Check if picture arrival publishing is enabled (default: true for backward compatibility)
        if !self.config.publish_picture_arrival.unwrap_or(true) {
            return;
        }
        
        let picture_event = PictureArrival {
            t: arrival_time,
            d: time_diff,
            s: frame_size,
        };
        
        if let Ok(payload) = serde_json::to_string(&picture_event) {
            let topic = format!("{}/cameras/{}/capturing", self.config.base_topic, camera_id);
            let qos = match self.config.qos {
                0 => QoS::AtMostOnce,
                1 => QoS::AtLeastOnce,
                _ => QoS::ExactlyOnce,
            };
            
            if let Err(e) = self.client.publish(
                topic,
                qos,
                false, // Don't retain picture arrival events
                payload.as_bytes(),
            ).await {
                error!("Failed to publish picture arrival for camera {}: {}", camera_id, e);
            }
        } else {
            error!("Failed to serialize picture arrival event for camera {}", camera_id);
        }
    }
    
    pub async fn publish_camera_image(&self, camera_id: &str, jpeg_data: &[u8], custom_topic: Option<&String>) -> Result<()> {
        let topic = if let Some(custom_topic) = custom_topic {
            custom_topic.clone()
        } else {
            format!("{}/cameras/{}/jpg", self.config.base_topic, camera_id)
        };
        
        let qos = match self.config.qos {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            _ => QoS::ExactlyOnce,
        };
        
        self.client.publish(
            topic,
            qos,
            false, // Don't retain image data
            jpeg_data,
        ).await?;
        
        Ok(())
    }
    
    pub async fn publish_throughput_stats(&self, camera_id: &str, stats: &ThroughputStats) -> Result<()> {
        let topic = format!("{}/cameras/{}/throughput", self.config.base_topic, camera_id);
        
        let qos = match self.config.qos {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            _ => QoS::ExactlyOnce,
        };
        
        let payload = serde_json::to_string(stats).map_err(|e| {
            StreamError::mqtt(format!("Failed to serialize throughput stats: {}", e))
        })?;
        
        self.client.publish(
            topic,
            qos,
            self.config.retain,
            payload,
        ).await.map_err(|e| {
            StreamError::mqtt(format!("Failed to publish throughput stats: {}", e))
        })?;
        
        Ok(())
    }
    
    pub async fn get_all_camera_status(&self) -> HashMap<String, CameraStatus> {
        let cameras = self.camera_status.read().await;
        cameras.clone()
    }
}