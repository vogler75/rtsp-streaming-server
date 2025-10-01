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
    pub recording: Option<CameraRecordingConfig>,
    
    #[serde(flatten)]
    pub transcoding_override: Option<TranscodingConfig>,

    // PTZ control configuration (optional)
    #[serde(default)]
    pub ptz: Option<PtzConfig>,
}

impl CameraConfig {
    /// Get the effective session segment minutes setting
    pub fn get_session_segment_minutes(&self) -> Option<u64> {
        self.recording.as_ref()?.session_segment_minutes
    }
    
    /// Get the effective frame storage enabled setting
    pub fn get_frame_storage_enabled(&self) -> Option<bool> {
        self.recording.as_ref()?.frame_storage_enabled
    }
    
    /// Get the effective frame storage retention setting
    pub fn get_frame_storage_retention(&self) -> Option<&String> {
        self.recording.as_ref()?.frame_storage_retention.as_ref()
    }
    
    /// Get the effective video storage type
    pub fn get_mp4_storage_type(&self) -> Option<&Mp4StorageType> {
        self.recording.as_ref()?.mp4_storage_type.as_ref()
    }
    
    /// Get the effective video storage retention setting
    pub fn get_mp4_storage_retention(&self) -> Option<&String> {
        self.recording.as_ref()?.mp4_storage_retention.as_ref()
    }
    
    /// Get the effective video segment minutes setting
    pub fn get_mp4_segment_minutes(&self) -> Option<u64> {
        self.recording.as_ref()?.mp4_segment_minutes
    }
    
    /// Get the effective HLS storage enabled setting
    pub fn get_hls_storage_enabled(&self) -> Option<bool> {
        self.recording.as_ref()?.hls_storage_enabled
    }
    
    /// Get the effective HLS storage retention setting
    pub fn get_hls_storage_retention(&self) -> Option<&String> {
        self.recording.as_ref()?.hls_storage_retention.as_ref()
    }
    
    /// Get the effective HLS segment seconds setting
    pub fn get_hls_segment_seconds(&self) -> Option<u64> {
        self.recording.as_ref()?.hls_segment_seconds
    }
    
    /// Get the effective pre-recording enabled setting
    pub fn get_pre_recording_enabled(&self) -> Option<bool> {
        self.recording.as_ref()?.pre_recording_enabled
    }
    
    /// Get the effective pre-recording buffer duration setting
    pub fn get_pre_recording_buffer_minutes(&self) -> Option<u64> {
        self.recording.as_ref()?.pre_recording_buffer_minutes
    }
    
