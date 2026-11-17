//! Admin-only device registry API.
//!
//! Devices are normally auto-registered when a tracker first connects. These
//! endpoints let an admin pre-register an IMEI with a vehicle name, rename it,
//! or delete the device together with its history.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Extension,
};
use serde::Deserialize;
use serde_json::json;

use crate::api::auth::CurrentUser;
use crate::ws::AppState;

fn admin_guard(user: &CurrentUser) -> Option<Response> {
    if user.is_admin() {
        None
    } else {
        Some((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "admin access required" })),
        )
            .into_response())
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub fn validate_imei(imei: &str) -> Result<(), String> {
    if imei.len() != 15 || !imei.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("IMEI must be exactly 15 digits".to_string());
    }
    Ok(())
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Vehicle name is required".to_string());
    }
    if name.chars().count() > 60 {
        return Err("Vehicle name must be at most 60 characters".to_string());
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct CreateDeviceRequest {
    pub imei: String,
    pub name: String,
}

pub async fn create_device(
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
    Json(body): Json<CreateDeviceRequest>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    let imei = body.imei.trim().to_string();
    let name = body.name.trim().to_string();
    if let Err(message) = validate_imei(&imei) {
        return error(StatusCode::BAD_REQUEST, &message);
    }
    if let Err(message) = validate_name(&name) {
        return error(StatusCode::BAD_REQUEST, &message);
    }

    let device = {
        let storage = state.storage.lock().unwrap();
        match storage.create_device(&imei, &name) {
            Ok(()) => storage.get_device(&imei).ok().flatten(),
            Err(err) => {
                let message = if err.to_string().contains("UNIQUE") {
                    "A device with that IMEI already exists"
                } else {
                    "Could not add device"
                };
                return error(StatusCode::CONFLICT, message);
            }
        }
    };

    match device {
        Some(device) => Json(json!({ "ok": true, "device": device })).into_response(),
        None => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load the new device",
        ),
    }
}

#[derive(Deserialize)]
pub struct UpdateDeviceRequest {
    pub name: String,
}

pub async fn update_device(
    Path(imei): Path<String>,
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
    Json(body): Json<UpdateDeviceRequest>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    let name = body.name.trim().to_string();
    if let Err(message) = validate_name(&name) {
        return error(StatusCode::BAD_REQUEST, &message);
    }

    let device = {
        let storage = state.storage.lock().unwrap();
        match storage.update_device_name(&imei, &name) {
            Ok(true) => storage.get_device(&imei).ok().flatten(),
            Ok(false) => return error(StatusCode::NOT_FOUND, "Device not found"),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not rename device"),
        }
    };

    match device {
        Some(device) => Json(json!({ "ok": true, "device": device })).into_response(),
        None => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not load the updated device",
        ),
    }
}

pub async fn delete_device(
    Path(imei): Path<String>,
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    let storage = state.storage.lock().unwrap();
    match storage.delete_device(&imei) {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "Device not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not delete device"),
    }
}
