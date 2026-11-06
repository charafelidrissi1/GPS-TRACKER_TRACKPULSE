//! Admin-only user management API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Extension,
};
use serde::Deserialize;
use serde_json::json;

use crate::api::auth::{
    hash_password, normalize_username, validate_password, validate_username, CurrentUser,
};
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

fn valid_role(role: &str) -> bool {
    role == "admin" || role == "viewer"
}

pub async fn list_users(
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    let storage = state.storage.lock().unwrap();
    match storage.list_users() {
        Ok(users) => Json(json!({ "users": users })).into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load users"),
    }
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub role: Option<String>,
}

pub async fn create_user(
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
    Json(body): Json<CreateUserRequest>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    if let Err(message) = validate_username(&body.username) {
        return error(StatusCode::BAD_REQUEST, &message);
    }
    if let Err(message) = validate_password(&body.password) {
        return error(StatusCode::BAD_REQUEST, &message);
    }

    let role = body.role.unwrap_or_else(|| "viewer".to_string());
    if !valid_role(&role) {
        return error(StatusCode::BAD_REQUEST, "Role must be admin or viewer");
    }

    let username = normalize_username(&body.username);
    let password_hash = match hash_password(&body.password) {
        Ok(hash) => hash,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not hash password"),
    };

    let user = {
        let storage = state.storage.lock().unwrap();
        match storage.create_user(&username, &password_hash, &role) {
            Ok(id) => storage.get_user(id).ok().flatten(),
            Err(err) => {
                let message = if err.to_string().contains("UNIQUE") {
                    "That username is already taken"
                } else {
                    "Could not create user"
                };
                return error(StatusCode::CONFLICT, message);
            }
        }
    };

    match user {
        Some(user) => Json(json!({ "ok": true, "user": user })).into_response(),
        None => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load the new user"),
    }
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub password: Option<String>,
}

pub async fn update_user(
    Path(id): Path<i64>,
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
    Json(body): Json<UpdateUserRequest>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    if id == current.id {
        if body.active == Some(false) {
            return error(StatusCode::BAD_REQUEST, "You cannot disable your own account");
        }
        if body.role.as_deref() == Some("viewer") {
            return error(StatusCode::BAD_REQUEST, "You cannot remove your own admin role");
        }
        if body.password.is_some() {
            return error(
                StatusCode::BAD_REQUEST,
                "Use Change password in the user menu for your own account",
            );
        }
    }

    if let Some(role) = body.role.as_deref() {
        if !valid_role(role) {
            return error(StatusCode::BAD_REQUEST, "Role must be admin or viewer");
        }
    }

    let target = {
        let storage = state.storage.lock().unwrap();
        storage.get_user(id).ok().flatten()
    };
    let Some(target) = target else {
        return error(StatusCode::NOT_FOUND, "User not found");
    };

    let loses_active_admin = target.role == "admin"
        && target.active
        && (body.role.as_deref() == Some("viewer") || body.active == Some(false));

    if loses_active_admin {
        let active_admins = {
            let storage = state.storage.lock().unwrap();
            storage.count_active_admins().unwrap_or(0)
        };
        if active_admins <= 1 {
            return error(
                StatusCode::BAD_REQUEST,
                "At least one active admin account is required",
            );
        }
    }

    let new_password_hash = if let Some(password) = body.password.as_deref() {
        if let Err(message) = validate_password(password) {
            return error(StatusCode::BAD_REQUEST, &message);
        }
        match hash_password(password) {
            Ok(hash) => Some(hash),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not hash password"),
        }
    } else {
        None
    };

    let updated = {
        let storage = state.storage.lock().unwrap();
        if let Err(_) = storage.update_user(id, body.role.as_deref(), body.active) {
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not update user");
        }
        if let Some(hash) = new_password_hash {
            let _ = storage.update_user_password(id, &hash);
            let _ = storage.delete_user_sessions(id);
        }
        storage.get_user(id).ok().flatten()
    };

    match updated {
        Some(user) => Json(json!({ "ok": true, "user": user })).into_response(),
        None => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load the updated user"),
    }
}

pub async fn delete_user(
    Path(id): Path<i64>,
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
) -> Response {
    if let Some(response) = admin_guard(&current) {
        return response;
    }

    if id == current.id {
        return error(StatusCode::BAD_REQUEST, "You cannot delete your own account");
    }

    let target = {
        let storage = state.storage.lock().unwrap();
        storage.get_user(id).ok().flatten()
    };
    let Some(target) = target else {
        return error(StatusCode::NOT_FOUND, "User not found");
    };

    if target.role == "admin" && target.active {
        let active_admins = {
            let storage = state.storage.lock().unwrap();
            storage.count_active_admins().unwrap_or(0)
        };
        if active_admins <= 1 {
            return error(
                StatusCode::BAD_REQUEST,
                "At least one active admin account is required",
            );
        }
    }

    {
        let storage = state.storage.lock().unwrap();
        if storage.delete_user(id).is_err() {
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not delete user");
        }
    }

    Json(json!({ "ok": true })).into_response()
}
