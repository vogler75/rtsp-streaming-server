use serde::{Deserialize, Serialize};
use std::fs;
use std::collections::HashMap;
use crate::errors::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub cameras: HashMap<String, CameraConfig>,
    pub transcoding: TranscodingConfig,
    pub mqtt: Option<MqttConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraConfig {
    pub enabled: Option<bool>,
    pub path: String,
    pub url: String,
    pub transport: String,
    pub reconnect_interval: u64,
    pub chunk_read_size: Option<usize>,
    pub token: Option<String>,
    pub ffmpeg: Option<FfmpegConfig>,
    #[serde(flatten)]
    pub transcoding_override: Option<TranscodingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfmpegConfig {
    // Command override - if set, replaces all other FFmpeg options
    pub command: Option<String>,          // Full FFmpeg command (without 'ffmpeg' prefix)
    
    // Input timing options
    pub use_wallclock_as_timestamps: Option<bool>, // -use_wallclock_as_timestamps 1 (must be first option)
    
    // Output format and codec settings
    pub output_format: Option<String>,    // -f (e.g., "mjpeg", "mpegts", "mp4")
    pub video_codec: Option<String>,      // -codec:v (e.g., "mpeg1video", "libx264")
    pub video_bitrate: Option<String>,    // -b:v (e.g., "200k", "1M")
    pub quality: Option<u8>,              // -q:v (JPEG quality 1-100)
    pub output_framerate: Option<u32>,    // -r (output framerate)
    pub gop_size: Option<u32>,            // -g (GOP size, keyframe interval)
    pub scale: Option<String>,            // -vf scale (e.g., "640:480", "1280:-1")
    pub movflags: Option<String>,         // -movflags (e.g., "frag_keyframe+empty_moov+default_base_moof" for fMP4)
    
    // Buffer and performance settings
    pub rtbufsize: Option<usize>,         // -rtbufsize (RTSP buffer size in bytes)
    
    // FFmpeg flags and options
    pub fflags: Option<String>,
    pub flags: Option<String>,
    pub avioflags: Option<String>,
    pub fps_mode: Option<String>,
    pub flush_packets: Option<String>,
    pub extra_input_args: Option<Vec<String>>,
    pub extra_output_args: Option<Vec<String>>,
    
    // Logging
    pub log_stderr: Option<String>,       // FFmpeg stderr logging: "file", "console", "both"
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscodingConfig {
    pub output_format: String,
    pub capture_framerate: u32,  // FFmpeg capture rate from camera
    pub send_framerate: u32,     // Rate at which we send frames to clients
    pub channel_buffer_size: Option<usize>, // Number of frames to buffer (1 = only latest)
    pub allow_duplicate_frames: Option<bool>, // Whether to send same frame multiple times
    pub debug_capture: Option<bool>, // Enable/disable capture rate debug output
    pub debug_sending: Option<bool>, // Enable/disable sending rate debug output
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    pub enabled: bool,
    pub broker_url: String,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub base_topic: String,
    pub qos: u8,
    pub retain: bool,
    pub keep_alive_secs: u64,
    pub publish_interval_secs: u64,
    pub publish_picture_arrival: Option<bool>, // Enable/disable picture arrival publishing
}

impl Default for Config {
    fn default() -> Self {
        let cameras = HashMap::new(); // No default cameras - must be configured
        
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
            cameras,
            transcoding: TranscodingConfig {
                output_format: "mjpeg".to_string(),
                capture_framerate: 30,
                send_framerate: 10,
                channel_buffer_size: Some(1),
                allow_duplicate_frames: Some(false),
                debug_capture: Some(true),
                debug_sending: Some(true),
            },
            mqtt: None,
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