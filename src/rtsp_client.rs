use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};
use tracing::{info, error, debug, warn};
use anyhow::Result;
use bytes::Bytes;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::config::RtspConfig;
use crate::transcoder::FrameTranscoder;
use crate::mqtt::{MqttHandle, CameraStatus};
use chrono::Utc;

pub struct RtspClient {
    camera_id: String,
    config: RtspConfig,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    transcoder: FrameTranscoder,
    capture_framerate: u32,
    send_framerate: u32,
    quality: Option<u8>,
    latest_frame: Arc<RwLock<Option<Bytes>>>,
    allow_duplicate_frames: bool,
    debug_capture: bool,
    debug_sending: bool,
    mqtt_handle: Option<MqttHandle>,
    capture_fps: Arc<RwLock<f32>>,
    send_fps: Arc<RwLock<f32>>,
}

impl RtspClient {
    pub async fn new(camera_id: String, config: RtspConfig, frame_sender: Arc<broadcast::Sender<Bytes>>, quality: Option<u8>, capture_framerate: u32, send_framerate: u32, allow_duplicate_frames: bool, debug_capture: bool, debug_sending: bool, mqtt_handle: Option<MqttHandle>) -> Self {
        Self {
            camera_id,
            config,
            frame_sender,
            transcoder: FrameTranscoder::new(quality.unwrap_or(75)).await,
            capture_framerate,
            send_framerate,
            quality,
            latest_frame: Arc::new(RwLock::new(None)),
            allow_duplicate_frames,
            debug_capture,
            debug_sending,
            mqtt_handle,
            capture_fps: Arc::new(RwLock::new(0.0)),
            send_fps: Arc::new(RwLock::new(0.0)),
        }
    }

