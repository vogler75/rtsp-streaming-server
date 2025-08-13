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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtspConfig {
    pub url: String,
    pub transport: String,
    pub reconnect_interval: u64,
    pub buffer_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscodingConfig {
    pub output_format: String,
    pub quality: u8,
    pub framerate: u32,
    pub max_latency_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
            rtsp: RtspConfig {
                url: "rtsp://admin:password@192.168.1.100:554/stream".to_string(),
                transport: "tcp".to_string(),
                reconnect_interval: 5,
                buffer_size: 1024000,
            },
            transcoding: TranscodingConfig {
                output_format: "mjpeg".to_string(),
                quality: 85,
                framerate: 30,
                max_latency_ms: 200,
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