//! HTTP API + WebSocket for the fleet dashboard.
//!
//! Authentication (see `auth.rs`):
//!   GET  /setup                     — first-run admin setup page
//!   GET  /login                     — sign-in page
//!   POST /api/auth/setup            — create the first admin
//!   POST /api/auth/login            — sign in (sets HttpOnly session cookie)
//!   POST /api/auth/logout           — sign out
//!   GET  /api/auth/me               — current user
//!   GET  /api/auth/status           — setup/authentication status
//!   POST /api/auth/password         — change own password
//!
//! Fleet endpoints:
//!   GET  /ws                        — authenticated WebSocket live stream
//!   GET  /api/devices               — device list with online status (JSON)
//!   GET  /api/positions?imei=X&limit=N — latest positions (JSON)
//!   GET  /api/positions/{imei}      — position history (JSON)
//!   GET  /api/tracks?imei=X&from=&to=&limit=N — track trace for map (JSON)
//!   GET  /api/geofences             — list geofences (JSON)
//!   POST /api/geofences             — create geofence {name, geojson} (JSON)
//!   DELETE /api/geofences/{id}      — delete geofence
//!   GET  /api/events?imei=X&limit=N — event log (JSON)
//!
//! Admin endpoints:
//!   GET/POST    /api/users          — list/create users
//!   PATCH/DELETE /api/users/{id}    — update/delete a user
//!   GET/POST    /api/settings       — read/update platform settings
//!   GET         /api/settings/public — public branding
//!
//! Pages:
//!   GET  /map                       — Leaflet dashboard page
//!   GET  /reports                   — GPS telemetry table (timestamp, speed, IO data)
//!   GET  /replay                    — animated trip replay (Traccar-style)
//!   GET  /users                     — user management (admin only)
//!   GET  /settings                  — platform settings (admin only)

mod auth;
mod settings;
mod users;

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, delete, patch},
    Extension, Router,
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use tower_http::cors::CorsLayer;
use serde::{Deserialize, Serialize};

use crate::api::auth::CurrentUser;
use crate::storage::Position;
use crate::ws::{self, AppState};

// ---------------------------------------------------------------------------
// Query / request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PositionsQuery {
    pub imei: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Deserialize)]
pub struct TracksQuery {
    pub imei: String,
    #[serde(default)]
    pub from: Option<u64>,
    #[serde(default)]
    pub to: Option<u64>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub imei: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Deserialize)]
pub struct CreateGeofence {
    pub name: String,
    pub geojson: serde_json::Value,
}

fn default_limit() -> usize {
    100
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct DeviceList {
    pub devices: Vec<crate::storage::Device>,
}

#[derive(Serialize)]
pub struct PositionResponse {
    pub positions: Vec<Position>,
}

#[derive(Serialize)]
pub struct GeofenceList {
    pub geofences: Vec<crate::storage::Geofence>,
}

#[derive(Serialize)]
pub struct EventList {
    pub events: Vec<crate::storage::Event>,
}

#[derive(Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/login", get(auth::login_page))
        .route("/setup", get(auth::setup_page))
        .route("/auth.js", get(auth::auth_js))
        .route("/api/auth/status", get(auth::status))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/password", post(auth::change_password))
        .route("/api/settings/public", get(settings::public_settings))
        .route("/ws", get(ws::ws_handler))
        .route("/api/devices", get(list_devices))
        .route("/api/stats", get(get_stats))
        .route("/api/positions", get(get_positions))
        .route("/api/positions/{imei}", get(get_positions_by_imei))
        .route("/api/tracks", get(get_tracks))
        .route("/api/geofences", get(list_geofences).post(create_geofence))
        .route("/api/geofences/{id}", delete(delete_geofence))
        .route("/api/events", get(list_events))
        .route("/api/users", get(users::list_users).post(users::create_user))
        .route(
            "/api/users/{id}",
            patch(users::update_user).delete(users::delete_user),
        )
        .route(
            "/api/settings",
            get(settings::get_settings).post(settings::update_settings),
        )
        .route("/map", get(map_page))
        .route("/map.html", get(map_page))
        .route("/tracks.html", get(tracks_page))
        .route("/geofences.html", get(geofences_page))
        .route("/events.html", get(events_page))
        .route("/reports.html", get(reports_page))
        .route("/replay.html", get(replay_page))
        .route("/users", get(users_page))
        .route("/users.html", get(users_page))
        .route("/settings", get(settings_page))
        .route("/settings.html", get(settings_page))
        // Clean URLs
        .route("/reports", get(reports_page))
        .route("/replay", get(replay_page))
        .route("/tracks", get(tracks_page))
        .route("/geofences", get(geofences_page))
        .route("/events", get(events_page))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ))
        .layer(CorsLayer::permissive()) // allow browser dashboards on any origin
        .with_state(state)
}

