use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::broadcast;
use tracing::{info, warn, error, trace};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::fmt::format::{Writer, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;
use std::fs::File;
use std::{io::BufReader};
use axum::{response::IntoResponse, Json};
use clap::{Parser};

mod config;
mod errors;
mod builders;
mod rtsp_client;
mod websocket_handler;
mod transcoder;
mod video_stream;
mod mqtt;
mod database;
mod recording;
mod websocket_control;
mod api_config;
mod api_recording;
mod watcher;
mod camera_manager;
mod mp4;
mod handlers;
mod pre_recording_buffer;
mod throughput_tracker;
mod ptz;
mod api_ptz;

use config::Config;
use errors::{Result, StreamError};
use api_recording::ApiResponse;

// Custom formatter to remove "rtsp_streaming_server::" prefix and pad to 80 chars
struct CustomFormatter;

impl<S, N> FormatEvent<S, N> for CustomFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let metadata = event.metadata();
        
        // Format timestamp
        write!(writer, "{} ", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ"))?;
        
        // Format level with color
        let level = metadata.level();
        let level_str = match *level {
            tracing::Level::ERROR => "\x1b[31mERROR\x1b[0m", // Red
            tracing::Level::WARN => "\x1b[33m WARN\x1b[0m",  // Yellow
            tracing::Level::INFO => "\x1b[32m INFO\x1b[0m",  // Green
            tracing::Level::DEBUG => "\x1b[36mDEBUG\x1b[0m", // Cyan
            tracing::Level::TRACE => "\x1b[37mTRACE\x1b[0m", // White
        };
        write!(writer, "{} ", level_str)?;
        
        // Format target with prefix removal and padding
        let target = metadata.target();
        let clean_target = target.strip_prefix("rtsp_streaming_server::").unwrap_or(target);
        let clean_target = if clean_target == "rtsp_streaming_server" { "main" } else { clean_target };
        write!(writer, "{:<40}: ", clean_target)?;
        
        // Format the message
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}
use video_stream::VideoStream;
use mqtt::{MqttPublisher, MqttHandle};
// Import removed - now using database::create_database_provider
use recording::RecordingManager;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.json")]
    config: String,
    
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
    
    /// Enable throughput tracking and database logging
    #[arg(long)]
    throughput: bool,
}

#[derive(Debug, Clone)]
pub struct Mp4BufferStats {
    pub frame_count: usize,
    pub size_bytes: usize,
}

impl Mp4BufferStats {
    pub fn new() -> Self {
        Self {
            frame_count: 0,
            size_bytes: 0,
        }
    }
    
    pub fn size_kb(&self) -> u64 {
        (self.size_bytes as f64 / 1024.0).round() as u64
    }
}

#[derive(Clone)]
struct CameraStreamInfo {
    camera_id: String,
    frame_sender: Arc<broadcast::Sender<bytes::Bytes>>,
    mqtt_handle: Option<MqttHandle>,
    camera_config: config::CameraConfig,
    recording_manager: Option<Arc<RecordingManager>>,
    task_handle: Option<Arc<tokio::task::JoinHandle<()>>>,
    capture_fps: Arc<tokio::sync::RwLock<f32>>, // Shared FPS counter from RtspClient
    pre_recording_buffer: Option<crate::pre_recording_buffer::PreRecordingBuffer>,
    mp4_buffer_stats: Arc<tokio::sync::RwLock<Mp4BufferStats>>, // MP4 buffer statistics
}

