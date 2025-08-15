use std::sync::Arc;
use tokio::sync::broadcast;
use bytes::Bytes;

use crate::config::{CameraConfig, FfmpegConfig, RtspConfig, TranscodingConfig};
use crate::errors::{Result, StreamError};
use crate::mqtt::MqttHandle;
use crate::rtsp_client::RtspClient;
use crate::video_stream::VideoStream;

/// Builder for RtspClient to replace the complex constructor
#[allow(dead_code)]
pub struct RtspClientBuilder {
    camera_id: Option<String>,
    config: Option<RtspConfig>,
    frame_sender: Option<Arc<broadcast::Sender<Bytes>>>,
    ffmpeg_config: Option<FfmpegConfig>,
    capture_framerate: u32,
    send_framerate: u32,
    allow_duplicate_frames: bool,
    debug_capture: bool,
    debug_sending: bool,
    mqtt_handle: Option<MqttHandle>,
}

#[allow(dead_code)]
impl RtspClientBuilder {
    pub fn new() -> Self {
        Self {
            camera_id: None,
            config: None,
            frame_sender: None,
            ffmpeg_config: None,
            capture_framerate: 0,
            send_framerate: 10,
            allow_duplicate_frames: false,
            debug_capture: false,
            debug_sending: false,
            mqtt_handle: None,
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

    pub fn send_framerate(mut self, framerate: u32) -> Self {
        self.send_framerate = framerate;
        self
    }

    pub fn allow_duplicate_frames(mut self, allow: bool) -> Self {
        self.allow_duplicate_frames = allow;
        self
    }

    pub fn debug_capture(mut self, debug: bool) -> Self {
        self.debug_capture = debug;
        self
    }

    pub fn debug_sending(mut self, debug: bool) -> Self {
        self.debug_sending = debug;
        self
    }

    pub fn mqtt_handle(mut self, mqtt_handle: Option<MqttHandle>) -> Self {
        self.mqtt_handle = mqtt_handle;
        self
    }

    pub async fn build(self) -> Result<RtspClient> {
        let camera_id = self.camera_id.ok_or_else(|| StreamError::config("Camera ID is required"))?;
        let config = self.config.ok_or_else(|| StreamError::config("RTSP config is required"))?;
        let frame_sender = self.frame_sender.ok_or_else(|| StreamError::config("Frame sender is required"))?;

        Ok(RtspClient::new_from_builder(
            camera_id,
            config,
            frame_sender,
            self.ffmpeg_config,
            self.capture_framerate,
            self.send_framerate,
            self.allow_duplicate_frames,
            self.debug_capture,
            self.debug_sending,
            self.mqtt_handle,
        ).await)
    }
}

impl Default for RtspClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for VideoStream to replace complex configuration logic
#[allow(dead_code)]
pub struct VideoStreamBuilder {
    camera_id: Option<String>,
    camera_config: Option<CameraConfig>,
    transcoding_config: Option<TranscodingConfig>,
    mqtt_handle: Option<MqttHandle>,
}

#[allow(dead_code)]
impl VideoStreamBuilder {
    pub fn new() -> Self {
        Self {
            camera_id: None,
            camera_config: None,
            transcoding_config: None,
            mqtt_handle: None,
        }
    }

    pub fn camera_id(mut self, camera_id: String) -> Self {
        self.camera_id = Some(camera_id);
        self
    }

    pub fn camera_config(mut self, camera_config: CameraConfig) -> Self {
        self.camera_config = Some(camera_config);
        self
    }

    pub fn transcoding_config(mut self, transcoding_config: TranscodingConfig) -> Self {
        self.transcoding_config = Some(transcoding_config);
        self
    }

    pub fn mqtt_handle(mut self, mqtt_handle: Option<MqttHandle>) -> Self {
        self.mqtt_handle = mqtt_handle;
        self
    }

    pub async fn build(self) -> Result<VideoStream> {
        let camera_id = self.camera_id.ok_or_else(|| StreamError::config("Camera ID is required"))?;
        let camera_config = self.camera_config.ok_or_else(|| StreamError::config("Camera config is required"))?;
        let transcoding_config = self.transcoding_config.ok_or_else(|| StreamError::config("Transcoding config is required"))?;

        VideoStream::new_from_builder(
            camera_id,
            camera_config,
            transcoding_config,
            self.mqtt_handle,
        ).await
    }
}

impl Default for VideoStreamBuilder {
    fn default() -> Self {
        Self::new()
    }
}