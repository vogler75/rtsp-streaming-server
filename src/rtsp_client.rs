use std::sync::Arc;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};
use tracing::{info, error, debug, warn};
use bytes::Bytes;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::config::{RtspConfig, FfmpegConfig, TranscodingConfig, CameraMqttConfig};
use crate::errors::{Result, StreamError};
use crate::transcoder::FrameTranscoder;
use crate::mqtt::{MqttHandle, CameraStatus};
use chrono::Utc;

pub struct RtspClient {
    camera_id: String,
    config: RtspConfig,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    transcoder: FrameTranscoder,
    capture_framerate: u32,
    ffmpeg_config: Option<FfmpegConfig>,
    transcoding_config: TranscodingConfig,
    debug_capture: bool,
    debug_duplicate_frames: bool,
    mqtt_handle: Option<MqttHandle>,
    camera_mqtt_config: Option<CameraMqttConfig>,
    capture_fps: Arc<RwLock<f32>>,
    last_picture_time: Arc<RwLock<Option<u128>>>, // Timestamp in milliseconds
    last_frame_hash: Arc<RwLock<Option<u64>>>, // Hash of last frame for deduplication
    duplicate_frame_count: Arc<RwLock<u64>>, // Count of duplicate frames since last status update
    last_mqtt_publish_time: Arc<RwLock<Option<u128>>>, // Last MQTT image publish timestamp
}

impl RtspClient {
    pub async fn new(camera_id: String, config: RtspConfig, frame_sender: Arc<broadcast::Sender<Bytes>>, ffmpeg_config: Option<FfmpegConfig>, transcoding_config: TranscodingConfig, capture_framerate: u32, debug_capture: bool, debug_duplicate_frames: bool, mqtt_handle: Option<MqttHandle>, camera_mqtt_config: Option<CameraMqttConfig>) -> Self {
        Self::new_from_builder(camera_id, config, frame_sender, ffmpeg_config, transcoding_config, capture_framerate, debug_capture, debug_duplicate_frames, mqtt_handle, camera_mqtt_config).await
    }

