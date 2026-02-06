use axum::{Json, response::IntoResponse, extract::{Path as AxumPath}};
use tracing::info;

use crate::{config, api_recording::ApiResponse, AppState, Args};

fn check_admin_token(headers: &axum::http::HeaderMap, admin_token: &Option<String>) -> bool {
    let Some(ref expected_token) = admin_token else { return true; };
    if let Some(auth_header) = headers.get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            let token = if auth_str.starts_with("Bearer ") { &auth_str[7..] } else { auth_str };
            return token == expected_token;
        }
    }
    false
}

pub async fn api_get_camera_config(
    _headers: axum::http::HeaderMap,
    path: AxumPath<String>,
    state: AppState,
) -> axum::response::Response {
    let camera_id = path.0;
    let camera_configs = state.camera_configs.read().await;
    if let Some(camera_config) = camera_configs.get(&camera_id) {
        Json(ApiResponse::success(camera_config.clone())).into_response()
    } else {
        (axum::http::StatusCode::NOT_FOUND,
         Json(ApiResponse::<()>::error("Camera configuration not found", 404)))
        .into_response()
    }
}

#[derive(serde::Deserialize)]
pub struct CreateCameraRequest {
    pub camera_id: String,
    pub config: config::CameraConfig,
}

pub async fn api_create_camera(
    headers: axum::http::HeaderMap,
    body: axum::extract::Json<CreateCameraRequest>,
    state: AppState,
) -> axum::response::Response {
    if !check_admin_token(&headers, &state.admin_token) {
        return (axum::http::StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::error("Unauthorized", 401)))
               .into_response();
    }
    let camera_id = body.camera_id.clone();
    let camera_config = body.config.clone();

    let camera_configs = state.camera_configs.read().await;
    if camera_configs.contains_key(&camera_id) {
        return (axum::http::StatusCode::CONFLICT,
                Json(ApiResponse::<()>::error("Camera already exists", 409)))
               .into_response();
    }
    drop(camera_configs);

    if camera_config.path.is_empty() || camera_config.url.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
                Json(ApiResponse::<()>::error("Path and URL are required", 400)))
               .into_response();
    }

    if let Err(e) = config::Config::save_camera_config(&camera_id, &camera_config, Some(&state.cameras_directory)) {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error(&format!("Failed to save camera config: {}", e), 500)))
               .into_response();
    }

    info!("Camera '{}' created successfully", camera_id);

    Json(ApiResponse::success(serde_json::json!({
        "message": "Camera created successfully",
        "camera_id": camera_id
    }))).into_response()
}

pub async fn api_update_camera(
    headers: axum::http::HeaderMap,
    path: AxumPath<String>,
    body: axum::extract::Json<config::CameraConfig>,
    state: AppState,
) -> axum::response::Response {
    if !check_admin_token(&headers, &state.admin_token) {
        return (axum::http::StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::error("Unauthorized", 401)))
               .into_response();
    }
    let camera_id = path.0;
    let camera_config = body.0;

    let camera_configs = state.camera_configs.read().await;
    if !camera_configs.contains_key(&camera_id) {
        return (axum::http::StatusCode::NOT_FOUND,
                Json(ApiResponse::<()>::error("Camera not found", 404)))
               .into_response();
    }
    drop(camera_configs);

    if camera_config.path.is_empty() || camera_config.url.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
                Json(ApiResponse::<()>::error("Path and URL are required", 400)))
               .into_response();
    }

    if let Err(e) = config::Config::save_camera_config(&camera_id, &camera_config, Some(&state.cameras_directory)) {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error(&format!("Failed to save camera config: {}", e), 500)))
               .into_response();
    }

    info!("Camera '{}' updated successfully", camera_id);

    Json(ApiResponse::success(serde_json::json!({
        "message": "Camera updated successfully",
        "camera_id": camera_id
    }))).into_response()
}

pub async fn api_delete_camera(
    headers: axum::http::HeaderMap,
    path: AxumPath<String>,
    state: AppState,
) -> axum::response::Response {
    if !check_admin_token(&headers, &state.admin_token) {
        return (axum::http::StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::error("Unauthorized", 401)))
               .into_response();
    }
    let camera_id = path.0;

    let camera_configs = state.camera_configs.read().await;
    if !camera_configs.contains_key(&camera_id) {
        return (axum::http::StatusCode::NOT_FOUND,
                Json(ApiResponse::<()>::error("Camera not found", 404)))
               .into_response();
    }
    drop(camera_configs);

    if let Err(e) = state.remove_camera(&camera_id).await {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error(&format!("Failed to stop camera stream: {}", e), 500)))
               .into_response();
    }

    if let Err(e) = config::Config::delete_camera_config(&camera_id, Some(&state.cameras_directory)) {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error(&format!("Failed to delete camera config: {}", e), 500)))
               .into_response();
    }

    info!("Camera '{}' deleted successfully", camera_id);

    Json(ApiResponse::success(serde_json::json!({
        "message": "Camera deleted successfully",
        "camera_id": camera_id
    }))).into_response()
}

