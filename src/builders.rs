use std::sync::Arc;
use tokio::sync::broadcast;
use bytes::Bytes;

use crate::config::{FfmpegConfig, RtspConfig, TranscodingConfig};
use crate::errors::{Result, StreamError};
use crate::mqtt::MqttHandle;
use crate::rtsp_client::RtspClient;

/// Builder for RtspClient to replace the complex constructor
#[allow(dead_code)]
pub struct RtspClientBuilder {
    camera_id: Option<String>,
    config: Option<RtspConfig>,
    frame_sender: Option<Arc<broadcast::Sender<Bytes>>>,
    ffmpeg_config: Option<FfmpegConfig>,
    transcoding_config: Option<TranscodingConfig>,
    capture_framerate: u32,
    debug_capture: bool,
    debug_duplicate_frames: bool,
    mqtt_handle: Option<MqttHandle>,
    latest_frame: Option<Arc<tokio::sync::RwLock<Option<Bytes>>>>,
}

#[allow(dead_code)]
impl RtspClientBuilder {
    pub fn new() -> Self {
        Self {
            camera_id: None,
            config: None,
            frame_sender: None,
            ffmpeg_config: None,
            transcoding_config: None,
            capture_framerate: 0,
            debug_capture: false,
            debug_duplicate_frames: false,
            mqtt_handle: None,
            latest_frame: None,
        }
    }

    pub fn camera_id(mut self, camera_id: String) -> Self {
        self.camera_id = Some(camera_id);
        self
    }

    pub fn config(mut self, config: RtspConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn frame_sender(mut self, frame_sender: Arc<broadcast::Sender<Bytes>>) -> Self {
        self.frame_sender = Some(frame_sender);
        self
    }

    pub fn ffmpeg_config(mut self, ffmpeg_config: Option<FfmpegConfig>) -> Self {
        self.ffmpeg_config = ffmpeg_config;
        self
    }

    pub fn capture_framerate(mut self, framerate: u32) -> Self {
        self.capture_framerate = framerate;
        self
    }



    pub fn debug_capture(mut self, debug: bool) -> Self {
        self.debug_capture = debug;
        self
    }


    pub fn mqtt_handle(mut self, mqtt_handle: Option<MqttHandle>) -> Self {
        self.mqtt_handle = mqtt_handle;
        self
    }

    pub fn latest_frame(mut self, latest_frame: Arc<tokio::sync::RwLock<Option<Bytes>>>) -> Self {
        self.latest_frame = Some(latest_frame);
        self
    }

    pub async fn build(self) -> Result<RtspClient> {
        let camera_id = self.camera_id.ok_or_else(|| StreamError::config("Camera ID is required"))?;
        let config = self.config.ok_or_else(|| StreamError::config("RTSP config is required"))?;
        let frame_sender = self.frame_sender.ok_or_else(|| StreamError::config("Frame sender is required"))?;

        let default_transcoding = TranscodingConfig {
            output_format: "mjpeg".to_string(),
            capture_framerate: 30,
            output_framerate: None,
            channel_buffer_size: Some(1024),
            debug_capture: Some(false),
            debug_duplicate_frames: Some(false),
        };
        
        let latest_frame = self.latest_frame.unwrap_or_else(|| Arc::new(tokio::sync::RwLock::new(None)));
        
        Ok(RtspClient::new_from_builder(
            camera_id,
            config,
            frame_sender,
            self.ffmpeg_config,
            self.transcoding_config.unwrap_or(default_transcoding),
            self.capture_framerate,
            self.debug_capture,
            self.debug_duplicate_frames,
            self.mqtt_handle,
            None, // Camera MQTT config not available in builder pattern
            None, // No external shutdown flag in builder pattern
            latest_frame,
        ).await)
    }
}

impl Default for RtspClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