    pub async fn start(&self) -> Result<()> {
        // Spawn the frame sender thread
        let frame_sender = self.frame_sender.clone();
        let latest_frame = self.latest_frame.clone();
        let send_framerate = self.send_framerate;
        let allow_duplicate_frames = self.allow_duplicate_frames;
        let debug_sending = self.debug_sending;
        let camera_id = self.camera_id.clone();
        let send_fps = self.send_fps.clone();
        let mqtt_handle = self.mqtt_handle.clone();
        
        tokio::spawn(async move {
            Self::frame_sender_task(camera_id, frame_sender, latest_frame, send_framerate, allow_duplicate_frames, debug_sending, send_fps, mqtt_handle).await;
        });
        
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
                            send_fps: 0.0,
                            clients_connected: self.frame_sender.receiver_count(),
                            last_frame_time: None,
                            ffmpeg_running: false,
                        };
                        mqtt.update_camera_status(self.camera_id.clone(), status).await;
                    }
                    
                    info!("[{}] Reconnecting in {} seconds...", self.camera_id, self.config.reconnect_interval);
                    sleep(Duration::from_secs(self.config.reconnect_interval)).await;
                }
            }
        }
    }
    
    async fn frame_sender_task(
        camera_id: String,
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        latest_frame: Arc<RwLock<Option<Bytes>>>,
        send_framerate: u32,
        allow_duplicate_frames: bool,
        debug_sending: bool,
        send_fps: Arc<RwLock<f32>>,
        _mqtt_handle: Option<MqttHandle>,
    ) {
        info!("[{}] Starting frame sender task at {} FPS (duplicates: {})", camera_id, send_framerate, allow_duplicate_frames);
        let frame_duration = Duration::from_millis(1000 / send_framerate as u64);
        let mut last_send_time = tokio::time::Instant::now();
        let mut sent_count = 0u64;
        let mut skipped_count = 0u64;
        let mut last_log_time = tokio::time::Instant::now();
        
        loop {
            // Wait for the next frame time
            let next_frame_time = last_send_time + frame_duration;
            tokio::time::sleep_until(next_frame_time).await;
            last_send_time = next_frame_time;
            
            // Get the latest frame (and optionally clear it)
            let frame = if allow_duplicate_frames {
                // Just read the frame, allow sending duplicates
                let guard = latest_frame.read().await;
                guard.clone()
            } else {
                // Take the frame and clear it (no duplicates)
                let mut guard = latest_frame.write().await;
                guard.take()  // This removes and returns the frame
            };
            
            // Send the frame if we have one, otherwise send empty ping
            if let Some(frame_data) = frame {
                // Send real frame data
                let _ = frame_sender.send(frame_data);
                sent_count += 1;
            } else {
                // No new frame available (when allow_duplicate_frames = false)
                // Send empty frame as ping to maintain connection and timing
                let _ = frame_sender.send(Bytes::new()); // Empty frame
                skipped_count += 1;
            }
            
            // Log statistics every second if enabled
            let now = tokio::time::Instant::now();
            if now.duration_since(last_log_time) >= Duration::from_secs(1) {
                let fps = sent_count as f32;
                *send_fps.write().await = fps;
                
                if debug_sending {
                    debug!("[{}] SENDING: {:2}/s Pings : {:2}/s", 
                           camera_id, sent_count, skipped_count);
                }
                sent_count = 0;
                skipped_count = 0;
                last_log_time = now;
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
        info!("[{}] Attempting to connect to RTSP camera: {}", self.camera_id, self.config.url);
        debug!("Parsing RTSP URL");
        
        let original_url = url::Url::parse(&self.config.url).map_err(|e| {
            error!("[{}] Invalid RTSP URL format: {}", self.camera_id, e);
            e
        })?;
        
        // Extract credentials and create URL without them for retina
        let creds = if !original_url.username().is_empty() {
            let username = original_url.username().to_string();
            let password = original_url.password().unwrap_or("").to_string();
            info!("[{}] Using credentials - username: {}, password: [{}] chars", self.camera_id, username, password.len());
            Some(retina::client::Credentials { username, password })
        } else {
            info!("[{}] No authentication credentials found in URL", self.camera_id);
            None
        };

        // Create URL without credentials for retina
        let mut url = original_url.clone();
        url.set_username("").unwrap();
        url.set_password(None).unwrap();
        info!("[{}] Cleaned URL for retina: {}", self.camera_id, url);

        info!("[{}] Creating RTSP session...", self.camera_id);
        let session_group = Arc::new(retina::client::SessionGroup::default());
        
        // Try to connect to RTSP stream
        info!("[{}] Connecting to RTSP server at {}:{}", self.camera_id, 
            url.host_str().unwrap_or("unknown"), 
            url.port().unwrap_or(554));
            
        match tokio::time::timeout(
            Duration::from_secs(10),
            retina::client::Session::describe(
                url.clone(),
                retina::client::SessionOptions::default()
                    .creds(creds)
                    .session_group(session_group)
                    .user_agent("RTSP-Streaming-Server/1.0".to_string()),
            )
        ).await {
            Ok(Ok(session)) => {
                info!("[{}] ✅ Successfully connected to RTSP server!", self.camera_id);
                info!("[{}] Available streams: {}", self.camera_id, session.streams().len());
                
                for (i, stream) in session.streams().iter().enumerate() {
                    info!("[{}] Stream {}: media={}, codec={:?}", self.camera_id, 
                        i, stream.media(), stream.encoding_name());
                }

                let video_stream_i = session.streams()
                    .iter()
                    .position(|s| s.media() == "video")
                    .ok_or_else(|| anyhow::anyhow!("❌ No video stream found in RTSP response"))?;

                info!("[{}] ✅ Found video stream at index {} with codec {:?}", self.camera_id, 
                    video_stream_i, session.streams()[video_stream_i].encoding_name());

                info!("[{}] 🎬 Starting real H.264 packet reception from camera", self.camera_id);
                
                // Now we need to properly setup and receive packets from the session
                // The retina library requires proper session setup for packet reception
                
                // For now, let's implement a more realistic approach:
                // We'll use direct RTSP streaming via FFmpeg instead of individual packet processing
                info!("🔄 Switching to direct RTSP to MJPEG transcoding via FFmpeg");
                
                return self.stream_rtsp_via_ffmpeg().await;
            }
            Ok(Err(e)) => {
                error!("[{}] ❌ Failed to connect to RTSP server: {}", self.camera_id, e);
                return Err(e.into());
            }
            Err(_) => {
                error!("❌ Timeout connecting to RTSP server (10 seconds)");
                return Err(anyhow::anyhow!("Connection timeout"));
            }
        }
    }

    async fn generate_test_frames(&self) -> Result<()> {
        info!("Starting test frame generation");
        let mut frame_count = 0u64;
        let mut last_log_time = tokio::time::Instant::now();
        
        loop {
            frame_count += 1;

            let jpeg_data = self.transcoder.create_test_frame().await?;
            
            // Store the frame in the latest frame store
            {
                let mut guard = self.latest_frame.write().await;
                *guard = Some(jpeg_data);
            }
            
            // Log test frame generation every second if enabled
            let now = tokio::time::Instant::now();
            if self.debug_capture && now.duration_since(last_log_time) >= Duration::from_secs(1) {
                let effective_framerate = if self.capture_framerate == 0 { 30 } else { self.capture_framerate };
                debug!("[{}] CAPTURE: {:2}/s Target: {:2}/s (test)", 
                       self.camera_id, frame_count, effective_framerate);
                frame_count = 0;
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
                        return Err(anyhow::anyhow!("FFmpeg process repeatedly failed"));
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
        if self.capture_framerate > 0 {
            if let Some(q) = self.quality {
                info!("Starting FFmpeg with capture framerate: {} FPS, quality: {}", self.capture_framerate, q);
            } else {
                info!("Starting FFmpeg with capture framerate: {} FPS, default quality", self.capture_framerate);
            }
        } else {
            if let Some(q) = self.quality {
                info!("Starting FFmpeg with natural camera framerate, quality: {}", q);
            } else {
                info!("Starting FFmpeg with natural camera framerate, default quality");
            }
        }
        
        // Create owned strings that will live long enough
        let quality_str = self.quality.map(|q| q.to_string());
        let fps_str = if self.capture_framerate > 0 {
            Some(format!("fps={}", self.capture_framerate))
        } else {
            None
        };
        
        // Build FFmpeg arguments with configurable options
        let mut ffmpeg_args = Vec::new();
        
        // Use custom FFmpeg options or defaults
        if let Some(ref opts) = self.config.ffmpeg_options {
            // Add fflags if specified and not empty
            if let Some(ref fflags) = opts.fflags {
                if !fflags.is_empty() {
                    ffmpeg_args.extend_from_slice(&["-fflags", fflags]);
                }
            } else {
                ffmpeg_args.extend_from_slice(&["-fflags", "+nobuffer+discardcorrupt"]);
            }
            
            // Add flags if specified and not empty
            if let Some(ref flags) = opts.flags {
                if !flags.is_empty() {
                    ffmpeg_args.extend_from_slice(&["-flags", flags]);
                }
            } else {
                ffmpeg_args.extend_from_slice(&["-flags", "low_delay"]);
            }
            
            // Add avioflags if specified and not empty
            if let Some(ref avioflags) = opts.avioflags {
                if !avioflags.is_empty() {
                    ffmpeg_args.extend_from_slice(&["-avioflags", avioflags]);
                }
            } else {
                ffmpeg_args.extend_from_slice(&["-avioflags", "direct"]);
            }
            
            // Add extra input arguments if specified
            if let Some(ref extra_input) = opts.extra_input_args {
                for arg in extra_input {
                    ffmpeg_args.push(arg);
                }
            }
        } else {
            // Use defaults if no custom options
            ffmpeg_args.extend_from_slice(&[
                "-fflags", "+nobuffer+discardcorrupt",
                "-flags", "low_delay",
                "-avioflags", "direct",
            ]);
        }
        
        // Add buffer size if configured (in KB)
        let buffer_size_str;
        if let Some(buffer_size) = self.config.ffmpeg_buffer_size {
            buffer_size_str = format!("{}k", buffer_size / 1024);
            ffmpeg_args.extend_from_slice(&["-rtbufsize", &buffer_size_str]);
            info!("FFmpeg buffer size set to: {}", buffer_size_str);
        }
        
        // Add basic arguments
        ffmpeg_args.extend_from_slice(&[
            "-rtsp_transport", &self.config.transport,
            "-i", &self.config.url,
            "-f", "mjpeg",
        ]);
        
        // Add quality parameter only if specified
        if let Some(ref quality_val) = quality_str {
            ffmpeg_args.extend_from_slice(&["-q:v", quality_val]);
        }
        
        // Add fps filter only if capture_framerate > 0
        if let Some(ref fps_filter) = fps_str {
            ffmpeg_args.extend_from_slice(&["-vf", fps_filter]);
            
            // Use custom fps_mode or default to cfr when using fps filter
            if let Some(ref opts) = self.config.ffmpeg_options {
                if let Some(ref fps_mode) = opts.fps_mode {
                    if !fps_mode.is_empty() {
                        ffmpeg_args.extend_from_slice(&["-fps_mode", fps_mode]);
                    }
                } else {
                    ffmpeg_args.extend_from_slice(&["-fps_mode", "cfr"]);
                }
            } else {
                ffmpeg_args.extend_from_slice(&["-fps_mode", "cfr"]);
            }
            info!("FFmpeg: Using fps filter: {}", fps_filter);
        } else {
            // Add fps_mode for natural framerate if specified and not empty
            if let Some(ref opts) = self.config.ffmpeg_options {
                if let Some(ref fps_mode) = opts.fps_mode {
                    if !fps_mode.is_empty() {
                        ffmpeg_args.extend_from_slice(&["-fps_mode", fps_mode]);
                    }
                }
            }
            info!("FFmpeg: No fps filter - using camera's natural frame rate");
        }
        
        // Add flush_packets option
        if let Some(ref opts) = self.config.ffmpeg_options {
            if let Some(ref flush) = opts.flush_packets {
                if !flush.is_empty() {
                    ffmpeg_args.extend_from_slice(&["-flush_packets", flush]);
                }
            } else {
                ffmpeg_args.extend_from_slice(&["-flush_packets", "1"]);
            }
        } else {
            ffmpeg_args.extend_from_slice(&["-flush_packets", "1"]);
        }
        
        ffmpeg_args.extend_from_slice(&[
            "-an",                                  // No audio
        ]);
        
        // Add extra output arguments if specified
        if let Some(ref opts) = self.config.ffmpeg_options {
            if let Some(ref extra_output) = opts.extra_output_args {
                for arg in extra_output {
                    ffmpeg_args.push(arg);
                }
            }
        }
        
        ffmpeg_args.push("-");  // Output to stdout
        
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
        if let Some(ref log_mode) = self.config.ffmpeg_log_stderr {
            if log_mode == "file" || log_mode == "console" || log_mode == "both" {
                let stderr = ffmpeg_cmd.stderr.take()
                    .ok_or_else(|| anyhow::anyhow!("Failed to get FFmpeg stderr"))?;
                
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
            .ok_or_else(|| anyhow::anyhow!("Failed to get FFmpeg stdout"))?;
            
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
                    return Err(anyhow::anyhow!("FFmpeg process died"));
                }
                
                // Read frames from stdout
                frame_result = self.read_mjpeg_frame(&mut reader, &mut buffer) => {
                    match frame_result {
                        Ok(jpeg_data) => {
                            frame_count += 1;
                            
                            // Store the frame in the latest frame store
                            {
                                let mut guard = self.latest_frame.write().await;
                                *guard = Some(Bytes::from(jpeg_data));
                            }
                            
                            // Log capture statistics every second if enabled
                            let now = tokio::time::Instant::now();
                            if now.duration_since(last_log_time) >= Duration::from_secs(1) {
                                let fps = frame_count as f32;
                                *self.capture_fps.write().await = fps;
                                
                                // Update MQTT status
                                if let Some(ref mqtt) = self.mqtt_handle {
                                    let status = CameraStatus {
                                        id: self.camera_id.clone(),
                                        connected: true,
                                        capture_fps: fps,
                                        send_fps: *self.send_fps.read().await,
                                        clients_connected: self.frame_sender.receiver_count(),
                                        last_frame_time: Some(Utc::now().to_rfc3339()),
                                        ffmpeg_running: true,
                                    };
                                    mqtt.update_camera_status(self.camera_id.clone(), status).await;
                                }
                                
                                if self.debug_capture {
                                    if self.capture_framerate > 0 {
                                        debug!("[{}] CAPTURE: {:2}/s Target: {:2}/s", 
                                               self.camera_id, frame_count, self.capture_framerate);
                                    } else {
                                        debug!("[{}] CAPTURE: {:2}/s Natural Rate", self.camera_id, frame_count);
                                    }
                                }
                                frame_count = 0;
                                last_log_time = now;
                            }
                        }
                        Err(e) => {
                            error!("[{}] Error reading MJPEG frame: {}", self.camera_id, e);
                            return Err(e);
                        }
                    }
                }
            }
        }
    }

    async fn read_mjpeg_frame(&self, reader: &mut tokio::io::BufReader<tokio::process::ChildStdout>, buffer: &mut Vec<u8>) -> Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        
        buffer.clear();
        buffer.reserve(100_000); // Pre-allocate for typical JPEG size
        
        // Use configured chunk size or default to 8KB for efficiency
        let chunk_size = self.config.chunk_read_size.unwrap_or(8192);
        let mut chunk = vec![0u8; chunk_size];
        let mut found_start = false;
        
        // Look for JPEG start marker (0xFF, 0xD8)
        while !found_start {
            let n = reader.read(&mut chunk).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("EOF while looking for JPEG start"));
            }
            
            for i in 0..n-1 {
                if chunk[i] == 0xFF && chunk[i+1] == 0xD8 {
                    // Found JPEG start, add everything from this point
                    buffer.extend_from_slice(&chunk[i..n]);
                    found_start = true;
                    break;
                }
            }
        }
        
        // Read until JPEG end marker (0xFF, 0xD9)
        loop {
            let n = reader.read(&mut chunk).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("EOF while looking for JPEG end"));
            }
            
            // Check if we have the end marker
            for i in 0..n-1 {
                if chunk[i] == 0xFF && chunk[i+1] == 0xD9 {
                    // Found JPEG end, add everything up to and including the marker
                    buffer.extend_from_slice(&chunk[0..=i+1]);
                    return Ok(std::mem::take(buffer)); // Move buffer content, not clone
                }
            }
            
            // Check boundary case: 0xFF at end of previous chunk, 0xD9 at start of this chunk
            if buffer.len() > 0 && buffer[buffer.len()-1] == 0xFF && chunk[0] == 0xD9 {
                buffer.push(0xD9);
                return Ok(std::mem::take(buffer)); // Move buffer content, not clone
            }
            
            // Add entire chunk to buffer
            buffer.extend_from_slice(&chunk[0..n]);
        }
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