pub async fn api_get_config(
    headers: axum::http::HeaderMap,
    args: Args,
    state: AppState,
) -> axum::response::Response {
    // Check admin token using the in-memory config from AppState
    if !check_admin_token(&headers, &state.admin_token) {
        return (axum::http::StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::error("Unauthorized", 401)))
               .into_response();
    }

    let config_path = &args.config;

    // Try to load config from file first
    match std::fs::read_to_string(config_path) {
        Ok(content) => {
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(json_value) => Json(ApiResponse::success(json_value)).into_response(),
                Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                          Json(ApiResponse::<()>::error(&format!("Failed to parse config JSON: {}", e), 500)))
                         .into_response()
            }
        }
        Err(_) => {
            // If file doesn't exist, return the in-memory config
            // Build config from AppState components
            let cameras = state.camera_configs.read().await;
            let config = config::Config {
                server: (*state.server_config).clone(),
                cameras: cameras.clone(),
                transcoding: (*state.transcoding_config).clone(),
                mqtt: None, // We don't store the full MQTT config in AppState
                recording: state.recording_config.as_ref().map(|rc| (**rc).clone()),
            };
            drop(cameras);
            
            Json(ApiResponse::success(config)).into_response()
        }
    }
}

/// Compare old and new config JSON values and return which top-level sections changed.
fn detect_changed_sections(old_config: &serde_json::Value, new_config: &serde_json::Value) -> Vec<String> {
    let sections = ["server", "transcoding", "mqtt", "recording"];
    let mut changed = Vec::new();

    for section in &sections {
        let old_section = old_config.get(*section);
        let new_section = new_config.get(*section);
        if old_section != new_section {
            changed.push(section.to_string());
        }
    }

    changed
}

/// Recursively merge JSON values, updating target with values from source
fn merge_json_values(target: &mut serde_json::Value, source: &serde_json::Value) {
    match (target.as_object_mut(), source.as_object()) {
        (Some(target_map), Some(source_map)) => {
            for (key, value) in source_map {
                if target_map.contains_key(key) {
                    merge_json_values(target_map.get_mut(key).unwrap(), value);
                } else {
                    target_map.insert(key.clone(), value.clone());
                }
            }
        }
        _ => {
            *target = source.clone();
        }
    }
}

pub async fn api_update_config(
    headers: axum::http::HeaderMap,
    body: axum::extract::Json<serde_json::Value>,
    args: Args,
    state: AppState,
) -> axum::response::Response {
    // Check admin token using the in-memory config from AppState
    if !check_admin_token(&headers, &state.admin_token) {
        return (axum::http::StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::error("Unauthorized", 401)))
               .into_response();
    }

    let config_path = &args.config;

    // Try to load current config from file, or use in-memory config if file doesn't exist
    let current_config = match config::Config::load(config_path) {
        Ok(config) => config,
        Err(_) => {
            // If file doesn't exist, build config from AppState
            let cameras = state.camera_configs.read().await;
            let config = config::Config {
                server: (*state.server_config).clone(),
                cameras: cameras.clone(),
                transcoding: (*state.transcoding_config).clone(),
                mqtt: None,
                recording: state.recording_config.as_ref().map(|rc| (**rc).clone()),
            };
            drop(cameras);
            config
        }
    };

    let mut current_config_value = match serde_json::to_value(&current_config) {
        Ok(val) => val,
        Err(e) => {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                   Json(ApiResponse::<()>::error(&format!("Failed to serialize current config: {}", e), 500)))
                  .into_response();
        }
    };

    if let Some(obj) = current_config_value.as_object_mut() {
        obj.remove("cameras");
    }

    // Save a copy of the old config before merging for change detection
    let old_config_value = current_config_value.clone();

    merge_json_values(&mut current_config_value, &body.0);

    match serde_json::from_value::<config::Config>(current_config_value.clone()) {
        Ok(_) => {
            let content = match serde_json::to_string_pretty(&current_config_value) {
                Ok(json) => json,
                Err(e) => {
                    return (axum::http::StatusCode::BAD_REQUEST,
                           Json(ApiResponse::<()>::error(&format!("Failed to serialize JSON: {}", e), 400)))
                          .into_response();
                }
            };

            match std::fs::write(config_path, content) {
                Ok(_) => {
                    let changed_sections = detect_changed_sections(&old_config_value, &current_config_value);

                    if changed_sections.is_empty() {
                        info!("Server configuration saved (no changes detected)");
                        Json(ApiResponse::success(serde_json::json!({
                            "message": "Configuration saved (no changes detected)",
                            "restart_required": false,
                            "camera_restart_recommended": false
                        }))).into_response()
                    } else {
                        let camera_affecting: Vec<&String> = changed_sections.iter()
                            .filter(|s| matches!(s.as_str(), "transcoding" | "recording" | "mqtt"))
                            .collect();
                        let camera_restart_recommended = !camera_affecting.is_empty();

                        let note = if camera_restart_recommended {
                            let section_names = camera_affecting.iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join("/");
                            format!(
                                "Server restart required. Some cameras may also need restarting because global {} settings changed.",
                                section_names
                            )
                        } else {
                            "Server restart required to apply changes.".to_string()
                        };

                        info!("Server configuration updated successfully (changed: {:?})", changed_sections);
                        Json(ApiResponse::success(serde_json::json!({
                            "message": "Configuration updated successfully",
                            "restart_required": true,
                            "camera_restart_recommended": camera_restart_recommended,
                            "changed_sections": changed_sections,
                            "note": note
                        }))).into_response()
                    }
                }
                Err(e) => {
                    (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                     Json(ApiResponse::<()>::error(&format!("Failed to write config file: {}", e), 500)))
                    .into_response()
                }
            }
        }
        Err(e) => {
            (axum::http::StatusCode::BAD_REQUEST,
             Json(ApiResponse::<()>::error(&format!("Invalid configuration: {}", e), 400)))
            .into_response()
        }
    }
}
