use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};
use tracing::{info, error, debug};
use anyhow::Result;
use bytes::Bytes;

use crate::config::RtspConfig;
use crate::transcoder::FrameTranscoder;

pub struct RtspClient {
    config: RtspConfig,
    frame_sender: Arc<broadcast::Sender<Bytes>>,
    transcoder: FrameTranscoder,
    capture_framerate: u32,
    send_framerate: u32,
    quality: u8,
    latest_frame: Arc<RwLock<Option<Bytes>>>,
    allow_duplicate_frames: bool,
}

impl RtspClient {
    pub async fn new(config: RtspConfig, frame_sender: Arc<broadcast::Sender<Bytes>>, quality: u8, capture_framerate: u32, send_framerate: u32, allow_duplicate_frames: bool) -> Self {
        Self {
            config,
            frame_sender,
            transcoder: FrameTranscoder::new(quality).await,
            capture_framerate,
            send_framerate,
            quality,
            latest_frame: Arc::new(RwLock::new(None)),
            allow_duplicate_frames,
        }
    }

    pub async fn start(&self) -> Result<()> {
        // Spawn the frame sender thread
        let frame_sender = self.frame_sender.clone();
        let latest_frame = self.latest_frame.clone();
        let send_framerate = self.send_framerate;
        let allow_duplicate_frames = self.allow_duplicate_frames;
        
        tokio::spawn(async move {
            Self::frame_sender_task(frame_sender, latest_frame, send_framerate, allow_duplicate_frames).await;
        });
        
        // Main capture loop
        loop {
            match self.connect_and_stream().await {
                Ok(_) => {
                    info!("RTSP stream ended normally");
                }
                Err(e) => {
                    error!("RTSP connection error: {}", e);
                    info!("Reconnecting in {} seconds...", self.config.reconnect_interval);
                    sleep(Duration::from_secs(self.config.reconnect_interval)).await;
                }
            }
        }
    }
    
