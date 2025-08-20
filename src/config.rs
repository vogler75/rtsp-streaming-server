use serde::{Deserialize, Serialize};
use std::fs;
use std::collections::HashMap;
use std::path::Path;
use crate::errors::Result;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Mp4StorageType {
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "filesystem")]  
    Filesystem,
    #[serde(rename = "database")]
    Database,
}

impl Default for Mp4StorageType {
    fn default() -> Self {
        Self::Filesystem
    }
}

impl std::fmt::Display for Mp4StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mp4StorageType::Disabled => write!(f, "disabled"),
            Mp4StorageType::Filesystem => write!(f, "filesystem"),
            Mp4StorageType::Database => write!(f, "database"),
        }
    }
}

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
    
    // Per-camera frame storage settings
    pub frame_storage_enabled: Option<bool>, // Override global frame storage setting
    
    // Per-camera MP4 recording settings (NEW)
    pub video_storage_type: Option<Mp4StorageType>, // Override global video storage type
    pub video_storage_retention: Option<String>, // Override global video retention (e.g., "30d")
    pub video_segment_minutes: Option<u64>, // Override global segment duration
    
    // BACKWARD COMPATIBILITY: Handle old video_storage_enabled field
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_storage_enabled: Option<bool>, // For migration only
    
    #[serde(flatten)]
    pub transcoding_override: Option<TranscodingConfig>,

    // PTZ control configuration (optional)
    #[serde(default)]
    pub ptz: Option<PtzConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtzConfig {
    pub enabled: bool,
    /// PTZ protocol: currently only "onvif" is supported
    #[serde(default = "default_ptz_protocol")] 
    pub protocol: String,
    /// ONVIF service URL, e.g. http://<ip>:<port>/onvif/device_service
    pub onvif_url: Option<String>,
    /// Credentials for ONVIF HTTP digest/basic auth
    pub username: Option<String>,
    pub password: Option<String>,
    /// Optional PTZ profile token (if not provided, will try to resolve first profile)
    pub profile_token: Option<String>,
}

fn default_ptz_protocol() -> String { "onvif".to_string() }

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
    pub data_timeout_secs: Option<u64>,   // Timeout in seconds to restart FFmpeg if no data (default: 60)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub tls: Option<TlsConfig>,
    pub cors_allow_origin: Option<String>,
    pub admin_token: Option<String>,  // Optional token for admin operations
    pub cameras_directory: Option<String>,  // Directory path for camera configuration files (default: "cameras")
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
    // Frame storage settings (unchanged)
    #[serde(default)]
    pub frame_storage_enabled: bool,
    pub database_path: String,
    #[serde(default = "default_max_frame_size")]
    pub max_frame_size: usize, // Maximum frame size in bytes for database storage
    #[serde(default)]
    pub frame_storage_retention: String, // Max age for frame recordings (e.g., "10m", "5h", "7d")
    
    // NEW: MP4 video storage settings
    #[serde(default)]
    pub video_storage_type: Mp4StorageType,
    #[serde(default = "default_video_storage_retention")]
    pub video_storage_retention: String, // Max age for video recordings (e.g., "30d")
    #[serde(default = "default_video_segment_minutes")]
    pub video_segment_minutes: u64, // Duration of each video segment in minutes
    #[serde(default = "default_mp4_framerate")]
    pub mp4_framerate: f32, // Framerate for MP4 recordings (e.g., 5.0, 15.0, 30.0)
    
    // Cleanup settings
    #[serde(default = "default_cleanup_interval_hours")]
    pub cleanup_interval_hours: u64, // How often to run cleanup (default: 1 hour)
    
    // BACKWARD COMPATIBILITY: Handle old video_storage_enabled field
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_storage_enabled: Option<bool>, // For migration only
}

fn default_max_frame_size() -> usize { 10 * 1024 * 1024 } // 10MB
fn default_video_storage_retention() -> String { "30d".to_string() }
fn default_video_segment_minutes() -> u64 { 5 }
fn default_mp4_framerate() -> f32 { 5.0 }
fn default_cleanup_interval_hours() -> u64 { 1 }

impl MqttConfig {
    pub fn substitute_variables(&mut self) {
        // Get the hostname
        let hostname = gethostname::gethostname()
            .to_string_lossy()
            .to_string();
        
        // Substitute ${hostname} in base_topic
        self.base_topic = self.base_topic.replace("${hostname}", &hostname);
        
        // Substitute ${hostname} in client_id
        self.client_id = self.client_id.replace("${hostname}", &hostname);
        
        info!("MQTT config substituted: base_topic = {}, client_id = {}", 
              self.base_topic, self.client_id);
    }
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
                cameras_directory: None,  // Default: "cameras"
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
                frame_storage_enabled: false,
                database_path: "recordings".to_string(),
                max_frame_size: default_max_frame_size(),
                frame_storage_retention: "24h".to_string(),
                video_storage_type: Mp4StorageType::Disabled,
                video_storage_retention: default_video_storage_retention(),
                video_segment_minutes: default_video_segment_minutes(),
                mp4_framerate: default_mp4_framerate(),
                cleanup_interval_hours: default_cleanup_interval_hours(),
                video_storage_enabled: None, // For migration only
            }),
        }
    }
}

impl CameraConfig {
    // Get the effective video storage type for this camera
    #[allow(dead_code)]
    pub fn get_effective_video_storage_type(&self, global_config: &RecordingConfig) -> Mp4StorageType {
        self.video_storage_type
            .clone()
            .unwrap_or(global_config.video_storage_type.clone())
    }
    
    // Handle backward compatibility for video storage settings
    pub fn migrate_video_storage_config(&mut self) {
        if let Some(enabled) = self.video_storage_enabled.take() {
            if self.video_storage_type.is_none() {
                self.video_storage_type = Some(if enabled {
                    Mp4StorageType::Filesystem
                } else {
                    Mp4StorageType::Disabled
                });
            }
        }
    }
}

impl RecordingConfig {
    // Handle backward compatibility for video storage settings
    pub fn migrate_video_storage_config(&mut self) {
        if let Some(enabled) = self.video_storage_enabled.take() {
            if self.video_storage_type == Mp4StorageType::default() {
                self.video_storage_type = if enabled {
                    Mp4StorageType::Filesystem
                } else {
                    Mp4StorageType::Disabled
                };
            }
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
        
        // Handle backward compatibility migration
        if let Some(ref mut recording) = config.recording {
            recording.migrate_video_storage_config();
        }
        
        // Substitute environment variables in MQTT config
        if let Some(ref mut mqtt) = config.mqtt {
            mqtt.substitute_variables();
        }
        
        // Load cameras from the configured cameras directory (default: "cameras")
        let cameras_dir = config.server.cameras_directory.as_deref().unwrap_or("cameras");
        config.cameras = Self::load_cameras_from_directory(cameras_dir)?;
        
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
                                    Ok(mut camera_config) => {
                                        // Handle migration for this camera
                                        camera_config.migrate_video_storage_config();
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
                                    Ok(mut camera_config) => {
                                        // Handle migration for this camera
                                        camera_config.migrate_video_storage_config();
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

    pub fn save_camera_config(camera_id: &str, config: &CameraConfig, cameras_dir: Option<&str>) -> Result<()> {
        let cameras_dir = cameras_dir.unwrap_or("cameras");
        
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

    pub fn delete_camera_config(camera_id: &str, cameras_dir: Option<&str>) -> Result<()> {
        let cameras_dir = cameras_dir.unwrap_or("cameras");
        
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