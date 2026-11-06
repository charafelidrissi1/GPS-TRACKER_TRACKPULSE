//! Tokio TCP server for Teltonika tracker connections (Phase 2+5).
//!
//! Flow per connection:
//!   1. Read tracker IMEI (ASCII digits) from the stream.
//!   2. Reply with login ACK (0x01 = ok).
//!   3. Loop: read AVL packets, parse them, persist positions, send data ACK,
//!      broadcast position + events (overspeed, geofence enter/exit) to
//!      WebSocket dashboard clients.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use bytes::{Buf, BytesMut};
use tokio::sync::broadcast;
use crate::protocol::{build_data_ack, build_login_ack, parse_avl_packet, ProtocolError};
use crate::storage::SqliteStorage;
use crate::ws::ServerMessage;

/// Address the TCP listener binds to.
pub const LISTEN_ADDR: &str = "0.0.0.0:8082";

/// TCP listen address, overridable with `GPS_TRACKER_TCP_ADDR` (useful for tests).
pub fn listen_addr() -> String {
    std::env::var("GPS_TRACKER_TCP_ADDR").unwrap_or_else(|_| LISTEN_ADDR.to_string())
}

/// Speed threshold (km/h) above which an overspeed event is generated.
const OVERSPEED_KMH: u16 = 80;

/// Maximum bytes to buffer while hunting for the IMEI.
const IMEI_MAX_BYTES: usize = 64;

/// Tracks whether a device is currently inside each geofence.
/// Key: (imei, geofence_id), Value: inside?
type GeofenceState = Arc<Mutex<HashMap<(String, i64), bool>>>;

/// Run the TCP listener forever.
pub async fn run(
    storage: Arc<Mutex<SqliteStorage>>,
    tx: broadcast::Sender<ServerMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr = listen_addr();
    let listener = TcpListener::bind(&listen_addr).await?;
    println!("Teltonika TCP server listening on {}", listen_addr);

    let geofence_state: GeofenceState = Arc::new(Mutex::new(HashMap::new()));

    loop {
        let (socket, peer) = listener.accept().await?;
        let storage = Arc::clone(&storage);
        let tx = tx.clone();
        let gf_state = Arc::clone(&geofence_state);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(socket, storage, tx, gf_state).await {
                eprintln!("connection from {} error: {}", peer, e);
            }
        });
    }
}

/// Persist an event and broadcast it to WebSocket clients.
fn emit_event(
    storage: &Arc<Mutex<SqliteStorage>>,
    tx: &broadcast::Sender<ServerMessage>,
    imei: &str,
    etype: &str,
    message: &str,
    timestamp_ms: u64,
) {
    {
        let guard = storage.lock().unwrap();
        let _ = guard.create_event(imei, etype, message, timestamp_ms);
    }
    let evt = serde_json::json!({
        "type": "event",
        "data": {
            "imei": imei,
            "type": etype,
            "message": message,
            "timestamp_ms": timestamp_ms,
            "created_at": chrono::Utc::now().to_rfc3339()
        }
    });
    let _ = tx.send(ServerMessage::Event(evt.to_string()));
}

/// Check geofences for enter/exit transitions and emit events.
fn check_geofences(
    storage: &Arc<Mutex<SqliteStorage>>,
    tx: &broadcast::Sender<ServerMessage>,
    gf_state: &GeofenceState,
    imei: &str,
    lat: f64,
    lon: f64,
    timestamp_ms: u64,
) {
    // Load geofences from storage.
    let geofences = {
        let guard = storage.lock().unwrap();
        guard.get_geofences().unwrap_or_default()
    };

    for gf in geofences {
        let Some(id) = gf.id else { continue };
        let inside = crate::geo::point_in_geojson(lat, lon, &gf.geojson);

        let key = (imei.to_string(), id);
        let prev = {
            let mut states = gf_state.lock().unwrap();
            let prev = states.get(&key).copied();
            states.insert(key, inside);
            prev
        };

        // Only emit on a transition. First observation inside counts as "enter".
        let transitioned = match prev {
            Some(p) => p != inside,
            None => inside,
        };
        if transitioned {
            if inside {
                emit_event(
                    storage,
                    tx,
                    imei,
                    "geofence_enter",
                    &format!("Entered geofence '{}'", gf.name),
                    timestamp_ms,
                );
            } else if prev == Some(true) {
                emit_event(
                    storage,
                    tx,
                    imei,
                    "geofence_exit",
                    &format!("Exited geofence '{}'", gf.name),
                    timestamp_ms,
                );
            }
        }
    }
}

