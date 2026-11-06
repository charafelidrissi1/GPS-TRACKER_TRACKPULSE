//! Platform settings API.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Extension,
};
use serde::Deserialize;
use serde_json::json;

use crate::api::auth::CurrentUser;
use crate::storage::AppSettings;
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

/// Public branding data used by the login/setup pages and the top bar.
pub async fn public_settings(State(state): State<AppState>) -> Json<serde_json::Value> {
    let settings = {
        let storage = state.storage.lock().unwrap();
        storage.get_settings().unwrap_or_default()
    };
    Json(json!({ "platform_name": settings.platform_name }))
}

pub async fn get_settings(
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    let storage = state.storage.lock().unwrap();
    match storage.get_settings() {
        Ok(settings) => Json(settings).into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load settings"),
    }
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    pub platform_name: String,
    pub overspeed_kmh: u16,
    pub online_timeout_seconds: i64,
}

pub async fn update_settings(
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
    Json(body): Json<UpdateSettingsRequest>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    let platform_name = body.platform_name.trim().to_string();
    if platform_name.is_empty() || platform_name.len() > 60 {
        return error(
            StatusCode::BAD_REQUEST,
            "Platform name must be between 1 and 60 characters",
        );
    }
    if !(1..=300).contains(&body.overspeed_kmh) {
        return error(
            StatusCode::BAD_REQUEST,
            "Overspeed threshold must be between 1 and 300 km/h",
        );
    }
    if !(60..=86_400).contains(&body.online_timeout_seconds) {
        return error(
            StatusCode::BAD_REQUEST,
            "Online timeout must be between 1 and 1440 minutes",
        );
    }

    let settings = AppSettings {
        platform_name,
        overspeed_kmh: body.overspeed_kmh,
        online_timeout_seconds: body.online_timeout_seconds,
    };

    let storage = state.storage.lock().unwrap();
    match storage.update_settings(&settings) {
        Ok(()) => Json(settings).into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not save settings"),
    }
}
