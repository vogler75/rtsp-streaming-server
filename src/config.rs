use serde::{Deserialize, Serialize};
use std::fs;
use std::collections::HashMap;
use std::path::Path;
use crate::errors::Result;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub cameras: HashMap<String, CameraConfig>,
    pub transcoding: TranscodingConfig,
    pub mqtt: Option<MqttConfig>,
    pub recording: Option<RecordingConfig>,
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
    pub mqtt: Option<CameraMqttConfig>,
    pub max_recording_age: Option<String>, // Override max age for this camera (e.g., "10m", "5h", "7d")
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
    
    // Timeout and restart settings
    pub data_timeout_secs: Option<u64>,   // Timeout in seconds to restart FFmpeg if no data (default: 5)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub tls: Option<TlsConfig>,
    pub cors_allow_origin: Option<String>,
    pub admin_token: Option<String>,  // Optional token for admin operations
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
    pub output_framerate: Option<u32>, // Output framerate (can be overridden per camera)
    pub channel_buffer_size: Option<usize>, // Number of frames to buffer (1 = only latest)
    pub debug_capture: Option<bool>, // Enable/disable capture rate debug output
    pub debug_duplicate_frames: Option<bool>, // Enable/disable duplicate frame warnings
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
    pub max_packet_size: Option<usize>, // Maximum MQTT packet size in bytes (default: 268435455)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraMqttConfig {
    pub publish_interval: u64, // Interval in seconds, 0 = publish every frame
    pub topic_name: Option<String>, // Optional custom topic name, defaults to <base_topic>/cameras/<cam-name>/jpg
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub enabled: bool,
    pub database_path: String,
    pub max_frame_size: Option<usize>, // Maximum frame size in bytes for database storage
    pub max_recording_age: Option<String>, // Max age for recordings (e.g., "10m", "5h", "7d")
    pub cleanup_interval_hours: Option<u64>, // How often to run cleanup (default: 1 hour)
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
                admin_token: None,
            },
            cameras,
            transcoding: TranscodingConfig {
                output_format: "mjpeg".to_string(),
                capture_framerate: 30,
                output_framerate: None,
                channel_buffer_size: Some(1024),
                debug_capture: Some(true),
                debug_duplicate_frames: Some(false),
            },
            mqtt: None,
            recording: Some(RecordingConfig {
                enabled: false,
                database_path: "recordings.db".to_string(),
                max_frame_size: Some(10 * 1024 * 1024), // 10MB
                max_recording_age: None,
                cleanup_interval_hours: Some(1),
            }),
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let mut config: Config = if path.ends_with(".json") {
            serde_json::from_str(&content)?
        } else {
            toml::from_str(&content)?
        };
        
        // Load cameras from the cameras directory
        config.cameras = Self::load_cameras_from_directory("cameras")?;
        
        Ok(config)
    }

    fn load_cameras_from_directory(cameras_dir: &str) -> Result<HashMap<String, CameraConfig>> {
        let mut cameras = HashMap::new();
        
        // Check if cameras directory exists
        if !Path::new(cameras_dir).exists() {
            eprintln!("Warning: cameras directory '{}' does not exist, no cameras will be loaded", cameras_dir);
            return Ok(cameras);
        }
        
        // Read all .json and .toml files in the cameras directory (for backward compatibility)
        let entries = fs::read_dir(cameras_dir)?;
        
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            
            if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
                match path.extension().and_then(|s| s.to_str()) {
                    Some("json") => {
                        match fs::read_to_string(&path) {
                            Ok(content) => {
                                match serde_json::from_str::<CameraConfig>(&content) {
                                    Ok(camera_config) => {
                                        info!("Loaded camera configuration: {} (JSON)", file_stem);
                                        cameras.insert(file_stem.to_string(), camera_config);
                                    }
                                    Err(e) => {
                                        eprintln!("Error parsing JSON camera config file {}: {}", path.display(), e);
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Error reading camera config file {}: {}", path.display(), e);
                            }
                        }
                    }
                    Some("toml") => {
                        // Backward compatibility with TOML files
                        match fs::read_to_string(&path) {
                            Ok(content) => {
                                match toml::from_str::<CameraConfig>(&content) {
                                    Ok(camera_config) => {
                                        info!("Loaded camera configuration: {} (TOML)", file_stem);
                                        cameras.insert(file_stem.to_string(), camera_config);
                                    }
                                    Err(e) => {
                                        eprintln!("Error parsing TOML camera config file {}: {}", path.display(), e);
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Error reading camera config file {}: {}", path.display(), e);
                            }
                        }
                    }
                    _ => {
                        // Skip non-config files
                    }
                }
            }
        }
        
        Ok(cameras)
    }

    pub fn save_camera_config(camera_id: &str, config: &CameraConfig) -> Result<()> {
        let cameras_dir = "cameras";
        
        // Ensure cameras directory exists
        if !Path::new(cameras_dir).exists() {
            fs::create_dir_all(cameras_dir)?;
        }
        
        let file_path = format!("{}/{}.json", cameras_dir, camera_id);
        let json_content = serde_json::to_string_pretty(config)?;
        fs::write(&file_path, json_content)?;
        
        info!("Saved camera configuration: {} to {}", camera_id, file_path);
        Ok(())
    }

    pub fn delete_camera_config(camera_id: &str) -> Result<()> {
        let cameras_dir = "cameras";
        
        // Try to delete both JSON and TOML files for backward compatibility
        let json_path = format!("{}/{}.json", cameras_dir, camera_id);
        let toml_path = format!("{}/{}.toml", cameras_dir, camera_id);
        
        let mut deleted = false;
        
        if Path::new(&json_path).exists() {
            fs::remove_file(&json_path)?;
            deleted = true;
            info!("Deleted camera configuration: {} (JSON)", camera_id);
        }
        
        if Path::new(&toml_path).exists() {
            fs::remove_file(&toml_path)?;
            deleted = true;
            info!("Deleted camera configuration: {} (TOML)", camera_id);
        }
        
        if !deleted {
            return Err(crate::errors::StreamError::config(&format!("Camera configuration file not found: {}", camera_id)).into());
        }
        
        Ok(())
    }

}