/// Handle a single tracker connection.
async fn handle_connection(
    mut socket: TcpStream,
    storage: Arc<Mutex<SqliteStorage>>,
    tx: broadcast::Sender<ServerMessage>,
    gf_state: GeofenceState,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = BytesMut::new();

    // ---- Phase 1: IMEI login ----
    let imei = read_imei(&mut socket, &mut buf).await?;
    socket.write_all(&build_login_ack(true)).await?;
    socket.flush().await?;
    println!("[{}] tracker logged in", imei);

    emit_event(
        &storage,
        &tx,
        &imei,
        "online",
        &format!("Device {} connected", imei),
        chrono::Utc::now().timestamp_millis() as u64,
    );

    // ---- Phase 2: AVL data stream ----
    loop {
        match parse_avl_packet(&buf, &imei) {
            Ok((proto_pos, consumed)) => {
                buf.advance(consumed);

                // Convert protocol::Position -> storage::Position,
                // extracting the telemetry IO elements we care about.
                let io = &proto_pos.io;
                let pos = crate::storage::Position {
                    imei: proto_pos.imei.clone(),
                    timestamp_ms: proto_pos.timestamp_ms,
                    latitude: proto_pos.latitude,
                    longitude: proto_pos.longitude,
                    altitude: proto_pos.altitude,
                    speed: proto_pos.speed,
                    angle: proto_pos.angle,
                    satellites: proto_pos.satellites,
                    odometer_m: io.get(&16).copied(),
                    fuel_level: io
                        .get(&201)
                        .or_else(|| io.get(&210))
                        .or_else(|| io.get(&203))
                        .or_else(|| io.get(&212))
                        .or_else(|| io.get(&214))
                        .map(|v| *v as f64),
                    device_battery_mv: io.get(&67).copied(),
                    vehicle_battery_mv: io.get(&66).copied(),
                    gsm_signal: io.get(&21).copied(),
                };

                // Persist position (updates device heartbeat) and read the
                // current overspeed threshold from platform settings.
                let overspeed_kmh = {
                    let guard = storage.lock().unwrap();
                    let _ = guard.insert_position(&pos);
                    guard
                        .get_settings()
                        .map(|settings| settings.overspeed_kmh)
                        .unwrap_or(OVERSPEED_KMH)
                };

                println!(
                    "[{}] position lat={:.6} lon={:.6} speed={}kmh sats={}",
                    imei, pos.latitude, pos.longitude, pos.speed, pos.satellites
                );

                // Broadcast position to all WebSocket dashboard clients
                let _ = tx.send(ServerMessage::Position(pos.clone()));

                // Overspeed detection
                if pos.speed > overspeed_kmh {
                    emit_event(
                        &storage,
                        &tx,
                        &imei,
                        "overspeed",
                        &format!(
                            "Overspeed: {} km/h exceeds limit {} km/h",
                            pos.speed, overspeed_kmh
                        ),
                        pos.timestamp_ms,
                    );
                }

                // Geofence enter/exit detection
                check_geofences(
                    &storage,
                    &tx,
                    &gf_state,
                    &imei,
                    pos.latitude,
                    pos.longitude,
                    pos.timestamp_ms,
                );

                // ACK: one record received
                socket.write_all(&build_data_ack(1)).await?;
                socket.flush().await?;
            }
            Err(ProtocolError::Incomplete) => {
                let n = socket.read_buf(&mut buf).await?;
                if n == 0 {
                    break;
                }
            }
            Err(e) => {
                eprintln!("[{}] parse error: {:?}; resyncing", imei, e);
                if !buf.is_empty() {
                    buf.advance(1);
                } else {
                    let n = socket.read_buf(&mut buf).await?;
                    if n == 0 {
                        break;
                    }
                }
            }
        }
    }

    // Device went offline
    emit_event(
        &storage,
        &tx,
        &imei,
        "offline",
        &format!("Device {} disconnected", imei),
        chrono::Utc::now().timestamp_millis() as u64,
    );

    println!("[{}] tracker disconnected", imei);
    Ok(())
}

/// Read the tracker IMEI from the stream.
async fn read_imei(
    socket: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<String, Box<dyn std::error::Error>> {
    loop {
        let n = socket.read_buf(buf).await?;
        if n == 0 {
            return Err("tracker disconnected during login".into());
        }

        let mut imei_len = 0;
        for &b in buf.iter() {
            if b.is_ascii_digit() {
                imei_len += 1;
            } else {
                break;
            }
        }

        let has_imei = imei_len >= 10 && imei_len <= 16;
        let has_delimiter = buf.iter().any(|b| *b == b'\n' || *b == b'\r' || *b == b'\0');

        if has_imei || (has_delimiter && imei_len > 0) {
            let imei = String::from_utf8_lossy(&buf[..imei_len]).trim().to_string();
            let end = if imei_len < buf.len() {
                let delim = buf[imei_len..]
                    .iter()
                    .position(|b| *b == b'\n' || *b == b'\r' || *b == b'\0')
                    .unwrap_or(0);
                imei_len + delim
            } else {
                imei_len
            };
            buf.advance(end);
            return Ok(imei);
        }

        if buf.len() > IMEI_MAX_BYTES {
            return Err("IMEI too long / malformed login".into());
        }
    }
}
