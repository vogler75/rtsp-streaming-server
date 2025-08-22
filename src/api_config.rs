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
) -> axum::response::Response {
    let config_path = &args.config;

    let current_config = match config::Config::load(config_path) {
        Ok(config) => config,
        Err(e) => {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                   Json(ApiResponse::<()>::error(&format!("Failed to load config: {}", e), 500)))
                  .into_response();
        }
    };

    if !check_admin_token(&headers, &current_config.server.admin_token) {
        return (axum::http::StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::error("Unauthorized", 401)))
               .into_response();
    }

    match std::fs::read_to_string(config_path) {
        Ok(content) => {
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(json_value) => Json(ApiResponse::success(json_value)).into_response(),
                Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                          Json(ApiResponse::<()>::error(&format!("Failed to parse config JSON: {}", e), 500)))
                         .into_response()
            }
        }
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                  Json(ApiResponse::<()>::error(&format!("Failed to read config file: {}", e), 500)))
                 .into_response()
    }
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
) -> axum::response::Response {
    let config_path = &args.config;

    let current_config = match config::Config::load(config_path) {
        Ok(config) => config,
        Err(e) => {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                   Json(ApiResponse::<()>::error(&format!("Failed to load config: {}", e), 500)))
                  .into_response();
        }
    };

    if !check_admin_token(&headers, &current_config.server.admin_token) {
        return (axum::http::StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::error("Unauthorized", 401)))
               .into_response();
    }

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
                    info!("Server configuration updated successfully");
                    Json(ApiResponse::success(serde_json::json!({
                        "message": "Configuration updated successfully",
                        "note": "Server restart may be required for some changes to take effect"
                    }))).into_response()
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
