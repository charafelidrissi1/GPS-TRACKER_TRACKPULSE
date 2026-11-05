//! WebSocket module for live position + event streaming (Phase 5).
//!
//! The TCP tracker server broadcasts every received position and generated
//! event on a `tokio::sync::broadcast` channel. Dashboard clients connect
//! via WebSocket to `/ws`, subscribe to that channel, and receive real-time
//! JSON messages.

use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::Response,
};
use futures_util::stream::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::storage::{Position, SqliteStorage};

/// Message broadcast from the server to all dashboard WebSocket clients.
#[derive(Clone, Debug)]
pub enum ServerMessage {
    Position(Position),
    Event(String), // pre-serialized JSON string
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<std::sync::Mutex<SqliteStorage>>,
    pub tx: broadcast::Sender<ServerMessage>,
}

/// Handle an incoming WebSocket upgrade request.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    let rx = state.tx.subscribe();
    ws.on_upgrade(move |socket| handle_socket(socket, rx))
}

/// Per-connection WebSocket loop: subscribe to broadcast, forward to client.
async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<ServerMessage>) {
    // Send an initial hello so the client knows the connection is open.
    let _ = socket.send(Message::Text(json!({
        "type": "hello",
        "message": "connected to GPS tracker"
    }).to_string().into())).await;

    loop {
        tokio::select! {
            // Incoming message from broadcast channel -> send to WebSocket client
            msg = rx.recv() => {
                match msg {
                    Ok(ServerMessage::Position(pos)) => {
                        let payload = json!({
                            "type": "position",
                            "data": pos
                        });
                        if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(ServerMessage::Event(evt_json)) => {
                        let payload: Value = serde_json::from_str(&evt_json).unwrap_or(json!({"type":"event","message":evt_json}));
                        if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("[ws] lagged behind by {} messages", n);
                        // Resubscribe; we may have missed messages but keep going.
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Incoming WebSocket message from client -> we ignore (client is read-only for now)
            Some(Ok(msg)) = socket.recv() => {
                match msg {
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Binary(_) => {}
                }
            }
            else => break,
        }
    }
}