fn generate_random_token(length: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

fn save_config_to_file(config: &Config, path: &str) -> Result<()> {
    // Don't include cameras in the saved config (they're loaded from cameras/ directory)
    let mut config_to_save = config.clone();
    config_to_save.cameras.clear();
    
    let json = serde_json::to_string_pretty(&config_to_save)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[derive(Clone)]
pub struct AppState {
    camera_streams: Arc<tokio::sync::RwLock<HashMap<String, CameraStreamInfo>>>,
    pub camera_configs: Arc<tokio::sync::RwLock<HashMap<String, config::CameraConfig>>>, // All camera configs (enabled and disabled)
    mqtt_handle: Option<MqttHandle>,
    pub recording_manager: Option<Arc<RecordingManager>>,
    transcoding_config: Arc<config::TranscodingConfig>,
    pub recording_config: Option<Arc<config::RecordingConfig>>,
    pub admin_token: Option<String>,
    pub cameras_directory: String,
    start_time: std::time::Instant,
    pub server_config: Arc<config::ServerConfig>, // Store full server config for API access
}

// CreateCameraRequest moved to api::admin


#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() -> Result<()> {
    // Parse command line arguments first to get verbose flag
    let args = Args::parse();
    
    // Configure logging based on verbose flag
    let log_level = if args.verbose {
        // Enable verbose logs for our crate and ONVIF PTZ target
        "rtsp_streaming_server=trace,ptz_onvif=trace"
    } else {
        "rtsp_streaming_server=info"
    };
    
    // Custom formatter to pad target names and remove prefix
    let fmt_layer = tracing_subscriber::fmt::layer()
        .event_format(CustomFormatter)
        .fmt_fields(tracing_subscriber::fmt::format::DefaultFields::new());
    
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(log_level))
        .with(fmt_layer)
        .init();

    let config = match Config::load(&args.config) {
        Ok(cfg) => {
            info!("Loaded configuration from {}", args.config);
            cfg
        }
        Err(e) => {
            warn!("Could not load configuration from {}: {}", args.config, e);
            info!("Starting with minimal configuration - no cameras configured");
            
            // Generate a random admin token for initial access
            let admin_token = generate_random_token(32);
            info!("========================================");
            info!("Generated admin token: {}", admin_token);
            info!("Use this token to access /dashboard for admin interface");
            info!("This token has been saved to {}", args.config);
            info!("========================================");
            
            let mut default_config = Config::default();
            default_config.server.admin_token = Some(admin_token);
            
            // Save the generated config to disk
            match save_config_to_file(&default_config, &args.config) {
                Ok(_) => info!("Saved default configuration to {}", args.config),
                Err(save_err) => error!("Failed to save configuration to {}: {}", args.config, save_err),
            }
            
            default_config
        }
    };

    info!("Starting RTSP streaming server on {}:{}", config.server.host, config.server.port);
    
    // Check and create required directories
    // 1. Check cameras directory
    let cameras_dir = config.server.cameras_directory.as_deref().unwrap_or("cameras");
    match std::fs::create_dir_all(cameras_dir) {
        Ok(_) => {
            info!("Cameras directory '{}' is ready", cameras_dir);
        }
        Err(e) => {
            error!("Failed to create cameras directory '{}': {}", cameras_dir, e);
            error!("The server cannot start without access to the cameras configuration directory");
            std::process::exit(1);
        }
    }
    
    // 2. Check recordings directory (if recording is configured)
    if let Some(ref recording_config) = config.recording {
        let recordings_dir = &recording_config.database_path;
        match std::fs::create_dir_all(recordings_dir) {
            Ok(_) => {
                info!("Recordings directory '{}' is ready", recordings_dir);
            }
            Err(e) => {
                error!("Failed to create recordings directory '{}': {}", recordings_dir, e);
                error!("The server cannot start without access to the recordings directory");
                std::process::exit(1);
            }
        }
    }
    
    // Check FFmpeg availability
    match tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
    {
        Ok(output) => {
            if output.status.success() {
                let version_output = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = version_output.lines().next() {
                    info!("FFmpeg found: {}", first_line);
                } else {
                    info!("FFmpeg is available");
                }
            } else {
                error!("FFmpeg is installed but failed to run: {}", String::from_utf8_lossy(&output.stderr));
                std::process::exit(1);
            }
        }
        Err(e) => {
            error!("FFmpeg is not available on the system PATH: {}", e);
            error!("Please install FFmpeg and ensure it's available in your PATH");
            std::process::exit(1);
        }
    }
    
    // Cleanup old HLS directories from previous runs
    mp4::cleanup_old_hls_directories().await;

    // Initialize MQTT if enabled
    let mqtt_handle: Option<MqttHandle> = if let Some(mqtt_config) = config.mqtt.clone() {
        if mqtt_config.enabled {
            info!("Initializing MQTT connection to {}", mqtt_config.broker_url);
            match MqttPublisher::new(mqtt_config).await {
                Ok(publisher) => {
                    match publisher.start().await {
                        Ok(handle) => {
                            info!("MQTT publisher started successfully");
                            Some(handle)
                        }
                        Err(e) => {
                            error!("Failed to start MQTT publisher: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to create MQTT publisher: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Initialize recording manager if enabled
    let recording_manager: Option<Arc<RecordingManager>> = if let Some(recording_config) = &config.recording {
        if recording_config.frame_storage_enabled || recording_config.mp4_storage_type != config::Mp4StorageType::Disabled {
            info!("Initializing recording system with database directory: {}", recording_config.database_path);
            
            // Directory already created and verified earlier
            match RecordingManager::new(Arc::new(recording_config.clone())).await {
                Ok(manager) => {
                    info!("Recording system initialized successfully");
                    // Initialize with camera configs for cleanup purposes
                    manager.update_camera_configs(config.cameras.clone()).await;
                    let manager = Arc::new(manager);
                        
                    // Start cleanup task if frame_storage_retention is configured
                    if !recording_config.frame_storage_retention.is_empty() && recording_config.frame_storage_retention != "0" {
                        let manager_clone = manager.clone();
                        let cleanup_interval = recording_config.cleanup_interval_hours;
                        tokio::spawn(async move {
                            let mut interval = tokio::time::interval(
                                tokio::time::Duration::from_secs(cleanup_interval * 3600)
                            );
                            
                            loop {
                                interval.tick().await;
                                if let Err(e) = manager_clone.cleanup_task().await {
                                    error!("Failed to cleanup recordings: {}", e);
                                }
                            }
                        });
                    }
                        
                    Some(manager)
                }
                Err(e) => {
                    error!("Failed to initialize recording manager: {}", e);
                    None
                }
            }
        } else {
            info!("Recording system disabled in configuration");
            None
        }
    } else {
        None
    };

    // Initialize throughput tracker if MQTT is enabled (always publish to MQTT) or --throughput flag is set (database logging)
    let throughput_tracker: Option<Arc<throughput_tracker::ThroughputTracker>> = 
        if mqtt_handle.is_some() || args.throughput {
            let tracker = Arc::new(throughput_tracker::ThroughputTracker::new_with_mqtt(mqtt_handle.clone(), args.throughput));
            
            // Start the throughput tracking task
            let tracker_clone = tracker.clone();
            tokio::spawn(async move {
                let _ = tracker_clone.start_tracking_task().await;
            });
            
            match (mqtt_handle.is_some(), args.throughput) {
                (true, true) => info!("Throughput tracker initialized: MQTT publishing + database logging enabled"),
                (true, false) => info!("Throughput tracker initialized: MQTT publishing enabled, database logging disabled"),
                (false, true) => info!("Throughput tracker initialized: Database logging enabled, MQTT publishing disabled"),
                (false, false) => info!("Throughput tracker initialized: No publishing/logging enabled"), // This shouldn't happen
            }
            
            // Set as global tracker for easy access throughout the application
            throughput_tracker::set_global_tracker(tracker.clone());
            
            Some(tracker)
        } else {
            None
        };

    // Store all camera configurations (enabled and disabled)
    let all_camera_configs = config.cameras.clone();
    
    // Create video streams only for enabled cameras
    let mut camera_streams: HashMap<String, CameraStreamInfo> = HashMap::new();
    
    for (camera_id, camera_config) in config.cameras.clone() {
        // Check if camera is enabled (default to true if not specified)
        let is_enabled = camera_config.enabled.unwrap_or(true);
        if !is_enabled {
            info!("Camera '{}' is disabled, loading config but not starting stream", camera_id);
            continue;
        }
        
        info!("Configuring camera '{}' on path '{}'...", camera_id, camera_config.path);
        
        match VideoStream::new(
            camera_id.clone(),
            camera_config.clone(),
            &config.transcoding,
            mqtt_handle.clone(),
            config.recording.as_ref(),
        ).await {
            Ok(video_stream) => {
                // Create database for this camera if recording is enabled
                if let Some(ref recording_manager_ref) = recording_manager {
                    if let Some(recording_config) = &config.recording {
                        info!("Creating {} database for camera '{}'", recording_config.database_type, camera_id);
                        
                        match database::create_database_provider(recording_config, Some(&camera_id)).await {
                            Ok(database) => {
                                if let Err(e) = recording_manager_ref.add_camera_database(&camera_id, database.clone()).await {
                                    error!("Failed to add database for camera '{}': {}", camera_id, e);
                                } else {
                                    info!("Database created successfully for camera '{}'", camera_id);
                                    
                                    // Also add database to throughput tracker if available and throughput flag is set
                                    if let Some(ref throughput_tracker_ref) = throughput_tracker {
                                        if args.throughput {
                                            throughput_tracker_ref.add_camera_database(&camera_id, database.clone()).await;
                                        }
                                        throughput_tracker_ref.register_camera(&camera_id).await;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to create database for camera '{}': {}", camera_id, e);
                            }
                        }
                    }
                }

                // Extract frame sender, FPS counter, and pre-recording buffer before starting (since start() consumes the video_stream)
                let frame_sender = video_stream.frame_sender.clone();
                let fps_counter = video_stream.get_fps_counter();
                let pre_recording_buffer = video_stream.pre_recording_buffer.clone();
                
                // Create MP4 buffer stats for this camera
                let mp4_buffer_stats = Arc::new(tokio::sync::RwLock::new(Mp4BufferStats::new()));
                
                // Register MP4 buffer stats with recording manager if available
                if let Some(ref recording_manager_ref) = recording_manager {
                    recording_manager_ref.register_mp4_buffer_stats(&camera_id, mp4_buffer_stats.clone()).await;
                }
                
                // Start the video stream and get the task handle
                let task_handle = video_stream.start().await;
                
                // Store the camera stream info for this camera's path
                camera_streams.insert(camera_config.path.clone(), CameraStreamInfo {
                    camera_id: camera_id.clone(),
                    frame_sender,
                    mqtt_handle: mqtt_handle.clone(),
                    camera_config: camera_config.clone(),
                    recording_manager: recording_manager.clone(),
                    task_handle: Some(Arc::new(task_handle)),
                    capture_fps: fps_counter,
                    pre_recording_buffer,
                    mp4_buffer_stats,
                });
                info!("Started camera '{}' on path '{}'" , camera_id, camera_config.path);
            }
            Err(e) => {
                error!("Failed to create video stream for camera '{}': {}", camera_id, e);
            }
        }
    }
    
    if camera_streams.is_empty() {
        warn!("No cameras configured or all failed to initialize. Server will start without cameras.");
        warn!("You can add cameras dynamically through the admin interface at /admin");
    }

    // Restart active recordings if recording manager is available
    if let Some(ref recording_manager) = recording_manager {
        if camera_streams.is_empty() {
            info!("Skipping recording restart check - no cameras configured");
        } else {
            info!("Checking for active recordings to restart...");
            
            // Create a map of camera_id -> frame_sender for the restart method
            let mut camera_frame_senders: HashMap<String, Arc<broadcast::Sender<bytes::Bytes>>> = HashMap::new();
            
            for stream_info in camera_streams.values() {
                camera_frame_senders.insert(
                    stream_info.camera_id.clone(),
                    stream_info.frame_sender.clone()
                );
            }
            
            // Restart active recordings
            if let Err(e) = recording_manager.restart_active_recordings_at_startup(&camera_frame_senders, &all_camera_configs).await {
                error!("Failed to restart active recordings at startup: {}", e);
            }
        }
    }

    let cors_layer = if let Some(origin) = &config.server.cors_allow_origin {
        if origin == "*" {
            tower_http::cors::CorsLayer::permissive()
        } else {
            match origin.parse::<axum::http::HeaderValue>() {
                Ok(origin_header) => {
                    tower_http::cors::CorsLayer::new()
                        .allow_origin(origin_header)
                        .allow_methods(tower_http::cors::Any)
                        .allow_headers(tower_http::cors::Any)
                }
                Err(_) => {
                    warn!("Invalid CORS origin '{}', falling back to permissive", origin);
                    tower_http::cors::CorsLayer::permissive()
                }
            }
        }
    } else {
        tower_http::cors::CorsLayer::permissive()
    };

    // Collect camera streams for API access
    
    // Convert camera_streams to be accessible by camera_id
    let mut camera_streams_by_id: HashMap<String, CameraStreamInfo> = HashMap::new();
    let camera_streams_by_path = camera_streams.clone();
    for (_, stream_info) in &camera_streams {
        camera_streams_by_id.insert(stream_info.camera_id.clone(), stream_info.clone());
    }
    
    let app_state = AppState {
        camera_streams: Arc::new(tokio::sync::RwLock::new(camera_streams_by_id)),
        camera_configs: Arc::new(tokio::sync::RwLock::new(all_camera_configs)),
        mqtt_handle: mqtt_handle.clone(),
        recording_manager: recording_manager.clone(),
        transcoding_config: Arc::new(config.transcoding.clone()),
        recording_config: config.recording.clone().map(Arc::new),
        admin_token: config.server.admin_token.clone(),
        cameras_directory: config.server.cameras_directory.clone().unwrap_or_else(|| "cameras".to_string()),
        start_time: std::time::Instant::now(),
        server_config: Arc::new(config.server.clone()),
    };

    // Build router with camera paths
    let mut app = axum::Router::new()
        //.nest_service("/static", tower_http::services::ServeDir::new("static"))
        .route("/dashboard", axum::routing::get(handlers::dashboard_handler))
        .route("/debug", axum::routing::get(handlers::debug_handler))
        .route("/hls.js", axum::routing::get(handlers::hlsjs_handler))
        .nest_service("/recordings", tower_http::services::ServeDir::new(app_state.recording_config.as_ref().map_or("recordings", |c| &c.database_path)));
    
    // Add routes for each camera (both stream and control endpoints)
    for (path, stream_info) in camera_streams_by_path {
        info!("Adding routes for camera at path: {}", path);
        
        // Stream endpoint: /<camera_path>/stream
        let stream_path = format!("{}/stream", path);
        let camera_id_for_stream = stream_info.camera_id.clone();
        let state_for_stream = app_state.clone();
        app = app.route(&stream_path, axum::routing::get(
            move |ws, query, addr| {
                let camera_id = camera_id_for_stream.clone();
                let state = state_for_stream.clone();
                async move {
                    handlers::dynamic_camera_stream_handler(ws, query, addr, camera_id, state).await
                }
            }
        ));

        // Control endpoint: /<camera_path>/control
        let control_path = format!("{}/control", path);
        let camera_id_for_control = stream_info.camera_id.clone();
        let state_for_control = app_state.clone();
        app = app.route(&control_path, axum::routing::get(
            move |headers, ws, query, addr| {
                let camera_id = camera_id_for_control.clone();
                let state = state_for_control.clone();
                async move {
                    handlers::dynamic_camera_control_handler(headers, ws, query, addr, camera_id, state).await
                }
            }
        ));

        // Live endpoint: /<camera_path>/live (WebSocket only)
        let live_path = format!("{}/live", path);
        let camera_id_for_live = stream_info.camera_id.clone();
        let state_for_live = app_state.clone();
        app = app.route(&live_path, axum::routing::get(
            move |ws, query, addr| {
                let camera_id = camera_id_for_live.clone();
                let state = state_for_live.clone();
                async move {
                    handlers::dynamic_camera_live_handler(ws, query, addr, camera_id, state).await
                }
            }
        ));

        // Camera page endpoint: /<camera_path> serves test.html
        app = app.route(&path, axum::routing::get(handlers::serve_test_page));
        
        // Test endpoint: /<camera_path>/test serves test.html
        let test_path = format!("{}/test", path);
        app = app.route(&test_path, axum::routing::get(handlers::serve_test_page));

        // REST API endpoints: /<camera_path>/control/*
        if stream_info.recording_manager.is_some() {
            let api_info = stream_info.clone();
            
            // Start recording
            let start_recording_path = format!("{}/control/recording/start", path);
            let start_info = api_info.clone();
            app = app.route(&start_recording_path, axum::routing::post(
                move |headers, json| api_recording::api_start_recording(
                    headers,
                    json,
                    start_info.camera_id.clone(),
                    start_info.camera_config.clone(),
                    start_info.recording_manager.clone().unwrap(),
                    start_info.frame_sender.clone(),
                    start_info.pre_recording_buffer.clone()
                )
            ));

            // Stop recording
            let stop_recording_path = format!("{}/control/recording/stop", path);
            let stop_info = api_info.clone();
            app = app.route(&stop_recording_path, axum::routing::post(
                move |headers| api_recording::api_stop_recording(
                    headers,
                    stop_info.camera_id.clone(),
                    stop_info.camera_config.clone(),
                    stop_info.recording_manager.clone().unwrap()
                )
            ));

            // List recordings
            let list_recordings_path = format!("{}/control/recordings", path);
            let list_info = api_info.clone();
            app = app.route(&list_recordings_path, axum::routing::get(
                move |headers, query| api_recording::api_list_recordings(
                    headers,
                    query,
                    list_info.camera_id.clone(),
                    list_info.camera_config.clone(),
                    list_info.recording_manager.clone().unwrap()
                )
            ));

            // Get recorded frames
            let frames_path = format!("{}/control/recordings/:session_id/frames", path);
            let frames_info = api_info.clone();
            app = app.route(&frames_path, axum::routing::get(
                move |headers, path, query| api_recording::api_get_recorded_frames(
                    headers,
                    path,
                    query,
                    frames_info.camera_config.clone(),
                    frames_info.recording_manager.clone().unwrap()
                )
            ));

            // Get single frame by timestamp
            let frame_by_timestamp_path = format!("{}/control/recordings/frames/:timestamp", path);
            let frame_info = api_info.clone();
            app = app.route(&frame_by_timestamp_path, axum::routing::get(
                move |headers, path, query| api_recording::api_get_frame_by_timestamp(
                    headers,
                    path,
                    query,
                    frame_info.camera_id.clone(),
                    frame_info.camera_config.clone(),
                    frame_info.recording_manager.clone().unwrap()
                )
            ));

            // Get active recording
            let active_recording_path = format!("{}/control/recording/active", path);
            let active_info = api_info.clone();
            app = app.route(&active_recording_path, axum::routing::get(
                move |headers| api_recording::api_get_active_recording(
                    headers,
                    active_info.camera_id.clone(),
                    active_info.camera_config.clone(),
                    active_info.recording_manager.clone().unwrap()
                )
            ));

            // Get recording database size
            let size_recording_path = format!("{}/control/recording/size", path);
            let size_info = api_info.clone();
            app = app.route(&size_recording_path, axum::routing::get(
                move |headers| api_recording::api_get_recording_size(
                    headers,
                    size_info.camera_id.clone(),
                    size_info.camera_config.clone(),
                    size_info.recording_manager.clone().unwrap()
                )
            ));

            // Set session keep flag
            let keep_flag_path = format!("{}/control/recordings/:session_id/keep", path);
            let keep_info = api_info.clone();
            app = app.route(&keep_flag_path, axum::routing::put(
                move |headers, path, query| api_recording::api_set_session_keep_flag(
                    headers,
                    path,
                    query,
                    keep_info.camera_id.clone(),
                    keep_info.camera_config.clone(),
                    keep_info.recording_manager.clone().unwrap()
                )
            ));

            // List MP4 segments
            let segments_path = format!("{}/control/recordings/mp4/segments", path);
            let segments_info = api_info.clone();
            app = app.route(&segments_path, axum::routing::get(
                move |headers, query| api_recording::api_list_mp4_segments(
                    headers,
                    query,
                    segments_info.camera_id.clone(),
                    segments_info.camera_config.clone(),
                    segments_info.recording_manager.clone().unwrap()
                )
            ));

            // Stream individual MP4 segments
            let stream_mp4_path = format!("{}/control/recordings/mp4/segments/:filename", path);
            let stream_info = api_info.clone();
            app = app.route(&stream_mp4_path, axum::routing::get(
                move |headers, path| api_recording::api_stream_mp4_segment(
                    headers,
                    path,
                    stream_info.camera_id.clone(),
                    stream_info.camera_config.clone(),
                    stream_info.recording_manager.clone().unwrap()
                )
            ));

            // HLS timerange playlist
            let hls_timerange_path = format!("{}/control/recordings/hls/timerange", path);
            let hls_info = api_info.clone();
            app = app.route(&hls_timerange_path, axum::routing::get(
                move |headers, query| api_recording::api_serve_hls_timerange(
                    headers,
                    query,
                    hls_info.camera_id.clone(),
                    hls_info.camera_config.clone(),
                    hls_info.recording_manager.clone().unwrap()
                )
            ));

            // HLS segments
            let hls_segments_path = format!("{}/control/recordings/hls/segments/:playlist_id/:segment_name", path);
            let hls_segment_info = api_info.clone();
            app = app.route(&hls_segments_path, axum::routing::get(
                move |headers, path| api_recording::api_serve_hls_segment(
                    headers,
                    path,
                    hls_segment_info.camera_id.clone(),
                    hls_segment_info.camera_config.clone(),
                    hls_segment_info.recording_manager.clone().unwrap()
                )
            ));
        }

        // PTZ control endpoints (handlers will validate if enabled in camera config)
        let ptz_info = stream_info.clone();
        let ptz_move_path = format!("{}/control/ptz/move", path);
        app = app.route(&ptz_move_path, axum::routing::post(move |headers, json| {
            let cfg = ptz_info.camera_config.clone();
            async move { api_ptz::api_ptz_move(headers, json, cfg).await }
        }));

        let ptz_info2 = stream_info.clone();
        let ptz_stop_path = format!("{}/control/ptz/stop", path);
        app = app.route(&ptz_stop_path, axum::routing::post(move |headers| {
            let cfg = ptz_info2.camera_config.clone();
            async move { api_ptz::api_ptz_stop(headers, cfg).await }
        }));

        let ptz_info3 = stream_info.clone();
        let ptz_goto_preset_path = format!("{}/control/ptz/goto_preset", path);
        app = app.route(&ptz_goto_preset_path, axum::routing::post(move |headers, json| {
            let cfg = ptz_info3.camera_config.clone();
            async move { api_ptz::api_ptz_goto_preset(headers, json, cfg).await }
        }));

        let ptz_info4 = stream_info.clone();
        let ptz_set_preset_path = format!("{}/control/ptz/set_preset", path);
        app = app.route(&ptz_set_preset_path, axum::routing::post(move |headers, json| {
            let cfg = ptz_info4.camera_config.clone();
            async move { api_ptz::api_ptz_set_preset(headers, json, cfg).await }
        }));
    }
    
    // Add API endpoints with captured state
    let api_state = app_state.clone();
    app = app.route("/api/status", axum::routing::get(move || {
        let state = api_state.clone();
        async move {
            trace!("[API] /api/status endpoint called");
            let uptime_secs = state.start_time.elapsed().as_secs();
            let camera_streams = state.camera_streams.read().await;
            let total_cameras = camera_streams.len();
            drop(camera_streams); // Release lock before await
            
            // Calculate total clients by summing clients from all cameras
            // Note: clients_connected includes WebSocket clients + internal systems (recording + control)
            // Each camera typically shows +2 clients at startup (recording=1, control=1)
            let mut total_clients = 0;
            if let Some(mqtt_handle) = &state.mqtt_handle {
                let all_camera_statuses = mqtt_handle.get_all_camera_status().await;
                total_clients = all_camera_statuses.values()
                    .map(|status| status.clients_connected)
                    .sum();
            }
            
            let status = serde_json::json!({
                "uptime_secs": uptime_secs,
                "total_clients": total_clients,
                "total_cameras": total_cameras
            });
            
            trace!("[API] /api/status returning response with uptime={}, clients={}, cameras={}", 
                  uptime_secs, total_clients, total_cameras);
            Json(ApiResponse::success(status)).into_response()
        }
    }));
    
    let api_state2 = app_state.clone();
    app = app.route("/api/cameras", axum::routing::get(move || {
        let state = api_state2.clone();
        async move {
            trace!("[API] /api/cameras endpoint called");
            
            // Get camera configurations first
            let camera_data = {
                let camera_configs = state.camera_configs.read().await;
                let mut data: Vec<(String, config::CameraConfig)> = camera_configs.iter()
                    .map(|(id, config)| (id.clone(), config.clone()))
                    .collect();
                data.sort_by(|a, b| a.0.cmp(&b.0));
                data
            };
            
            // Get active stream IDs, their receiver counts, FPS, pre-recording buffer stats, and MP4 buffer stats separately to avoid holding both locks
            let (active_stream_ids, stream_receiver_counts, stream_fps_values, pre_recording_buffer_frame_counts, pre_recording_buffer_size_kb, mp4_buffer_frame_counts, mp4_buffer_size_kb) = {
                let camera_streams = state.camera_streams.read().await;
                let ids = camera_streams.keys().cloned().collect::<std::collections::HashSet<String>>();
                let counts: std::collections::HashMap<String, usize> = camera_streams.iter()
                    .map(|(id, info)| (id.clone(), info.frame_sender.receiver_count()))
                    .collect();
                
                // Collect FPS values (we need to await them, but we can't do async in map)
                let mut fps_values = std::collections::HashMap::new();
                for (id, info) in camera_streams.iter() {
                    let fps = *info.capture_fps.read().await;
                    fps_values.insert(id.clone(), fps);
                }
                
                // Collect pre-recording buffer frame counts and sizes
                let mut buffer_frame_counts = std::collections::HashMap::new();
                let mut buffer_size_kb = std::collections::HashMap::new();
                for (id, info) in camera_streams.iter() {
                    if let Some(ref pre_recording_buffer) = info.pre_recording_buffer {
                        let stats = pre_recording_buffer.get_stats().await;
                        buffer_frame_counts.insert(id.clone(), stats.frame_count);
                        buffer_size_kb.insert(id.clone(), (stats.total_size_bytes as f64 / 1024.0).round() as u64);
                    }
                }
                
                // Collect MP4 buffer frame counts and sizes
                let mut mp4_buffer_frames = std::collections::HashMap::new();
                let mut mp4_buffer_kb = std::collections::HashMap::new();
                for (id, info) in camera_streams.iter() {
                    let mp4_stats = info.mp4_buffer_stats.read().await;
                    mp4_buffer_frames.insert(id.clone(), mp4_stats.frame_count);
                    mp4_buffer_kb.insert(id.clone(), mp4_stats.size_kb());
                }
                
                (ids, counts, fps_values, buffer_frame_counts, buffer_size_kb, mp4_buffer_frames, mp4_buffer_kb)
            };
            
            trace!("[API] Got {} total configs, {} active streams", 
                   camera_data.len(), active_stream_ids.len());
            
            let mut cameras = Vec::new();
            
            // Get all camera statuses at once for efficiency
            let all_camera_statuses = if let Some(mqtt_handle) = &state.mqtt_handle {
                mqtt_handle.get_all_camera_status().await
            } else {
                HashMap::new()
            };
            
            for (camera_id, camera_config) in camera_data {
                let is_enabled = camera_config.enabled.unwrap_or(true);
                let is_active = active_stream_ids.contains(&camera_id);
                let token_required = camera_config.token.is_some();
                
                let camera_status = if is_active && is_enabled {
                    // Camera is enabled and has an active stream
                    if let Some(real_status) = all_camera_statuses.get(&camera_id) {
                        // We have MQTT status data
                        serde_json::json!({
                            "id": real_status.id,
                            "path": camera_config.path,
                            "enabled": is_enabled,
                            "connected": real_status.connected,
                            "capture_fps": real_status.capture_fps,
                            "clients_connected": real_status.clients_connected,
                            "last_frame_time": real_status.last_frame_time,
                            "ffmpeg_running": real_status.ffmpeg_running,
                            "duplicate_frames": real_status.duplicate_frames,
                            "token_required": token_required,
                            "pre_recording_buffer_frames": pre_recording_buffer_frame_counts.get(&camera_id).copied().unwrap_or(0),
                            "pre_recording_buffer_size_kb": pre_recording_buffer_size_kb.get(&camera_id).copied().unwrap_or(0),
                            "mp4_buffered_frames": mp4_buffer_frame_counts.get(&camera_id).copied().unwrap_or(0),
                            "mp4_buffered_size_kb": mp4_buffer_size_kb.get(&camera_id).copied().unwrap_or(0)
                        })
                    } else {
                        // No MQTT status, but camera stream is active - get basic info
                        let clients_connected = stream_receiver_counts.get(&camera_id).copied().unwrap_or(0);
                        let capture_fps = stream_fps_values.get(&camera_id).copied().unwrap_or(0.0);
                        
                        // Camera is active (streaming) even without MQTT
                        serde_json::json!({
                            "id": camera_id,
                            "path": camera_config.path,
                            "enabled": is_enabled,
                            "connected": true,  // Stream is active, so it's connected
                            "capture_fps": capture_fps,  // Get actual FPS from stream
                            "clients_connected": clients_connected,
                            "last_frame_time": null,
                            "ffmpeg_running": true,  // If stream is active, FFmpeg must be running
                            "duplicate_frames": 0,
                            "token_required": token_required,
                            "pre_recording_buffer_frames": pre_recording_buffer_frame_counts.get(&camera_id).copied().unwrap_or(0),
                            "pre_recording_buffer_size_kb": pre_recording_buffer_size_kb.get(&camera_id).copied().unwrap_or(0),
                            "mp4_buffered_frames": mp4_buffer_frame_counts.get(&camera_id).copied().unwrap_or(0),
                            "mp4_buffered_size_kb": mp4_buffer_size_kb.get(&camera_id).copied().unwrap_or(0)
                        })
                    }
                } else {
                    // Camera is disabled or not active
                    serde_json::json!({
                        "id": camera_id,
                        "path": camera_config.path,
                        "enabled": is_enabled,
                        "connected": false,
                        "capture_fps": 0.0,
                        "clients_connected": 0,
                        "last_frame_time": null,
                        "ffmpeg_running": false,
                        "duplicate_frames": 0,
                        "token_required": token_required,
                        "pre_recording_buffer_frames": 0,
                        "pre_recording_buffer_size_kb": 0,
                        "mp4_buffered_frames": 0,
                        "mp4_buffered_size_kb": 0
                    })
                };
                
                cameras.push(camera_status);
            }
            
            let response = serde_json::json!({
                "cameras": cameras,
                "count": cameras.len()
            });
            
            trace!("[API] /api/cameras returning {} cameras", cameras.len());
            Json(ApiResponse::success(response)).into_response()
        }
    }));

    // Camera management API endpoints
    let admin_state = app_state.clone();
    app = app.route("/api/admin/cameras", axum::routing::post(move |headers: axum::http::HeaderMap, body: axum::extract::Json<api_config::CreateCameraRequest>| {
        let state = admin_state.clone();
        async move {
            api_config::api_create_camera(headers, body, state).await
        }
    }));

    let admin_state2 = app_state.clone();
    app = app.route("/api/admin/cameras/:id", axum::routing::get(move |headers: axum::http::HeaderMap, path: axum::extract::Path<String>| {
        let state = admin_state2.clone();
        async move {
            api_config::api_get_camera_config(headers, path, state).await
        }
    }));

    let admin_state3 = app_state.clone();
    app = app.route("/api/admin/cameras/:id", axum::routing::put(move |headers: axum::http::HeaderMap, path: axum::extract::Path<String>, body: axum::extract::Json<config::CameraConfig>| {
        let state = admin_state3.clone();
        async move {
            api_config::api_update_camera(headers, path, body, state).await
        }
    }));

    let admin_state4 = app_state.clone();
    app = app.route("/api/admin/cameras/:id", axum::routing::delete(move |headers: axum::http::HeaderMap, path: axum::extract::Path<String>| {
        let state = admin_state4.clone();
        async move {
            api_config::api_delete_camera(headers, path, state).await
        }
    }));

    // Server configuration management API endpoints
    let args_get = args.clone();
    let admin_config_state = app_state.clone();
    app = app.route("/api/admin/config", axum::routing::get(move |headers: axum::http::HeaderMap| {
        let args = args_get.clone();
        let state = admin_config_state.clone();
        async move {
            api_config::api_get_config(headers, args, state).await
        }
    }));

    let args_put = args.clone();
    let admin_update_state = app_state.clone();
    app = app.route("/api/admin/config", axum::routing::put(move |headers: axum::http::HeaderMap, body: axum::extract::Json<serde_json::Value>| {
        let args = args_put.clone();
        let state = admin_update_state.clone();
        async move {
            api_config::api_update_config(headers, body, args, state).await
        }
    }));
    
    // Add fallback handler for dynamic camera routes
    let fallback_state = app_state.clone();
    app = app.fallback(move |uri: axum::http::Uri, ws: Option<axum::extract::WebSocketUpgrade>, query: axum::extract::Query<std::collections::HashMap<String, String>>, addr: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>, headers: axum::http::HeaderMap| {
        let state = fallback_state.clone();
        async move {
            handlers::dynamic_camera_fallback_handler(uri, ws, query, addr, headers, state).await
        }
    });

    app = app.layer(cors_layer);

    // Start camera configuration file watcher
    if let Err(e) = watcher::start_camera_config_watcher(app_state.clone()).await {
        error!("Failed to start camera configuration watcher: {}", e);
    }

    let addr = format!("{}:{}", config.server.host, config.server.port);
    
    // Check if TLS is enabled
    // Convert the router to stateless by applying the state
    let stateless_app = app.with_state(app_state);
    
    if let Some(tls_config) = &config.server.tls {
        if tls_config.enabled {
            info!("Starting HTTPS server on {}", addr);
            start_https_server(stateless_app, &addr, tls_config).await?;
        } else {
            info!("Starting HTTP server on {}", addr);
            start_http_server(stateless_app, &addr).await?;
        }
    } else {
        info!("Starting HTTP server on {}", addr);
        start_http_server(stateless_app, &addr).await?;
    }

    Ok(())
}



// API Request/Response structs

async fn start_http_server(app: axum::Router, addr: &str) -> Result<()> {
    use socket2::{Domain, Protocol, Socket, Type};
    use std::net::SocketAddr;
    
    let addr: SocketAddr = addr.parse()?;
    
    // Create socket with custom settings for better connection handling
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    
    // Set socket options for better performance
    socket.set_reuse_address(true)?;
    socket.set_tcp_nodelay(true)?;
    socket.set_keepalive(true)?;
    
    // Set socket to non-blocking mode for Tokio compatibility
    socket.set_nonblocking(true)?;
    
    // Set a larger backlog for pending connections
    socket.bind(&addr.into())?;
    socket.listen(1024)?; // Increased from default (usually 128)
    
    let std_listener: std::net::TcpListener = socket.into();
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    info!("HTTP server listening on http://{} with enhanced socket configuration", addr);
    
    // Configure server with higher connection limits and better performance
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.expect("failed to listen for ctrl+c");
            info!("Shutting down HTTP server...");
        })
        .await?;
    Ok(())
}

async fn start_https_server(app: axum::Router, addr: &str, tls_cfg: &config::TlsConfig) -> Result<()> {
    // Load TLS certificates
    let cert_file = File::open(&tls_cfg.cert_path)
        .map_err(|e| StreamError::server(format!("Failed to open certificate file '{}': {}", tls_cfg.cert_path, e)))?;
    let key_file = File::open(&tls_cfg.key_path)
        .map_err(|e| StreamError::server(format!("Failed to open private key file '{}': {}", tls_cfg.key_path, e)))?;

    let mut cert_reader = BufReader::new(cert_file);
    let mut key_reader = BufReader::new(key_file);

    // Parse certificate and key
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .map_err(|e| StreamError::server(format!("Failed to parse certificate: {}", e)))?
        .into_iter()
        .map(rustls::Certificate)
        .collect();
    
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
        .map_err(|e| StreamError::server(format!("Failed to parse private key: {}", e)))?;
    
    if keys.is_empty() {
        // Try RSA private keys if PKCS8 fails
        let mut key_reader = BufReader::new(File::open(&tls_cfg.key_path)?);
        keys = rustls_pemfile::rsa_private_keys(&mut key_reader)
            .map_err(|e| StreamError::server(format!("Failed to parse RSA private key: {}", e)))?;
    }
    
    let private_key = keys.into_iter().next()
        .ok_or_else(|| StreamError::server("No private key found in key file"))?;

    // Create TLS configuration
    let rustls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, rustls::PrivateKey(private_key))
        .map_err(|e| StreamError::server(format!("Failed to create TLS config: {}", e)))?;

    info!("HTTPS server listening on https://{}", addr);
    info!("Certificate: {}", tls_cfg.cert_path);
    info!("Private key: {}", tls_cfg.key_path);

    // Start HTTPS server
    let tls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(rustls_config));
    axum_server::bind_rustls(addr.parse()?, tls_config)
        .serve(app.into_make_service())
        .await
        .map_err(|e| StreamError::server(format!("HTTPS server error: {}", e)))?;

    Ok(())
}

// admin API handlers moved to api::admin

// Camera management functions for dynamic reload



