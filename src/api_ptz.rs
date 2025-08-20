use std::sync::Arc;
use axum::{Json, response::IntoResponse};
use serde::Deserialize;

use crate::config;
use crate::ptz::{PtzVelocity, PtzPresetRequest, PtzController, onvif_ptz::OnvifPtz};

#[derive(Debug, Deserialize)]
pub struct MoveRequest {
    pub pan: f32,
    pub tilt: f32,
    pub zoom: Option<f32>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct PresetRequest {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct SetPresetRequest {
    pub name: Option<String>,
    pub token: Option<String>,
}

fn check_auth(headers: &axum::http::HeaderMap, camera_config: &config::CameraConfig) -> std::result::Result<(), axum::response::Response> {
    if let Some(expected_token) = &camera_config.token {
        if let Some(auth_header) = headers.get("authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    if token == expected_token { return Ok(()); }
                }
            }
        }
        return Err((axum::http::StatusCode::UNAUTHORIZED, "Invalid or missing Authorization header").into_response());
    }
    Ok(())
}

fn build_ptz_controller(camera_config: &config::CameraConfig) -> Result<Arc<dyn PtzController>, axum::response::Response> {
    let ptz_cfg = match &camera_config.ptz { Some(p) if p.enabled => p, _ => {
        return Err((axum::http::StatusCode::SERVICE_UNAVAILABLE, "PTZ not enabled for this camera").into_response());
    }};
    if ptz_cfg.protocol.to_lowercase() == "onvif" {
        let endpoint = ptz_cfg.onvif_url.clone().ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST, "Missing onvif_url in PTZ config").into_response())?;
        let profile = ptz_cfg.profile_token.clone().unwrap_or_else(|| "profile1".to_string());
        let controller = OnvifPtz::new(endpoint, ptz_cfg.username.clone(), ptz_cfg.password.clone(), profile);
        Ok(Arc::new(controller))
    } else {
        Err((axum::http::StatusCode::BAD_REQUEST, "Unsupported PTZ protocol").into_response())
    }
}

pub async fn api_ptz_move(headers: axum::http::HeaderMap, axum::extract::Json(req): Json<MoveRequest>, camera_config: config::CameraConfig) -> axum::response::Response {
    if let Err(resp) = check_auth(&headers, &camera_config) { return resp; }
    let ctrl = match build_ptz_controller(&camera_config) { Ok(c) => c, Err(r) => return r };
    let vel = PtzVelocity { pan: req.pan, tilt: req.tilt, zoom: req.zoom.unwrap_or(0.0) };
    match ctrl.continuous_move(vel, req.timeout_secs).await {
        Ok(_) => (axum::http::StatusCode::OK, "ok").into_response(),
        Err(e) => (axum::http::StatusCode::BAD_GATEWAY, format!("PTZ move failed: {}", e)).into_response(),
    }
}

pub async fn api_ptz_stop(headers: axum::http::HeaderMap, camera_config: config::CameraConfig) -> axum::response::Response {
    if let Err(resp) = check_auth(&headers, &camera_config) { return resp; }
    let ctrl = match build_ptz_controller(&camera_config) { Ok(c) => c, Err(r) => return r };
    match ctrl.stop().await {
        Ok(_) => (axum::http::StatusCode::OK, "ok").into_response(),
        Err(e) => (axum::http::StatusCode::BAD_GATEWAY, format!("PTZ stop failed: {}", e)).into_response(),
    }
}

pub async fn api_ptz_goto_preset(headers: axum::http::HeaderMap, axum::extract::Json(req): Json<PresetRequest>, camera_config: config::CameraConfig) -> axum::response::Response {
    if let Err(resp) = check_auth(&headers, &camera_config) { return resp; }
    let ctrl = match build_ptz_controller(&camera_config) { Ok(c) => c, Err(r) => return r };
    match ctrl.goto_preset(&req.token, None).await {
        Ok(_) => (axum::http::StatusCode::OK, "ok").into_response(),
        Err(e) => (axum::http::StatusCode::BAD_GATEWAY, format!("PTZ goto preset failed: {}", e)).into_response(),
    }
}

pub async fn api_ptz_set_preset(headers: axum::http::HeaderMap, axum::extract::Json(req): Json<SetPresetRequest>, camera_config: config::CameraConfig) -> axum::response::Response {
    if let Err(resp) = check_auth(&headers, &camera_config) { return resp; }
    let ctrl = match build_ptz_controller(&camera_config) { Ok(c) => c, Err(r) => return r };
    match ctrl.set_preset(PtzPresetRequest { name: req.name, token: req.token }).await {
        Ok(token) => (axum::http::StatusCode::OK, Json(serde_json::json!({"preset_token": token}))).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_GATEWAY, format!("PTZ set preset failed: {}", e)).into_response(),
    }
}
