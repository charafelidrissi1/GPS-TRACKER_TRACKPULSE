mod protocol;
mod server;
mod storage;
mod api;
mod ws;
mod geo;

use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use crate::server::listen_addr;
use crate::storage::{db_path, SqliteStorage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database = db_path();
    let http_addr =
        std::env::var("GPS_TRACKER_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let tcp_addr = listen_addr();
    let dashboard_addr = http_addr.replace("0.0.0.0", "localhost");

    println!("==============================================");
    println!("  GPS Tracker Fleet Platform (Rust)");
    println!("==============================================");
    println!("TCP tracker  : {}", tcp_addr);
    println!("HTTP API     : {}", http_addr);
    println!("WebSocket    : ws://{}/ws", dashboard_addr);
    println!("Dashboard    : http://{}/map", dashboard_addr);
    println!("First run    : open /setup to create the admin account");
    println!("Database     : {}", database);
    println!("Map tiles    : OpenStreetMap (free, no API key)");
    println!();

    // Shared SQLite storage (accessed by TCP server + HTTP API)
    let storage: Arc<Mutex<SqliteStorage>> =
        Arc::new(Mutex::new(SqliteStorage::open(&database)?));
    println!("SQLite storage opened");

    // Broadcast channel for live position/event streaming to WebSocket clients
    let (tx, _rx) = broadcast::channel::<ws::ServerMessage>(128);
    let api_state = ws::AppState {
        storage: Arc::clone(&storage),
        tx: tx.clone(),
    };

    // Spawn the HTTP API server (port 8080) concurrently
    let api_handle = tokio::spawn(async move {
        if let Err(e) = api::start(api_state, &http_addr).await {
            eprintln!("HTTP API error: {}", e);
        }
    });

    // Pass the broadcast sender to the TCP server so it can notify clients
    server::run(storage, tx).await?;

    // If TCP server returns, wait for API task too
    let _ = api_handle.await;
    Ok(())
}
