mod protocol;
mod server;
mod storage;
mod api;
mod ws;
mod geo;

use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use crate::server::LISTEN_ADDR;
use crate::storage::{SqliteStorage, DB_PATH};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("==============================================");
    println!("  GPS Tracker Fleet Platform (Rust)");
    println!("==============================================");
    println!("TCP tracker  : {}", LISTEN_ADDR);
    println!("HTTP API     : 0.0.0.0:8080");
    println!("WebSocket    : 0.0.0.0:8080/ws");
    println!("Dashboard    : http://localhost:8080/map");
    println!("Database     : {}", DB_PATH);
    println!("Map tiles    : OpenStreetMap (free, no API key)");
    println!();

    // Shared SQLite storage (accessed by TCP server + HTTP API)
    let storage: Arc<Mutex<SqliteStorage>> =
        Arc::new(Mutex::new(SqliteStorage::open(DB_PATH)?));
    println!("SQLite storage opened");

    // Broadcast channel for live position/event streaming to WebSocket clients
    let (tx, _rx) = broadcast::channel::<ws::ServerMessage>(128);
    let api_state = ws::AppState {
        storage: Arc::clone(&storage),
        tx: tx.clone(),
    };

    // Spawn the HTTP API server (port 8080) concurrently
    let api_handle = tokio::spawn(async move {
        if let Err(e) = api::start(api_state, "0.0.0.0:8080").await {
            eprintln!("HTTP API error: {}", e);
        }
    });

    // Pass the broadcast sender to the TCP server so it can notify clients
    server::run(storage, tx).await?;

    // If TCP server returns, wait for API task too
    let _ = api_handle.await;
    Ok(())
}