    async fn frame_sender_task(
        frame_sender: Arc<broadcast::Sender<Bytes>>,
        latest_frame: Arc<RwLock<Option<Bytes>>>,
        send_framerate: u32,
        allow_duplicate_frames: bool,
    ) {
        info!("Starting frame sender task at {} FPS (duplicates: {})", send_framerate, allow_duplicate_frames);
        let frame_duration = Duration::from_millis(1000 / send_framerate as u64);
        let mut last_send_time = tokio::time::Instant::now();
        let mut sent_count = 0u64;
        let mut skipped_count = 0u64;
        
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
            
            if (sent_count + skipped_count) % 100 == 0 {
                debug!("Frame sender: sent {} frames, {} pings (no new frame) at {} FPS", 
                       sent_count, skipped_count, send_framerate);
            }
        }
    }

    async fn connect_and_stream(&self) -> Result<()> {
        info!("Connecting to RTSP stream: {}", self.config.url);
        
        // Try to connect to real RTSP stream first
        match self.connect_real_rtsp().await {
            Ok(_) => {
                info!("RTSP connection ended");
            }
            Err(e) => {
                error!("Failed to connect to RTSP stream: {}", e);
                info!("Falling back to test frame generation");
                self.generate_test_frames().await?;
            }
        }
        
        Ok(())
    }

    async fn connect_real_rtsp(&self) -> Result<()> {
        info!("Attempting to connect to RTSP camera: {}", self.config.url);
        debug!("Parsing RTSP URL");
        
        let original_url = url::Url::parse(&self.config.url).map_err(|e| {
            error!("Invalid RTSP URL format: {}", e);
            e
        })?;
        
        // Extract credentials and create URL without them for retina
        let creds = if !original_url.username().is_empty() {
            let username = original_url.username().to_string();
            let password = original_url.password().unwrap_or("").to_string();
            info!("Using credentials - username: {}, password: [{}] chars", username, password.len());
            Some(retina::client::Credentials { username, password })
        } else {
            info!("No authentication credentials found in URL");
            None
        };

        // Create URL without credentials for retina
        let mut url = original_url.clone();
        url.set_username("").unwrap();
        url.set_password(None).unwrap();
        info!("Cleaned URL for retina: {}", url);

        info!("Creating RTSP session...");
        let session_group = Arc::new(retina::client::SessionGroup::default());
        
        // Try to connect to RTSP stream
        info!("Connecting to RTSP server at {}:{}", 
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
                info!("✅ Successfully connected to RTSP server!");
                info!("Available streams: {}", session.streams().len());
                
                for (i, stream) in session.streams().iter().enumerate() {
                    info!("Stream {}: media={}, codec={:?}", 
                        i, stream.media(), stream.encoding_name());
                }

                let video_stream_i = session.streams()
                    .iter()
                    .position(|s| s.media() == "video")
                    .ok_or_else(|| anyhow::anyhow!("❌ No video stream found in RTSP response"))?;

                info!("✅ Found video stream at index {} with codec {:?}", 
                    video_stream_i, session.streams()[video_stream_i].encoding_name());

                info!("🎬 Starting real H.264 packet reception from camera");
                
                // Now we need to properly setup and receive packets from the session
                // The retina library requires proper session setup for packet reception
                
                // For now, let's implement a more realistic approach:
                // We'll use direct RTSP streaming via FFmpeg instead of individual packet processing
                info!("🔄 Switching to direct RTSP to MJPEG transcoding via FFmpeg");
                
                return self.stream_rtsp_via_ffmpeg().await;
            }
            Ok(Err(e)) => {
                error!("❌ Failed to connect to RTSP server: {}", e);
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
        
        loop {
            frame_count += 1;
            if frame_count % 100 == 0 {
                debug!("Generated {} test frames", frame_count);
            }

            let jpeg_data = self.transcoder.create_test_frame().await?;
            
            // Store the frame in the latest frame store
            {
                let mut guard = self.latest_frame.write().await;
                *guard = Some(jpeg_data);
            }
            
            // Generate frames at configured capture FPS
            let frame_duration_ms = 1000 / self.capture_framerate as u64;
            tokio::time::sleep(Duration::from_millis(frame_duration_ms)).await;
        }
    }

    async fn stream_rtsp_via_ffmpeg(&self) -> Result<()> {
        info!("🎥 Starting direct RTSP to MJPEG streaming via FFmpeg");
        
        // Use FFmpeg to directly read from RTSP and output MJPEG frames with low latency
        info!("Starting FFmpeg with capture framerate: {} FPS, quality: {}", self.capture_framerate, self.quality);
        
        // Create owned strings that will live long enough
        let quality_str = self.quality.to_string();
        let fps_str = format!("fps={}", self.capture_framerate);
        
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
        
        ffmpeg_args.extend_from_slice(&[
            "-rtsp_transport", &self.config.transport,
            "-i", &self.config.url,
            "-f", "mjpeg",
            "-q:v", &quality_str,                   // JPEG quality from config
            "-vf", &fps_str,                        // Force exact framerate
            "-vsync", "cfr",                        // Constant frame rate output
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
        
        // Read MJPEG frames from FFmpeg stdout
        loop {
            match self.read_mjpeg_frame(&mut reader, &mut buffer).await {
                Ok(jpeg_data) => {
                    frame_count += 1;
                    
                    if frame_count % 100 == 0 {
                        info!("📷 Captured {} frames from camera at {} FPS", frame_count, self.capture_framerate);
                    }
                    
                    // Store the frame in the latest frame store
                    {
                        let mut guard = self.latest_frame.write().await;
                        *guard = Some(Bytes::from(jpeg_data));
                    }
                }
                Err(e) => {
                    error!("Error reading MJPEG frame: {}", e);
                    break;
                }
            }
        }
        
        // Wait for FFmpeg process to finish
        let _ = ffmpeg_cmd.wait().await;
        
        Err(anyhow::anyhow!("FFmpeg process ended"))
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