pub async fn start(state: AppState, addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("HTTP API listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn root_handler() -> Redirect {
    Redirect::to("/map")
}

async fn list_devices(State(state): State<AppState>) -> Json<DeviceList> {
    let storage = state.storage.lock().unwrap();
    let devices = storage.get_devices().unwrap_or_default();
    Json(DeviceList { devices })
}

async fn get_stats(State(state): State<AppState>) -> Json<crate::storage::Stats> {
    let storage = state.storage.lock().unwrap();
    Json(storage.get_stats().unwrap_or_default())
}

async fn get_positions(
    Query(params): Query<PositionsQuery>,
    State(state): State<AppState>,
) -> Json<PositionResponse> {
    let storage = state.storage.lock().unwrap();
    let positions = storage.get_positions(&params.imei, params.limit).unwrap_or_default();
    Json(PositionResponse { positions })
}

async fn get_positions_by_imei(
    Path(imei): Path<String>,
    State(state): State<AppState>,
) -> Json<PositionResponse> {
    let storage = state.storage.lock().unwrap();
    let positions = storage.get_positions(&imei, 100).unwrap_or_default();
    Json(PositionResponse { positions })
}

async fn get_tracks(
    Query(params): Query<TracksQuery>,
    State(state): State<AppState>,
) -> Json<PositionResponse> {
    let storage = state.storage.lock().unwrap();
    let positions = storage
        .get_tracks(&params.imei, params.from, params.to, params.limit)
        .unwrap_or_default();
    Json(PositionResponse { positions })
}

async fn list_geofences(State(state): State<AppState>) -> Json<GeofenceList> {
    let storage = state.storage.lock().unwrap();
    let geofences = storage.get_geofences().unwrap_or_default();
    Json(GeofenceList { geofences })
}

async fn create_geofence(
    State(state): State<AppState>,
    axum::extract::Json(payload): axum::extract::Json<CreateGeofence>,
) -> Json<OkResponse> {
    let storage = state.storage.lock().unwrap();
    let _ = storage.create_geofence(&payload.name, &payload.geojson);
    Json(OkResponse { ok: true })
}

async fn delete_geofence(
    Path(id): Path<i64>,
    State(state): State<AppState>,
) -> Json<OkResponse> {
    let storage = state.storage.lock().unwrap();
    let ok = storage.delete_geofence(id).unwrap_or(false);
    Json(OkResponse { ok })
}

async fn list_events(
    Query(params): Query<EventsQuery>,
    State(state): State<AppState>,
) -> Json<EventList> {
    let storage = state.storage.lock().unwrap();
    let events = storage
        .get_events(params.imei.as_deref(), params.limit)
        .unwrap_or_default();
    Json(EventList { events })
}

async fn map_page() -> Html<&'static str> {
    Html(include_str!("map.html"))
}

async fn tracks_page() -> Html<&'static str> {
    Html(include_str!("tracks.html"))
}

async fn reports_page() -> Html<&'static str> {
    Html(include_str!("reports.html"))
}

async fn replay_page() -> Html<&'static str> {
    Html(include_str!("replay.html"))
}

async fn geofences_page() -> Html<&'static str> {
    Html(include_str!("geofences.html"))
}

async fn events_page() -> Html<&'static str> {
    Html(include_str!("events.html"))
}

async fn users_page(Extension(user): Extension<CurrentUser>) -> Response {
    if !user.is_admin() {
        return Redirect::to("/map").into_response();
    }
    Html(include_str!("users.html")).into_response()
}

async fn settings_page(Extension(user): Extension<CurrentUser>) -> Response {
    if !user.is_admin() {
        return Redirect::to("/map").into_response();
    }
    Html(include_str!("settings.html")).into_response()
}
