use std::path::Path;
use std::fs;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use notify::{Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{info, error};

use crate::config;
use crate::errors::{Result, StreamError};

// Re-export AppState for the watcher functions
pub use crate::AppState;

pub async fn start_camera_config_watcher(app_state: AppState) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(100);
    
    // Create file watcher
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            match res {
                Ok(event) => {
                    if let Err(e) = tx.blocking_send(event) {
                        error!("Failed to send file watcher event: {}", e);
                    }
                }
                Err(e) => error!("File watcher error: {}", e),
            }
        },
        NotifyConfig::default(),
    ).map_err(|e| StreamError::config(&format!("File watcher error: {}", e)))?;
    
    // Watch the cameras directory
    let cameras_dir_path = Path::new(&app_state.cameras_directory);
    if !cameras_dir_path.exists() {
        info!("Creating cameras directory '{}' for watching...", app_state.cameras_directory);
        fs::create_dir_all(cameras_dir_path)?;
    }
    
    watcher.watch(cameras_dir_path, RecursiveMode::NonRecursive)
        .map_err(|e| StreamError::config(&format!("Failed to watch cameras directory: {}", e)))?;
    info!("Started watching cameras directory '{}' for configuration changes", app_state.cameras_directory);
    
    // Keep watcher alive and handle events with debouncing
    tokio::spawn(async move {
        let _watcher = watcher; // Keep watcher alive
        let mut last_events: HashMap<String, Instant> = HashMap::new();
        
        while let Some(event) = rx.recv().await {
            // Debounce events for each camera to prevent rapid duplicate calls
            let mut should_process = false;
            if let Some(camera_id) = event.paths.get(0).and_then(|p| get_camera_id_from_path(p)) {
                let now = Instant::now();
                let should_process_this = if let Some(last_time) = last_events.get(&camera_id) {
                    now.duration_since(*last_time) >= Duration::from_millis(500) // 500ms debounce
                } else {
                    true
                };
                
                if should_process_this {
                    last_events.insert(camera_id, now);
                    should_process = true;
                }
            } else {
                should_process = true; // Process events we can't identify
            }
            
            if should_process {
                handle_file_event(event, &app_state).await;
            }
        }
    });
    
    Ok(())
}

async fn handle_file_event(event: Event, app_state: &AppState) {
    match event.kind {
        EventKind::Create(_) => {
            for path in event.paths {
                if let Some(camera_id) = get_camera_id_from_path(&path) {
                    info!("Detected new camera configuration: {}", camera_id);
                    if let Ok(camera_config) = load_camera_config(&camera_id, &app_state.cameras_directory) {
                        if let Err(e) = app_state.add_camera(camera_id.clone(), camera_config).await {
                            error!("Failed to add camera '{}': {}", camera_id, e);
                        }
                    }
                }
            }
        }
        EventKind::Modify(_) => {
            for path in event.paths {
                if let Some(camera_id) = get_camera_id_from_path(&path) {
                    info!("Detected camera configuration change: {}", camera_id);
                    if let Ok(camera_config) = load_camera_config(&camera_id, &app_state.cameras_directory) {
                        if let Err(e) = app_state.restart_camera(camera_id.clone(), camera_config).await {
                            error!("Failed to restart camera '{}': {}", camera_id, e);
                        }
                    }
                }
            }
        }
        EventKind::Remove(_) => {
            for path in event.paths {
                if let Some(camera_id) = get_camera_id_from_path(&path) {
                    info!("Detected camera configuration removal: {}", camera_id);
                    if let Err(e) = app_state.remove_camera(&camera_id).await {
                        error!("Failed to remove camera '{}': {}", camera_id, e);
                    }
                }
            }
        }
        _ => {
            // Ignore other event types
        }
    }
}

fn get_camera_id_from_path(path: &Path) -> Option<String> {
    if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
        if file_name.ends_with(".json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                return Some(stem.to_string());
            }
        }
    }
    None
}

fn load_camera_config(camera_id: &str, cameras_dir: &str) -> Result<config::CameraConfig> {
    let json_path = format!("{}/{}.json", cameras_dir, camera_id);
    
    let content = fs::read_to_string(&json_path)
        .map_err(|e| StreamError::config(&format!("Failed to read camera config file {}: {}", json_path, e)))?;
    
    serde_json::from_str::<config::CameraConfig>(&content)
        .map_err(|e| StreamError::config(&format!("Failed to parse JSON camera config file {}: {}", json_path, e)))
}