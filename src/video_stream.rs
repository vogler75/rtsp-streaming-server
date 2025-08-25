use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, error};
use bytes::Bytes;

use crate::config::{CameraConfig, TranscodingConfig, RtspConfig};
use crate::errors::Result;
use crate::rtsp_client::RtspClient;
use crate::mqtt::MqttHandle;
use crate::pre_recording_buffer::PreRecordingBuffer;

pub struct VideoStream {
    pub camera_id: String,
    pub frame_sender: Arc<broadcast::Sender<Bytes>>,
    rtsp_client: RtspClient,
    pub pre_recording_buffer: Option<PreRecordingBuffer>,
}

impl VideoStream {
    pub async fn new(
        camera_id: String,
        camera_config: CameraConfig,
        default_transcoding: &TranscodingConfig,
        mqtt_handle: Option<MqttHandle>,
        global_recording_config: Option<&crate::config::RecordingConfig>,
        shutdown_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
        latest_frame: Arc<tokio::sync::RwLock<Option<bytes::Bytes>>>,
    ) -> Result<Self> {
        Self::new_from_builder(camera_id, camera_config, default_transcoding.clone(), mqtt_handle, global_recording_config, shutdown_flag, latest_frame).await
    }

    pub async fn new_from_builder(
        camera_id: String,
        camera_config: CameraConfig,
        default_transcoding: TranscodingConfig,
        mqtt_handle: Option<MqttHandle>,
        global_recording_config: Option<&crate::config::RecordingConfig>,
        shutdown_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
        latest_frame: Arc<tokio::sync::RwLock<Option<bytes::Bytes>>>,
    ) -> Result<Self> {
        // Use camera-specific transcoding config if available, otherwise use default
        let transcoding = camera_config.transcoding_override.as_ref().unwrap_or(&default_transcoding);
        
        let channel_buffer_size = transcoding.channel_buffer_size.unwrap_or(1024);
        info!("Creating video stream for camera '{}' on path '{}' with buffer size: {} frames", 
              camera_id, camera_config.path, channel_buffer_size);
        
        let (frame_tx, _) = broadcast::channel(channel_buffer_size);
        let frame_tx = Arc::new(frame_tx);
        
        // Create RtspConfig from camera config
        let rtsp_config = RtspConfig {
            url: camera_config.url.clone(),
            transport: camera_config.transport.clone(),
            reconnect_interval: camera_config.reconnect_interval,
            chunk_read_size: camera_config.chunk_read_size,
        };
        
        // Initialize pre-recording buffer if enabled (with proper fallback to global config)
        let effective_pre_recording_enabled = match camera_config.get_pre_recording_enabled() {
            Some(enabled) => {
                info!("Using camera-specific pre-recording setting for '{}': {}", camera_id, enabled);
                enabled
            }
            None => {
                let global_enabled = global_recording_config.map(|cfg| cfg.pre_recording_enabled).unwrap_or(false);
                info!("Using global pre-recording setting for '{}': {}", camera_id, global_enabled);
                global_enabled
            }
        };
        
        let pre_recording_buffer = if effective_pre_recording_enabled {
            let buffer_minutes = camera_config.get_pre_recording_buffer_minutes()
                .or_else(|| global_recording_config.map(|cfg| cfg.pre_recording_buffer_minutes))
                .unwrap_or(3);
            let cleanup_interval = camera_config.get_pre_recording_cleanup_interval_seconds()
                .or_else(|| global_recording_config.map(|cfg| cfg.pre_recording_cleanup_interval_seconds))
                .unwrap_or(1);
            info!("Enabling pre-recording buffer for camera '{}' with {} minutes duration and {} second cleanup interval", 
                  camera_id, buffer_minutes, cleanup_interval);
            Some(PreRecordingBuffer::new(buffer_minutes, cleanup_interval))
        } else {
            info!("Pre-recording buffer disabled for camera '{}'", camera_id);
            None
        };

        let rtsp_client = RtspClient::new(
            camera_id.clone(),
            rtsp_config,
            frame_tx.clone(),
            camera_config.ffmpeg.clone(),
            transcoding.clone(),
            transcoding.capture_framerate,
            transcoding.debug_capture.unwrap_or(false),
            transcoding.debug_duplicate_frames.unwrap_or(false),
            mqtt_handle,
            camera_config.mqtt.clone(),
            shutdown_flag,
            latest_frame,
        ).await;
        
        Ok(Self {
            camera_id,
            frame_sender: frame_tx,
            rtsp_client,
            pre_recording_buffer,
        })
    }
    
    pub fn get_fps_counter(&self) -> Arc<tokio::sync::RwLock<f32>> {
        self.rtsp_client.get_fps_counter()
    }
    
    pub async fn start(self) -> tokio::task::JoinHandle<()> {
        let camera_id = self.camera_id.clone();
        
        // Start pre-recording buffer tasks if enabled
        let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        
        if let Some(ref buffer) = self.pre_recording_buffer {
            info!("Starting pre-recording buffer tasks for camera '{}'", camera_id);
            
            // Start the cleanup task
            let cleanup_task = buffer.start_cleanup_task(camera_id.clone()).await;
            tasks.push(cleanup_task);
            info!("Started cleanup task for camera '{}'", camera_id);
            
            // Start frame forwarding task to populate the pre-recording buffer
            let frame_forwarding_task = self.start_frame_forwarding_task().await;
            tasks.push(frame_forwarding_task);
            info!("Started frame forwarding task for camera '{}'", camera_id);
        } else {
            info!("No pre-recording buffer configured for camera '{}'", camera_id);
        }
        
        let rtsp_client = self.rtsp_client;
        tokio::spawn(async move {
            info!("Starting video stream for camera '{}'", camera_id);
            
            // Start RTSP client
            let rtsp_task = tokio::spawn(async move {
                if let Err(e) = rtsp_client.start().await {
                    error!("RTSP client error for camera '{}': {}", camera_id, e);
                }
            });
            
            // Wait for either RTSP client to finish or any buffer task to finish
            if tasks.is_empty() {
                // No pre-recording buffer, just wait for RTSP client
                let _ = rtsp_task.await;
            } else {
                // Wait for any task to complete
                let mut all_tasks = vec![rtsp_task];
                all_tasks.extend(tasks);
                
                // Use select_all or join_all depending on desired behavior
                // For now, let's wait for all tasks to complete
                for task in all_tasks {
                    let _ = task.await;
                }
            }
        })
    }
    
    /// Start a task that forwards frames from the broadcast channel to the pre-recording buffer
    async fn start_frame_forwarding_task(&self) -> tokio::task::JoinHandle<()> {
        let frame_receiver = self.frame_sender.subscribe();
        let buffer = self.pre_recording_buffer.as_ref().unwrap().clone();
        let camera_id = self.camera_id.clone();
        
        tokio::spawn(async move {
            let mut receiver = frame_receiver;
            info!("Pre-recording frame forwarding task started for camera '{}'", camera_id);
            loop {
                match receiver.recv().await {
                    Ok(frame_data) => {
                        buffer.add_frame(frame_data).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        error!("Pre-recording buffer lagged for camera '{}', skipped {} frames", camera_id, skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Frame channel closed for camera '{}', stopping pre-recording buffer", camera_id);
                        break;
                    }
                }
            }
        })
    }
}