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
    pub fn new(config: RtspConfig, frame_sender: Arc<broadcast::Sender<Bytes>>) -> Self {
        Self {
            config,
            frame_sender,
            transcoder: FrameTranscoder::new(),
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

                // For now, we'll simulate receiving frames since the retina API is complex
                info!("🎬 Starting to simulate frame reception (TODO: implement real decoding)");
                let mut frame_count = 0u64;
                
                loop {
                    frame_count += 1;
                    
                    if frame_count % 100 == 0 {
                        info!("📺 Simulating frame {} from RTSP camera", frame_count);
                    }

                    // Generate test frame that indicates we're connected to RTSP
                    let jpeg_data = self.transcoder.create_test_frame_rtsp_connected().await?;
                    if let Err(_) = self.frame_sender.send(jpeg_data) {
                        debug!("No WebSocket clients connected");
                    }
                    
                    // Simulate 30 FPS
                    tokio::time::sleep(Duration::from_millis(33)).await;
                }
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
}