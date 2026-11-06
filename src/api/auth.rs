//! Authentication, sessions, and the first-run setup flow.
//!
//! Passwords are hashed with Argon2. A random session token is stored in the
//! `sessions` table and sent to the browser as an HttpOnly cookie.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Json, Redirect, Response},
    Extension,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::storage::User;
use crate::ws::AppState;

pub const SESSION_COOKIE: &str = "tp_session";
const SESSION_TTL_SECONDS: i64 = 7 * 24 * 3600;

/// The authenticated user attached to a request by the middleware.
#[derive(Debug, Clone, Serialize)]
pub struct CurrentUser {
    pub id: i64,
    pub username: String,
    pub role: String,
}

impl From<User> for CurrentUser {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role: user.role,
        }
    }
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

// ---------------------------------------------------------------------------
// Password + validation helpers
// ---------------------------------------------------------------------------

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| err.to_string())
}

pub fn verify_password(password: &str, password_hash: &str) -> bool {
    PasswordHash::new(password_hash)
        .ok()
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

pub fn validate_username(username: &str) -> Result<(), String> {
    let username = username.trim();
    if !(3..=32).contains(&username.len()) {
        return Err("Username must be between 3 and 32 characters".to_string());
    }
    if !username
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err("Username may only contain letters, numbers, dot, dash and underscore".to_string());
    }
    Ok(())
}

pub fn validate_password(password: &str) -> Result<(), String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".to_string());
    }
    if password.len() > 128 {
        return Err("Password must be at most 128 characters".to_string());
    }
    Ok(())
}

pub fn normalize_username(username: &str) -> String {
    username.trim().to_string()
}

// ---------------------------------------------------------------------------
// Cookies + sessions
// ---------------------------------------------------------------------------

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("tp_session=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn session_cookie(token: &str, max_age: i64) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        SESSION_COOKIE, token, max_age
    )
}

fn clear_session_cookie() -> String {
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        SESSION_COOKIE
    )
}

fn attach_cookie(mut response: Response, cookie: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

fn session_user(state: &AppState, headers: &HeaderMap) -> Option<User> {
    let token = cookie_token(headers)?;
    let storage = state.storage.lock().ok()?;
    storage
        .get_session_user(&token, Utc::now().timestamp())
        .ok()
        .flatten()
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn new_session(state: &AppState, user_id: i64) -> Result<(String, i64), ()> {
    let token = Uuid::new_v4().simple().to_string();
    let expires_at = Utc::now().timestamp() + SESSION_TTL_SECONDS;
    let storage = state.storage.lock().map_err(|_| ())?;
    let _ = storage.cleanup_expired_sessions(Utc::now().timestamp());
    storage
        .create_session(&token, user_id, expires_at)
        .map_err(|_| ())?;
    Ok((token, expires_at))
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();

    let has_users = {
        let storage = state.storage.lock().unwrap();
        storage.user_count().unwrap_or(0) > 0
    };

    // First run: only the setup flow, its assets, and public branding are open.
    if !has_users {
        if matches!(
            path.as_str(),
            "/setup" | "/api/auth/setup" | "/api/auth/status" | "/api/settings/public" | "/auth.js"
        ) {
            return next.run(req).await;
        }
        if path.starts_with("/api") || path == "/ws" {
            return json_error(StatusCode::PRECONDITION_REQUIRED, "setup required");
        }
        return Redirect::to("/setup").into_response();
    }

    if path == "/setup" {
        return if session_user(&state, req.headers()).is_some() {
            Redirect::to("/map").into_response()
        } else {
            Redirect::to("/login").into_response()
        };
    }

    if path == "/login" {
        return if session_user(&state, req.headers()).is_some() {
            Redirect::to("/map").into_response()
        } else {
            next.run(req).await
        };
    }

    let public = matches!(path.as_str(), "/auth.js" | "/favicon.ico")
        || matches!(
            path.as_str(),
            "/api/auth/login" | "/api/auth/status" | "/api/settings/public"
        );

    if public {
        return next.run(req).await;
    }

    match session_user(&state, req.headers()) {
        Some(user) => {
            req.extensions_mut().insert(CurrentUser::from(user));
            next.run(req).await
        }
        None => {
            if path.starts_with("/api") || path == "/ws" {
                json_error(StatusCode::UNAUTHORIZED, "unauthorized")
            } else {
                Redirect::to("/login").into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pages and assets
// ---------------------------------------------------------------------------

pub async fn login_page() -> Html<&'static str> {
    Html(include_str!("login.html"))
}

pub async fn setup_page() -> Html<&'static str> {
    Html(include_str!("setup.html"))
}

pub async fn auth_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        include_str!("auth.js"),
    )
}

// ---------------------------------------------------------------------------
// Auth API
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Response {
    let username = normalize_username(&body.username);
    if username.is_empty() || body.password.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "Username and password are required");
    }

    let auth = {
        let storage = state.storage.lock().unwrap();
        storage.find_user_by_username(&username).ok().flatten()
    };

    let Some(auth) = auth else {
        return json_error(StatusCode::UNAUTHORIZED, "Invalid username or password");
    };

    if !auth.user.active {
        return json_error(StatusCode::FORBIDDEN, "This account is disabled");
    }

    if !verify_password(&body.password, &auth.password_hash) {
        return json_error(StatusCode::UNAUTHORIZED, "Invalid username or password");
    }

    let (token, _expires_at) = match new_session(&state, auth.user.id) {
        Ok(session) => session,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not create session"),
    };

    {
        let storage = state.storage.lock().unwrap();
        let _ = storage.set_last_login(auth.user.id);
    }

    let user = CurrentUser::from(auth.user);
    let response = Json(json!({ "ok": true, "user": user })).into_response();
    attach_cookie(response, &session_cookie(&token, SESSION_TTL_SECONDS))
}

