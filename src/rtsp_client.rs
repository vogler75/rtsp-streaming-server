use std::sync::Arc;
use tokio::sync::broadcast;
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
}

impl RtspClient {
    pub async fn new(config: RtspConfig, frame_sender: Arc<broadcast::Sender<Bytes>>, quality: u8) -> Self {
        Self {
            config,
            frame_sender,
            transcoder: FrameTranscoder::new(quality).await,
        }
    }

    pub async fn start(&self) -> Result<()> {
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
            if let Err(_) = self.frame_sender.send(jpeg_data) {
                debug!("No active WebSocket connections to send frame to");
            }
            
            // Generate frames at ~30 FPS
            tokio::time::sleep(Duration::from_millis(33)).await;
        }
    }

    async fn stream_rtsp_via_ffmpeg(&self) -> Result<()> {
        info!("🎥 Starting direct RTSP to MJPEG streaming via FFmpeg");
        
        // Use FFmpeg to directly read from RTSP and output MJPEG frames
        let mut ffmpeg_cmd = tokio::process::Command::new("ffmpeg")
            .args([
                "-rtsp_transport", &self.config.transport,
                "-i", &self.config.url,
                "-f", "mjpeg",
                "-q:v", "5", // High quality JPEG
                "-vf", "fps=30", // Ensure consistent framerate
                "-an", // No audio
                "-"  // Output to stdout
            ])
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
                        info!("📷 Streamed {} real frames from camera via FFmpeg", frame_count);
                    }
                    
                    if let Err(_) = self.frame_sender.send(Bytes::from(jpeg_data)) {
                        debug!("No WebSocket clients connected");
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
        
        // Look for JPEG start marker (0xFF, 0xD8)
        let mut byte = [0u8; 1];
        loop {
            reader.read_exact(&mut byte).await?;
            if byte[0] == 0xFF {
                reader.read_exact(&mut byte).await?;
                if byte[0] == 0xD8 {
                    // Found JPEG start
                    buffer.push(0xFF);
                    buffer.push(0xD8);
                    break;
                }
            }
        }
        
        // Read until JPEG end marker (0xFF, 0xD9)
        let mut prev_byte = 0u8;
        loop {
            reader.read_exact(&mut byte).await?;
            buffer.push(byte[0]);
            
            if prev_byte == 0xFF && byte[0] == 0xD9 {
                // Found JPEG end
                break;
            }
            prev_byte = byte[0];
        }
        
        Ok(buffer.clone())
    }
}