    pub async fn new_from_builder(camera_id: String, config: RtspConfig, frame_sender: Arc<broadcast::Sender<Bytes>>, ffmpeg_config: Option<FfmpegConfig>, transcoding_config: TranscodingConfig, capture_framerate: u32, debug_capture: bool, debug_duplicate_frames: bool, mqtt_handle: Option<MqttHandle>, camera_mqtt_config: Option<CameraMqttConfig>) -> Self {
        Self {
            camera_id,
            config,
            frame_sender,
            transcoder: FrameTranscoder::new(
                ffmpeg_config.as_ref()
                    .and_then(|c| c.quality)
                    .unwrap_or(75)
            ).await,
            capture_framerate,
            ffmpeg_config,
            transcoding_config,
            debug_capture,
            debug_duplicate_frames,
            mqtt_handle,
            camera_mqtt_config,
            capture_fps: Arc::new(RwLock::new(0.0)),
            last_picture_time: Arc::new(RwLock::new(None)),
            last_frame_hash: Arc::new(RwLock::new(None)),
            duplicate_frame_count: Arc::new(RwLock::new(0)),
            last_mqtt_publish_time: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self) -> Result<()> {
        // Main capture loop
        loop {
            match self.connect_and_stream().await {
                Ok(_) => {
                    info!("[{}] RTSP stream ended normally", self.camera_id);
                }
                Err(e) => {
                    error!("[{}] RTSP connection error: {}", self.camera_id, e);
                    
                    // Update MQTT status to disconnected
                    if let Some(ref mqtt) = self.mqtt_handle {
                        let status = CameraStatus {
                            id: self.camera_id.clone(),
                            connected: false,
                            capture_fps: 0.0,
                            clients_connected: self.frame_sender.receiver_count(), // Includes WebSocket clients + internal systems (recording, control)
                            last_frame_time: None,
                            ffmpeg_running: false,
                            duplicate_frames: 0, // No duplicates when disconnected
                        };
                        mqtt.update_camera_status(self.camera_id.clone(), status).await;
                    }
                    
                    info!("[{}] Reconnecting in {} seconds...", self.camera_id, self.config.reconnect_interval);
                    sleep(Duration::from_secs(self.config.reconnect_interval)).await;
                }
            }
        }
    }
    

    async fn connect_and_stream(&self) -> Result<()> {
        info!("[{}] Connecting to RTSP stream: {}", self.camera_id, self.config.url);
        
        // Try to connect to real RTSP stream first
        match self.connect_real_rtsp().await {
            Ok(_) => {
                info!("[{}] RTSP connection ended", self.camera_id);
            }
            Err(e) => {
                error!("[{}] Failed to connect to RTSP stream: {}", self.camera_id, e);
                info!("[{}] Falling back to test frame generation", self.camera_id);
                self.generate_test_frames().await?;
            }
        }
        
        Ok(())
    }

    async fn connect_real_rtsp(&self) -> Result<()> {
        info!("[{}] Connecting to stream: {}", self.camera_id, self.config.url);
        
        // Validate URL format
        let _url = url::Url::parse(&self.config.url).map_err(|e| {
            error!("[{}] Invalid URL format: {}", self.camera_id, e);
            StreamError::rtsp_connection(format!("Invalid URL: {}", e))
        })?;
        
        // Use FFmpeg directly for all stream types (RTSP, HTTP, HTTPS, etc.)
        info!("[{}] Starting stream capture via FFmpeg", self.camera_id);
        return self.stream_rtsp_via_ffmpeg().await;
    }

    async fn generate_test_frames(&self) -> Result<()> {
        info!("Starting test frame generation");
        let mut _frame_count = 0u64;
        let mut last_log_time = tokio::time::Instant::now();
        
        loop {
            _frame_count += 1;

            let jpeg_data = self.transcoder.create_test_frame().await?;
            
            // Send frame directly to broadcast
            let _ = self.frame_sender.send(jpeg_data.clone());
            
            // Track picture arrival time for MQTT publishing (non-blocking)
            if let Some(ref mqtt) = self.mqtt_handle {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                    
                let mut last_time_guard = self.last_picture_time.write().await;
                let time_diff = if let Some(last_time) = *last_time_guard {
                    now.saturating_sub(last_time)
                } else {
                    0 // First picture, no time difference
                };
                *last_time_guard = Some(now);
                drop(last_time_guard);
                
                // Spawn MQTT publishing in background to avoid blocking frame processing
                let mqtt_clone = mqtt.clone();
                let camera_id_clone = self.camera_id.clone();
                tokio::spawn(async move {
                    mqtt_clone.publish_picture_arrival(&camera_id_clone, now, time_diff, 0).await; // Test frames have no meaningful size
                });
                
                // Handle camera-specific MQTT image publishing for test frames
                if let Some(ref camera_mqtt) = &self.camera_mqtt_config {
                    let should_publish = if camera_mqtt.publish_interval == 0 {
                        // Publish every frame when interval is 0
                        true
                    } else {
                        // Check if enough time has passed since last publish
                        let mut last_publish_guard = self.last_mqtt_publish_time.write().await;
                        let should_publish = if let Some(last_publish) = *last_publish_guard {
                            let interval_ms = camera_mqtt.publish_interval * 1000;
                            now.saturating_sub(last_publish) >= interval_ms as u128
                        } else {
                            true // First publish
                        };
                        
                        if should_publish {
                            *last_publish_guard = Some(now);
                        }
                        drop(last_publish_guard);
                        should_publish
                    };
                    
                    if should_publish {
                        // Clone necessary data for async task
                        let mqtt_clone2 = mqtt.clone();
                        let camera_id_clone2 = self.camera_id.clone();
                        let jpeg_data_clone = jpeg_data.clone();
                        let topic_name = camera_mqtt.topic_name.clone();
                        
                        // Spawn MQTT image publishing in background
                        tokio::spawn(async move {
                            if let Err(e) = mqtt_clone2.publish_camera_image(
                                &camera_id_clone2,
                                &jpeg_data_clone,
                                topic_name.as_ref()
                            ).await {
                                error!("Failed to publish test camera image for {}: {}", camera_id_clone2, e);
                            }
                        });
                    }
                }
            }
            
            // Reset frame count every second for test frame generation
            let now = tokio::time::Instant::now();
            if now.duration_since(last_log_time) >= Duration::from_secs(1) {
                _frame_count = 0;
                last_log_time = now;
            }
            
            // Generate frames at configured capture FPS
            // Use default of 30 FPS if capture_framerate is 0 (indicating max available)
            let effective_framerate = if self.capture_framerate == 0 { 30 } else { self.capture_framerate };
            let frame_duration_ms = 1000 / effective_framerate as u64;
            tokio::time::sleep(Duration::from_millis(frame_duration_ms)).await;
        }
    }

    async fn stream_rtsp_via_ffmpeg(&self) -> Result<()> {
        info!("🎥 Starting direct RTSP to MJPEG streaming via FFmpeg");
        
        let mut retry_count = 0;
        let max_retries = 10;
        
        loop {
            match self.run_ffmpeg_process().await {
                Ok(_) => {
                    info!("FFmpeg process ended normally");
                    retry_count = 0; // Reset on successful run
                }
                Err(e) => {
                    retry_count += 1;
                    error!("FFmpeg process failed (attempt {}): {}", retry_count, e);
                    
                    if retry_count >= max_retries {
                        error!("FFmpeg failed {} times, giving up", max_retries);
                        return Err(StreamError::ffmpeg("FFmpeg process repeatedly failed"));
                    }
                    
                    // Exponential backoff: 1s, 2s, 4s, 8s, 16s, max 30s
                    let delay = std::cmp::min(1u64 << (retry_count - 1), 30);
                    warn!("[{}] Waiting {} seconds before retrying FFmpeg...", self.camera_id, delay);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                }
            }
        }
    }
    
    async fn run_ffmpeg_process(&self) -> Result<()> {
        // Use FFmpeg to directly read from RTSP and output MJPEG frames with low latency
        let ffmpeg = self.ffmpeg_config.as_ref();
        if self.capture_framerate > 0 {
            if let Some(q) = ffmpeg.and_then(|c| c.quality) {
                info!("Starting FFmpeg with capture framerate: {} FPS, quality: {}", self.capture_framerate, q);
            } else {
                info!("Starting FFmpeg with capture framerate: {} FPS, default quality", self.capture_framerate);
            }
        } else {
            if let Some(q) = ffmpeg.and_then(|c| c.quality) {
                info!("Starting FFmpeg with natural camera framerate, quality: {}", q);
            } else {
                info!("Starting FFmpeg with natural camera framerate, default quality");
            }
        }
        
        // Create owned strings that will live long enough
        let quality_str = ffmpeg.and_then(|c| c.quality).map(|q| q.to_string());
        // Use FFmpeg config output_framerate first, then fall back to transcoding config default
        // Treat 0 as "not set" (don't apply output framerate limiting)
        let output_fps_str = ffmpeg.and_then(|c| c.output_framerate)
            .or(self.transcoding_config.output_framerate)
            .filter(|&fps| fps > 0)  // Only use if > 0
            .map(|fps| fps.to_string());
        let fps_str = if self.capture_framerate > 0 {
            Some(format!("fps={}", self.capture_framerate))
        } else {
            None
        };
        
        // Build FFmpeg arguments with configurable options
        let mut ffmpeg_args: Vec<String> = Vec::new();
        
        // Check if command override is specified
        let use_command_override = ffmpeg
            .as_ref()
            .and_then(|config| config.command.as_ref())
            .is_some();
        
        if use_command_override {
            // Use custom command override
            if let Some(ref config) = ffmpeg {
                if let Some(ref command) = config.command {
                    info!("[{}] Using custom FFmpeg command override", self.camera_id);
                    
                    // Split the command string into arguments (simple space-based splitting)
                    // Note: For more complex quoting, users can use extra_input_args and extra_output_args
                    let args: Vec<&str> = command.split_whitespace().collect();
                    
                    // Replace placeholders in the command
                    for arg in args {
                        let replaced_arg = arg.replace("$url", &self.config.url);
                        ffmpeg_args.push(replaced_arg.to_string());
                    }
                }
            }
        } else {
            // Use granular configuration options
            if let Some(ref config) = ffmpeg {
            // Add use_wallclock_as_timestamps as the first option if enabled
            if config.use_wallclock_as_timestamps.unwrap_or(true) {
                ffmpeg_args.push("-use_wallclock_as_timestamps".to_string());
                ffmpeg_args.push("1".to_string());
            }
            
            // Add fflags if specified and not empty
            if let Some(ref fflags) = config.fflags {
                if !fflags.is_empty() {
                    ffmpeg_args.push("-fflags".to_string());
                    ffmpeg_args.push(fflags.clone());
                }
            }
            
            // Add flags if specified and not empty
            if let Some(ref flags) = config.flags {
                if !flags.is_empty() {
                    ffmpeg_args.push("-flags".to_string());
                    ffmpeg_args.push(flags.clone());
                }
            }
            
            // Add avioflags if specified and not empty
            if let Some(ref avioflags) = config.avioflags {
                if !avioflags.is_empty() {
                    ffmpeg_args.push("-avioflags".to_string());
                    ffmpeg_args.push(avioflags.clone());
                }
            }
            
            // Add extra input arguments if specified
            if let Some(ref extra_input) = config.extra_input_args {
                for arg in extra_input {
                    ffmpeg_args.push(arg.clone());
                }
            }
            }
            // No default values - only use what's explicitly configured
            
            // Check if URL is RTSP to add RTSP-specific options
            let is_rtsp_url = self.config.url.to_lowercase().starts_with("rtsp://");
            
            // Add RTSP buffer size if configured (in KB) and URL is RTSP
            if is_rtsp_url {
                if let Some(buffer_size) = ffmpeg.and_then(|c| c.rtbufsize) {
                    let buffer_size_str = format!("{}k", buffer_size / 1024);
                    ffmpeg_args.push("-rtbufsize".to_string());
                    ffmpeg_args.push(buffer_size_str.clone());
                    info!("FFmpeg RTSP buffer size set to: {}", buffer_size_str);
                }
                
                // Add RTSP transport option only for RTSP URLs
                ffmpeg_args.push("-rtsp_transport".to_string());
                ffmpeg_args.push(self.config.transport.clone());
            }
            
            // Add input URL
            ffmpeg_args.push("-i".to_string());
            ffmpeg_args.push(self.config.url.clone());
        
            // Add output format (default to mjpeg if not specified)
            let format = ffmpeg
                .and_then(|c| c.output_format.as_deref())
                .unwrap_or("mjpeg");
            ffmpeg_args.push("-f".to_string());
            ffmpeg_args.push(format.to_string());
        
            // Add video codec if specified
            if let Some(ref codec) = ffmpeg.and_then(|c| c.video_codec.as_ref()) {
                ffmpeg_args.push("-codec:v".to_string());
                ffmpeg_args.push(codec.to_string());
            }
        
            // Add video bitrate if specified
            if let Some(ref bitrate) = ffmpeg.and_then(|c| c.video_bitrate.as_ref()) {
                ffmpeg_args.push("-b:v".to_string());
                ffmpeg_args.push(bitrate.to_string());
            }
        
            // Add quality parameter only if specified (mainly for MJPEG)
            if let Some(ref quality_val) = quality_str {
                ffmpeg_args.push("-q:v".to_string());
                ffmpeg_args.push(quality_val.clone());
            }
        
            // Add output framerate if specified
            if let Some(ref fps) = output_fps_str {
                ffmpeg_args.push("-r".to_string());
                ffmpeg_args.push(fps.clone());
            }
        
        
            // Add movflags if specified (important for fMP4 streaming)
            if let Some(ref movflags) = ffmpeg.and_then(|c| c.movflags.as_ref()) {
                ffmpeg_args.push("-movflags".to_string());
                ffmpeg_args.push(movflags.to_string());
            }
        
        // Build video filter chain if needed
        let mut video_filters = Vec::new();
        
        // Add scale filter if specified
        if let Some(ref scale) = ffmpeg.and_then(|c| c.scale.as_ref()) {
            video_filters.push(format!("scale={}", scale));
        }
        
        // Add fps filter only if capture_framerate > 0
        if let Some(ref fps_filter) = fps_str {
            video_filters.push(fps_filter.clone());
        }
        
        // Apply video filters if any
        let filter_chain;
        if !video_filters.is_empty() {
            filter_chain = video_filters.join(",");
            ffmpeg_args.push("-vf".to_string());
            ffmpeg_args.push(filter_chain.clone());
            
            // Use custom fps_mode only if explicitly configured
            let fps_mode = ffmpeg.and_then(|c| c.fps_mode.as_ref());
            
            if let Some(ref mode) = fps_mode {
                if !mode.is_empty() {
                    ffmpeg_args.push("-fps_mode".to_string());
                    ffmpeg_args.push(mode.to_string());
                }
            }
            // No default fps_mode - let FFmpeg decide
            
            info!("FFmpeg: Using video filters: {}", filter_chain);
        } else {
            // Add fps_mode for natural framerate if specified and not empty
            let fps_mode = ffmpeg.and_then(|c| c.fps_mode.as_ref());
            
            if let Some(ref mode) = fps_mode {
                if !mode.is_empty() {
                    ffmpeg_args.push("-fps_mode".to_string());
                    ffmpeg_args.push(mode.to_string());
                }
            }
            info!("FFmpeg: No video filters - using camera's natural frame rate");
        }
        
        // Add flush_packets option only if explicitly configured
        let flush_packets = ffmpeg.and_then(|c| c.flush_packets.as_ref());
        
        if let Some(ref flush) = flush_packets {
            if !flush.is_empty() {
                ffmpeg_args.push("-flush_packets".to_string());
                ffmpeg_args.push(flush.to_string());
            }
        }
        // No default flush_packets - let FFmpeg decide
        
            ffmpeg_args.push("-an".to_string());
        
            // Add extra output arguments if specified
            let extra_output = ffmpeg.and_then(|c| c.extra_output_args.as_ref());
            
            if let Some(extra_output) = extra_output {
                for arg in extra_output {
                    ffmpeg_args.push(arg.clone());
                }
            }
        
            ffmpeg_args.push("-".to_string());  // Output to stdout
        }
        
        // On Windows, try to use ffmpeg.exe from current directory first, then from PATH
        let ffmpeg_path = if cfg!(windows) && std::path::Path::new("./ffmpeg.exe").exists() {
            "./ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        
        // Log the full FFmpeg command
        let full_command = format!("{} {}", ffmpeg_path, ffmpeg_args.join(" "));
        info!("[{}] FFmpeg command: {}", self.camera_id, full_command);
        
        let mut ffmpeg_cmd = tokio::process::Command::new(ffmpeg_path)
            .args(&ffmpeg_args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        info!("[{}] 📡 FFmpeg process started, reading MJPEG stream from camera", self.camera_id);
        
        // Handle stderr logging if enabled
        let log_mode = ffmpeg.and_then(|c| c.log_stderr.as_ref());
        if let Some(log_mode) = log_mode {
            if log_mode == "file" || log_mode == "console" || log_mode == "both" {
                let stderr = ffmpeg_cmd.stderr.take()
                    .ok_or_else(|| StreamError::ffmpeg("Failed to get FFmpeg stderr"))?;
                
                let log_filename = format!("{}.log", self.camera_id);
                let camera_id = self.camera_id.clone();
                let log_mode_clone = log_mode.clone();
                
                info!("[{}] FFmpeg stderr logging enabled (mode: {})", self.camera_id, log_mode);
                
                // Spawn a task to handle stderr logging
                tokio::spawn(async move {
                    if let Err(e) = log_ffmpeg_stderr(stderr, &log_filename, &camera_id, &log_mode_clone).await {
                        error!("[{}] Failed to log FFmpeg stderr: {}", camera_id, e);
                    }
                });
            }
        }
        
        let stdout = ffmpeg_cmd.stdout.take()
            .ok_or_else(|| StreamError::ffmpeg("Failed to get FFmpeg stdout"))?;
            
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut frame_count = 0u64;
        let mut buffer = Vec::new();
        let mut last_log_time = tokio::time::Instant::now();
        
        // Read MJPEG frames from FFmpeg stdout with process monitoring
        loop {
            tokio::select! {
                // Monitor FFmpeg process status
                exit_status = ffmpeg_cmd.wait() => {
                    match exit_status {
                        Ok(status) => {
                            if status.success() {
                                info!("FFmpeg process exited normally");
                            } else {
                                error!("FFmpeg process exited with error: {}", status);
                            }
                        }
                        Err(e) => {
                            error!("[{}] Failed to wait for FFmpeg process: {}", self.camera_id, e);
                        }
                    }
                    return Err(StreamError::ffmpeg("FFmpeg process died"));
                }
                
                // Read frame data from stdout (MJPEG or other format)
                frame_result = self.read_mjpeg_frame(&mut reader, &mut buffer) => {
                    match frame_result {
                        Ok(frame_data) => {
                            // Validate frame is not empty or too small (minimum JPEG is ~100 bytes)
                            if frame_data.len() == 0 {
                                warn!("[{}] Skipping invalid frame: too small ({} bytes)", self.camera_id, frame_data.len());
                                continue;
                            }
                            
                            // Get frame size before processing
                            let frame_size = frame_data.len();
                            
                            // Calculate hash of frame data for deduplication
                            let mut hasher = DefaultHasher::new();
                            frame_data.hash(&mut hasher);
                            let current_hash = hasher.finish();
                            
                            // Check for duplicate frames
                            let mut last_hash_guard = self.last_frame_hash.write().await;
                            let is_duplicate = if let Some(last_hash) = *last_hash_guard {
                                last_hash == current_hash
                            } else {
                                false // First frame
                            };
                            
                            if is_duplicate {
                                // Increment duplicate counter
                                let mut dup_count_guard = self.duplicate_frame_count.write().await;
                                *dup_count_guard += 1;
                                let dup_count = *dup_count_guard;
                                drop(dup_count_guard);
                                drop(last_hash_guard);
                                
                                // Optional warning for duplicate frames
                                if self.debug_duplicate_frames {
                                    warn!("[{}] Skipping duplicate frame (size: {} bytes, total duplicates: {})", 
                                          self.camera_id, frame_size, dup_count);
                                }
                                
                                // Skip processing duplicate frame
                                continue;
                            } else {
                                // Update last frame hash
                                *last_hash_guard = Some(current_hash);
                                drop(last_hash_guard);
                            }
                            
                            frame_count += 1;
                            
                            // Measure frame processing time for diagnostics
                            let frame_start_time = std::time::Instant::now();
                            
                            // Send frame directly to broadcast
                            let _ = self.frame_sender.send(Bytes::from(frame_data.clone()));
                            
                            // Track picture arrival time for MQTT publishing (non-blocking)
                            if let Some(ref mqtt) = self.mqtt_handle {
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis();
                                    
                                let mut last_time_guard = self.last_picture_time.write().await;
                                let time_diff = if let Some(last_time) = *last_time_guard {
                                    now.saturating_sub(last_time)
                                } else {
                                    0 // First picture, no time difference
                                };

                                // Spawn MQTT publishing in background to avoid blocking frame processing
                                let mqtt_clone = mqtt.clone();
                                let camera_id_clone = self.camera_id.clone();
                                let frame_size_for_mqtt = frame_size;
                                tokio::spawn(async move {
                                    mqtt_clone.publish_picture_arrival(&camera_id_clone, now, time_diff, frame_size_for_mqtt).await;
                                });                                

                                // Note: Rapid frames are now handled by duplicate detection above

                                *last_time_guard = Some(now);
                                drop(last_time_guard);
                                
                                // Handle camera-specific MQTT image publishing
                                if let Some(ref camera_mqtt) = &self.camera_mqtt_config {
                                    let should_publish = if camera_mqtt.publish_interval == 0 {
                                        // Publish every frame when interval is 0
                                        true
                                    } else {
                                        // Check if enough time has passed since last publish
                                        let mut last_publish_guard = self.last_mqtt_publish_time.write().await;
                                        let should_publish = if let Some(last_publish) = *last_publish_guard {
                                            let interval_ms = camera_mqtt.publish_interval * 1000;
                                            now.saturating_sub(last_publish) >= interval_ms as u128
                                        } else {
                                            true // First publish
                                        };
                                        
                                        if should_publish {
                                            *last_publish_guard = Some(now);
                                        }
                                        drop(last_publish_guard);
                                        should_publish
                                    };
                                    
                                    if should_publish {
                                        // Clone necessary data for async task
                                        let mqtt_clone = mqtt.clone();
                                        let camera_id_clone = self.camera_id.clone();
                                        let frame_data_clone = frame_data.clone();
                                        let topic_name = camera_mqtt.topic_name.clone();
                                        
                                        // Spawn MQTT image publishing in background
                                        tokio::spawn(async move {
                                            if let Err(e) = mqtt_clone.publish_camera_image(
                                                &camera_id_clone,
                                                &frame_data_clone,
                                                topic_name.as_ref()
                                            ).await {
                                                error!("Failed to publish camera image for {}: {}", camera_id_clone, e);
                                            }
                                        });
                                    }
                                }
                            }
                            
                            // Measure and log frame processing time if it's slow
                            let processing_duration = frame_start_time.elapsed();
                            if processing_duration.as_millis() > 10 {
                                warn!("[{}] Slow frame processing: {}ms", self.camera_id, processing_duration.as_millis());
                            }
                            
                            // Log capture statistics every second if enabled
                            let now = tokio::time::Instant::now();
                            if now.duration_since(last_log_time) >= Duration::from_secs(1) {
                                let fps = frame_count as f32;
                                *self.capture_fps.write().await = fps;
                                
                                // Update MQTT status
                                if let Some(ref mqtt) = self.mqtt_handle {
                                    // Get and reset duplicate count
                                    let mut dup_count_guard = self.duplicate_frame_count.write().await;
                                    let duplicate_count = *dup_count_guard;
                                    *dup_count_guard = 0; // Reset counter after reading
                                    drop(dup_count_guard);
                                    
                                    let status = CameraStatus {
                                        id: self.camera_id.clone(),
                                        connected: true,
                                        capture_fps: fps,
                                        clients_connected: self.frame_sender.receiver_count(), // Includes WebSocket clients + internal systems (recording, control)
                                        last_frame_time: Some(Utc::now().to_rfc3339()),
                                        ffmpeg_running: true,
                                        duplicate_frames: duplicate_count,
                                    };
                                    mqtt.update_camera_status(self.camera_id.clone(), status).await;
                                }
                                
                                if self.debug_capture {
                                    if self.capture_framerate > 0 {
                                        debug!("[{}] Capturing: {:2}/s Target: {:2}/s", 
                                               self.camera_id, frame_count, self.capture_framerate);
                                    } else {
                                        debug!("[{}] Capturing: {:2}/s", self.camera_id, frame_count);
                                    }
                                }
                                frame_count = 0;
                                last_log_time = now;
                            }
                        }
                        Err(e) => {
                            // Check if FFmpeg process is still alive before returning error
                            match ffmpeg_cmd.try_wait() {
                                Ok(Some(status)) => {
                                    // Process has exited
                                    error!("[{}] FFmpeg process died with status: {}", self.camera_id, status);
                                    return Err(StreamError::ffmpeg("FFmpeg process died"));
                                }
                                Ok(None) => {
                                    // Process is still running, but we got an error reading frame
                                    error!("[{}] Error reading frame data while FFmpeg is running: {}", self.camera_id, e);
                                    // Try to continue if it's just a corrupted frame
                                    if e.to_string().contains("EOF") {
                                        // EOF might mean FFmpeg is dying, return error
                                        return Err(e);
                                    }
                                    // For other errors, try to continue
                                    warn!("[{}] Attempting to continue after frame read error", self.camera_id);
                                    continue;
                                }
                                Err(check_err) => {
                                    error!("[{}] Failed to check FFmpeg process status: {}", self.camera_id, check_err);
                                    return Err(e);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn read_mjpeg_frame(&self, reader: &mut tokio::io::BufReader<tokio::process::ChildStdout>, buffer: &mut Vec<u8>) -> Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        
        // JPEG frames start with 0xFF 0xD8 and end with 0xFF 0xD9
        const JPEG_START: [u8; 2] = [0xFF, 0xD8];
        const JPEG_END: [u8; 2] = [0xFF, 0xD9];
        
        // Clear the buffer for a new frame
        buffer.clear();
        
        // Read until we find the start of a JPEG frame
        let mut byte = [0u8; 1];
        let mut prev_byte = 0u8;
        let mut bytes_skipped = 0u32;
        
        // Skip to the start of the next JPEG frame
        loop {
            if reader.read_exact(&mut byte).await.is_err() {
                return Err(StreamError::ffmpeg("EOF while searching for JPEG start"));
            }
            
            bytes_skipped += 1;
            
            // If we're skipping too many bytes, something is wrong
            if bytes_skipped > 100_000 {
                return Err(StreamError::ffmpeg("Skipped too many bytes looking for JPEG start - stream corrupted"));
            }
            
            if prev_byte == JPEG_START[0] && byte[0] == JPEG_START[1] {
                // Found start of JPEG, add the start marker to buffer
                buffer.extend_from_slice(&JPEG_START);
                break;
            }
            prev_byte = byte[0];
        }
        
        // Read until we find the end of the JPEG frame
        prev_byte = 0;
        loop {
            if reader.read_exact(&mut byte).await.is_err() {
                return Err(StreamError::ffmpeg("EOF while reading JPEG data"));
            }
            
            buffer.push(byte[0]);
            
            if prev_byte == JPEG_END[0] && byte[0] == JPEG_END[1] {
                // Found end of JPEG
                break;
            }
            prev_byte = byte[0];
            
            // Sanity check: if frame is too large, something is wrong
            if buffer.len() > 10 * 1024 * 1024 { // 10MB max
                return Err(StreamError::ffmpeg("JPEG frame too large, likely corrupted"));
            }
        }
        
        Ok(buffer.clone())
    }
}

async fn log_ffmpeg_stderr(
    stderr: tokio::process::ChildStderr,
    log_filename: &str,
    camera_id: &str,
    log_mode: &str,
) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    
    // Open or create the log file if needed
    let mut log_file = if log_mode == "file" || log_mode == "both" {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_filename)
            .await?;
        
        // Write a timestamp header
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let header = format!("\n=== FFmpeg stderr log for {} started at {} (mode: {}) ===\n", camera_id, timestamp, log_mode);
        file.write_all(header.as_bytes()).await?;
        file.flush().await?;
        Some(file)
    } else {
        None
    };
    
    // Read stderr line by line and write to log file
    let reader = BufReader::new(stderr);
    let mut lines = reader.lines();
    
    while let Some(line) = lines.next_line().await? {
        // Log to file if enabled
        if let Some(ref mut file) = log_file {
            let log_line = format!("{}\n", line);
            file.write_all(log_line.as_bytes()).await?;
            file.flush().await?;
        }
        
        // Log to console if enabled
        if log_mode == "console" || log_mode == "both" {
            info!("[{}] FFmpeg: {}", camera_id, line);
        }
        
        // Note: FFmpeg stderr is NOT published to MQTT to avoid packet size issues
        // Use file or console logging instead for FFmpeg diagnostics
    }
    
    // Write closing marker to file if enabled
    if let Some(ref mut file) = log_file {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let footer = format!("=== FFmpeg stderr log for {} ended at {} ===\n", camera_id, timestamp);
        file.write_all(footer.as_bytes()).await?;
        file.flush().await?;
    }
    
    Ok(())
}