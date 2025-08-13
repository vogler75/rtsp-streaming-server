use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub rtsp: RtspConfig,
    pub transcoding: TranscodingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub tls: Option<TlsConfig>,
    pub cors_allow_origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: String,
    pub key_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtspConfig {
    pub url: String,
    pub transport: String,
    pub reconnect_interval: u64,
    pub chunk_read_size: Option<usize>,
    pub ffmpeg_buffer_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscodingConfig {
    pub output_format: String,
    pub quality: u8,
    pub capture_framerate: u32,  // FFmpeg capture rate from camera
    pub send_framerate: u32,     // Rate at which we send frames to clients
    pub channel_buffer_size: Option<usize>, // Number of frames to buffer (1 = only latest)
    pub allow_duplicate_frames: Option<bool>, // Whether to send same frame multiple times
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
                tls: Some(TlsConfig {
                    enabled: false,
                    cert_path: "certs/server.crt".to_string(),
                    key_path: "certs/server.key".to_string(),
                }),
                cors_allow_origin: Some("*".to_string()),
            },
            rtsp: RtspConfig {
                url: "rtsp://admin:password@192.168.1.100:554/stream".to_string(),
                transport: "tcp".to_string(),
                reconnect_interval: 5,
                chunk_read_size: None,
                ffmpeg_buffer_size: None,
            },
            transcoding: TranscodingConfig {
                output_format: "mjpeg".to_string(),
                quality: 85,
                capture_framerate: 30,
                send_framerate: 10,
                channel_buffer_size: Some(1),
                allow_duplicate_frames: Some(false),
            },
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

}