    /// Get the effective pre-recording cleanup interval setting
    pub fn get_pre_recording_cleanup_interval_seconds(&self) -> Option<u64> {
        self.recording.as_ref()?.pre_recording_cleanup_interval_seconds
    }
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
    pub publish_interval: u64, // Interval in milliseconds, 0 = publish every frame
    pub topic_name: Option<String>, // Optional custom topic name, defaults to <base_topic>/cameras/<cam-name>/jpg
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraRecordingConfig {
    // General settings
    pub session_segment_minutes: Option<u64>, // Override global session segmentation (None=use global, 0=disabled, n=minutes)
    
    // Pre-recording buffer settings (memory-only)
    pub pre_recording_enabled: Option<bool>, // Override global pre-recording enabled setting
    pub pre_recording_buffer_minutes: Option<u64>, // Override global buffer duration
    pub pre_recording_cleanup_interval_seconds: Option<u64>, // Override global cleanup interval
    
    // Frame storage settings
    pub frame_storage_enabled: Option<bool>, // Override global frame storage setting
    pub frame_storage_retention: Option<String>, // Override global frame retention (e.g., "10m", "5h", "24h")
    
    // MP4 recording settings
    pub mp4_storage_type: Option<Mp4StorageType>, // Override global video storage type
    pub mp4_storage_retention: Option<String>, // Override global video retention (e.g., "30d")
    pub mp4_segment_minutes: Option<u64>, // Override global segment duration
    
    // HLS storage settings
    pub hls_storage_enabled: Option<bool>, // Override global HLS storage setting
    pub hls_storage_retention: Option<String>, // Override global HLS retention (e.g., "30d")
    pub hls_segment_seconds: Option<u64>, // Override global HLS segment duration in seconds
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DatabaseType {
    #[serde(rename = "sqlite")]
    SQLite,
    #[serde(rename = "postgresql")]
    PostgreSQL,
}

impl Default for DatabaseType {
    fn default() -> Self {
        Self::SQLite
    }
}

impl std::fmt::Display for DatabaseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseType::SQLite => write!(f, "sqlite"),
            DatabaseType::PostgreSQL => write!(f, "postgresql"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    // Frame storage settings (unchanged)
    #[serde(default)]
    pub frame_storage_enabled: bool,
    pub database_path: String,
    
    // Database configuration
    #[serde(default)]
    pub database_type: DatabaseType,
    pub database_url: Option<String>, // PostgreSQL connection string (e.g., "postgres://user:pass@localhost/")
    
    #[serde(default = "default_session_segment_minutes")]
    pub session_segment_minutes: u64, // Duration for session segmentation in minutes (default: 60)
    #[serde(default = "default_max_frame_size")]
    pub max_frame_size: usize, // Maximum frame size in bytes for database storage
    #[serde(default)]
    pub frame_storage_retention: String, // Max age for frame recordings (e.g., "10m", "5h", "7d")
    
    // Pre-recording buffer settings (memory-only)
    #[serde(default)]
    pub pre_recording_enabled: bool, // Enable pre-recording buffer
    #[serde(default = "default_pre_recording_buffer_minutes")]
    pub pre_recording_buffer_minutes: u64, // Buffer duration in minutes
    #[serde(default = "default_pre_recording_cleanup_interval_seconds")]
    pub pre_recording_cleanup_interval_seconds: u64, // How often to cleanup buffer frames
    
    // NEW: MP4 video storage settings
    #[serde(default)]
    pub mp4_storage_type: Mp4StorageType,
    #[serde(default = "default_mp4_storage_retention")]
    pub mp4_storage_retention: String, // Max age for video recordings (e.g., "30d")
    #[serde(default = "default_mp4_segment_minutes")]
    pub mp4_segment_minutes: u64, // Duration of each video segment in minutes

    // HLS storage settings
    #[serde(default)]
    pub hls_storage_enabled: bool, // Enable HLS segment storage in database
    #[serde(default = "default_hls_storage_retention")]
    pub hls_storage_retention: String, // Max age for HLS recordings (e.g., "30d")
    #[serde(default = "default_hls_segment_seconds")]
    pub hls_segment_seconds: u64, // Duration of each HLS segment in seconds
    
    // Cleanup settings
    #[serde(default = "default_cleanup_interval_hours")]
    pub cleanup_interval_hours: u64, // How often to run cleanup (default: 1 hour)
}

fn default_max_frame_size() -> usize { 10 * 1024 * 1024 } // 10MB
fn default_session_segment_minutes() -> u64 { 60 } // 60 minutes (1 hour)
fn default_pre_recording_buffer_minutes() -> u64 { 1 } // 5 minutes default buffer
fn default_pre_recording_cleanup_interval_seconds() -> u64 { 1 } // Check every 1 second
fn default_mp4_storage_retention() -> String { "30d".to_string() }
fn default_mp4_segment_minutes() -> u64 { 5 }
fn default_hls_storage_retention() -> String { "30d".to_string() }
fn default_hls_segment_seconds() -> u64 { 6 }
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
                debug_capture: Some(false),
                debug_duplicate_frames: Some(false),
            },
            mqtt: Some(MqttConfig {
                enabled: false,
                broker_url: "mqtt://localhost:1883".to_string(),
                client_id: "".to_string(),
                username: None,
                password: None,
                base_topic: "Videoserver/${hostname}".to_string(),
                qos: 0,
                retain: false,
                keep_alive_secs: 60,
                publish_interval_secs: 5,
                publish_picture_arrival: Some(false),
                max_packet_size: None,
            }),
            recording: Some(RecordingConfig {
                frame_storage_enabled: false,
                database_path: "recordings".to_string(),
                database_type: DatabaseType::SQLite,
                database_url: None,
                session_segment_minutes: default_session_segment_minutes(),
                max_frame_size: default_max_frame_size(),
                frame_storage_retention: "24h".to_string(),
                pre_recording_enabled: false,
                pre_recording_buffer_minutes: default_pre_recording_buffer_minutes(),
                pre_recording_cleanup_interval_seconds: default_pre_recording_cleanup_interval_seconds(),
                mp4_storage_type: Mp4StorageType::Disabled,
                mp4_storage_retention: default_mp4_storage_retention(),
                mp4_segment_minutes: default_mp4_segment_minutes(),
                cleanup_interval_hours: default_cleanup_interval_hours(),
                hls_storage_enabled: false,
                hls_storage_retention: default_hls_storage_retention(),
                hls_segment_seconds: default_hls_segment_seconds(),
            }),
        }
    }
}


impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let mut config: Config = serde_json::from_str(&content)?;
        
        
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
        
        // Read all .json files in the cameras directory
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
        
        let json_path = format!("{}/{}.json", cameras_dir, camera_id);
        
        if Path::new(&json_path).exists() {
            fs::remove_file(&json_path)?;
            info!("Deleted camera configuration: {} (JSON)", camera_id);
            Ok(())
        } else {
            Err(crate::errors::StreamError::config(&format!("Camera configuration file not found: {}", camera_id)).into())
        }
    }

}