#[derive(Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

pub async fn setup(
    State(state): State<AppState>,
    Json(body): Json<SetupRequest>,
) -> Response {
    let user_count = {
        let storage = state.storage.lock().unwrap();
        storage.user_count().unwrap_or(0)
    };
    if user_count > 0 {
        return json_error(StatusCode::CONFLICT, "Setup is already complete");
    }

    if let Err(message) = validate_username(&body.username) {
        return json_error(StatusCode::BAD_REQUEST, &message);
    }
    if let Err(message) = validate_password(&body.password) {
        return json_error(StatusCode::BAD_REQUEST, &message);
    }

    let username = normalize_username(&body.username);
    let password_hash = match hash_password(&body.password) {
        Ok(hash) => hash,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not hash password"),
    };

    let user_id = {
        let storage = state.storage.lock().unwrap();
        match storage.create_user(&username, &password_hash, "admin") {
            Ok(id) => id,
            Err(err) => {
                let message = if err.to_string().contains("UNIQUE") {
                    "That username is already taken"
                } else {
                    "Could not create the admin account"
                };
                return json_error(StatusCode::CONFLICT, message);
            }
        }
    };

    let (token, _expires_at) = match new_session(&state, user_id) {
        Ok(session) => session,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not create session"),
    };

    {
        let storage = state.storage.lock().unwrap();
        let _ = storage.set_last_login(user_id);
    }

    let user = CurrentUser {
        id: user_id,
        username,
        role: "admin".to_string(),
    };
    let response = Json(json!({ "ok": true, "user": user })).into_response();
    attach_cookie(response, &session_cookie(&token, SESSION_TTL_SECONDS))
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_token(&headers) {
        let storage = state.storage.lock().unwrap();
        let _ = storage.delete_session(&token);
    }
    let response = Json(json!({ "ok": true })).into_response();
    attach_cookie(response, &clear_session_cookie())
}

pub async fn me(Extension(user): Extension<CurrentUser>) -> Json<CurrentUser> {
    Json(user)
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Extension(current): Extension<CurrentUser>,
    Json(body): Json<ChangePasswordRequest>,
) -> Response {
    let auth = {
        let storage = state.storage.lock().unwrap();
        storage
            .find_user_by_username(&current.username)
            .ok()
            .flatten()
    };

    let Some(auth) = auth else {
        return json_error(StatusCode::UNAUTHORIZED, "Account not found");
    };

    if !verify_password(&body.current_password, &auth.password_hash) {
        return json_error(StatusCode::BAD_REQUEST, "Current password is incorrect");
    }
    if let Err(message) = validate_password(&body.new_password) {
        return json_error(StatusCode::BAD_REQUEST, &message);
    }
    if body.current_password == body.new_password {
        return json_error(StatusCode::BAD_REQUEST, "New password must be different");
    }

    let password_hash = match hash_password(&body.new_password) {
        Ok(hash) => hash,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not hash password"),
    };

    {
        let storage = state.storage.lock().unwrap();
        let _ = storage.update_user_password(current.id, &password_hash);
        let _ = storage.delete_user_sessions(current.id);
    }

    let (token, _expires_at) = match new_session(&state, current.id) {
        Ok(session) => session,
        Err(_) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not create session"),
    };

    let response = Json(json!({ "ok": true })).into_response();
    attach_cookie(response, &session_cookie(&token, SESSION_TTL_SECONDS))
}

#[derive(Serialize)]
pub struct AuthStatus {
    pub setup_required: bool,
    pub authenticated: bool,
    pub user: Option<CurrentUser>,
}

pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> Json<AuthStatus> {
    let setup_required = {
        let storage = state.storage.lock().unwrap();
        storage.user_count().unwrap_or(0) == 0
    };
    let user = session_user(&state, &headers).map(CurrentUser::from);
    Json(AuthStatus {
        setup_required,
        authenticated: user.is_some(),
        user,
    })
}
