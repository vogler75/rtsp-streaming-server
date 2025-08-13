use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};
use tracing::{info, error, debug, warn};
use anyhow::Result;
use bytes::Bytes;

use crate::config::RtspConfig;
use crate::transcoder::FrameTranscoder;

pub struct RtspClient {
    camera_id: String,
    config: RtspConfig,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    transcoder: FrameTranscoder,
    capture_framerate: u32,
    send_framerate: u32,
    quality: u8,
    latest_frame: Arc<RwLock<Option<Bytes>>>,
    allow_duplicate_frames: bool,
    debug_capture: bool,
    debug_sending: bool,
}

impl RtspClient {
    pub async fn new(camera_id: String, config: RtspConfig, frame_sender: Arc<broadcast::Sender<Bytes>>, quality: u8, capture_framerate: u32, send_framerate: u32, allow_duplicate_frames: bool, debug_capture: bool, debug_sending: bool) -> Self {
        Self {
            camera_id,
            config,
            frame_sender,
            transcoder: FrameTranscoder::new(quality).await,
            capture_framerate,
            send_framerate,
            quality,
            latest_frame: Arc::new(RwLock::new(None)),
            allow_duplicate_frames,
            debug_capture,
            debug_sending,
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
        
        tokio::spawn(async move {
            Self::frame_sender_task(camera_id, frame_sender, latest_frame, send_framerate, allow_duplicate_frames, debug_sending).await;
        });
        
        // Main capture loop
        loop {
            match self.connect_and_stream().await {
                Ok(_) => {
                    info!("[{}] RTSP stream ended normally", self.camera_id);
                }
                Err(e) => {
                    error!("[{}] RTSP connection error: {}", self.camera_id, e);
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
            if debug_sending && now.duration_since(last_log_time) >= Duration::from_secs(1) {
                debug!("[{}] SENDING: {:2}/s Pings : {:2}/s", 
                       camera_id, sent_count, skipped_count);
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
                debug!("[{}] CAPTURE: {:2}/s Target: {:2}/s (test)", 
                       self.camera_id, frame_count, self.capture_framerate);
                frame_count = 0;
                last_log_time = now;
            }
            
            // Generate frames at configured capture FPS
            let frame_duration_ms = 1000 / self.capture_framerate as u64;
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
            info!("Starting FFmpeg with capture framerate: {} FPS, quality: {}", self.capture_framerate, self.quality);
        } else {
            info!("Starting FFmpeg with natural camera framerate, quality: {}", self.quality);
        }
        
        // Create owned strings that will live long enough
        let quality_str = self.quality.to_string();
        let fps_str = if self.capture_framerate > 0 {
            Some(format!("fps={}", self.capture_framerate))
        } else {
            None
        };
        
        // Build FFmpeg arguments with optional buffer size
        let mut ffmpeg_args = vec![
            "-fflags", "+nobuffer+discardcorrupt",  // Disable buffering, discard corrupt frames
            "-flags", "low_delay",                  // Low delay mode
            "-avioflags", "direct",                 // Direct I/O to avoid buffering
        ];
        
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
            "-q:v", &quality_str,                   // JPEG quality from config
        ]);
        
        // Add fps filter only if capture_framerate > 0
        if let Some(ref fps_filter) = fps_str {
            ffmpeg_args.extend_from_slice(&[
                "-vf", fps_filter,                  // Force exact framerate
                "-vsync", "cfr",                    // Constant frame rate output
            ]);
            info!("FFmpeg: Using fps filter: {}", fps_filter);
        } else {
            info!("FFmpeg: No fps filter - using camera's natural frame rate");
        }
        
        ffmpeg_args.extend_from_slice(&[
            "-flush_packets", "1",                  // Flush packets immediately
            "-an",                                  // No audio
            "-"                                     // Output to stdout
        ]);
        
        let mut ffmpeg_cmd = tokio::process::Command::new("ffmpeg")
            .args(&ffmpeg_args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        info!("📡 FFmpeg process started, reading MJPEG stream from camera");
        
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
                            if self.debug_capture && now.duration_since(last_log_time) >= Duration::from_secs(1) {
                                if self.capture_framerate > 0 {
                                    debug!("[{}] CAPTURE: {:2}/s Target: {:2}/s", 
                                           self.camera_id, frame_count, self.capture_framerate);
                                } else {
                                    debug!("[{}] CAPTURE: {:2}/s Natural Rate", self.camera_id, frame_count);
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