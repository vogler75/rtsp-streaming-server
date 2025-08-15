use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, error};
use bytes::Bytes;

use crate::config::{CameraConfig, TranscodingConfig, RtspConfig};
use crate::errors::Result;
use crate::rtsp_client::RtspClient;
use crate::mqtt::MqttHandle;

pub struct VideoStream {
    pub camera_id: String,
    pub frame_sender: Arc<broadcast::Sender<Bytes>>,
    rtsp_client: RtspClient,
}

impl VideoStream {
    pub async fn new(
        camera_id: String,
        camera_config: CameraConfig,
        default_transcoding: &TranscodingConfig,
        mqtt_handle: Option<MqttHandle>,
    ) -> Result<Self> {
        Self::new_from_builder(camera_id, camera_config, default_transcoding.clone(), mqtt_handle).await
    }

    pub async fn new_from_builder(
        camera_id: String,
        camera_config: CameraConfig,
        default_transcoding: TranscodingConfig,
        mqtt_handle: Option<MqttHandle>,
    ) -> Result<Self> {
        // Use camera-specific transcoding config if available, otherwise use default
        let transcoding = camera_config.transcoding_override.as_ref().unwrap_or(&default_transcoding);
        
        let channel_buffer_size = transcoding.channel_buffer_size.unwrap_or(1);
        info!("Creating video stream for camera '{}' on path '{}' with buffer size: {} frames", 
              camera_id, camera_config.path, channel_buffer_size);
        
        let (frame_tx, _) = broadcast::channel(channel_buffer_size);
        let frame_tx = Arc::new(frame_tx);
        
        // Create RtspConfig from camera config
        let rtsp_config = RtspConfig {
            url: camera_config.url.clone(),
            transport: camera_config.transport.clone(),
            reconnect_interval: camera_config.reconnect_interval,
            chunk_read_size: camera_config.chunk_read_size,
        };
        
        let rtsp_client = RtspClient::new(
            camera_id.clone(),
            rtsp_config,
            frame_tx.clone(),
            camera_config.ffmpeg.clone(),
            transcoding.capture_framerate,
            transcoding.send_framerate,
            transcoding.allow_duplicate_frames.unwrap_or(false),
            transcoding.debug_capture.unwrap_or(true),
            transcoding.debug_sending.unwrap_or(true),
            transcoding.debug_duplicate_frames.unwrap_or(false),
            mqtt_handle,
        ).await;
        
        Ok(Self {
            camera_id,
            frame_sender: frame_tx,
            rtsp_client,
        })
    }
    
    pub async fn start(self) {
        let camera_id = self.camera_id.clone();
        tokio::spawn(async move {
            info!("Starting video stream for camera '{}'", camera_id);
            if let Err(e) = self.rtsp_client.start().await {
                error!("RTSP client error for camera '{}': {}", camera_id, e);
            }
        });
    }
}