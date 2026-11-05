# TrackPulse — GPS Tracker Platform

A small, self-hosted GPS tracking platform written in Rust. It speaks the
Teltonika AVL protocol (Codec 8), stores positions in SQLite, and serves a live
fleet dashboard over HTTP and WebSocket. Maps use OpenStreetMap — free, no API
key required.

## Features

- **Teltonika TCP server** — IMEI login, Codec 8 AVL parsing, data ACKs, IO
  telemetry (odometer, fuel level, battery/vehicle voltage, GSM signal).
- **Live dashboard** — Leaflet + OpenStreetMap, smooth marker updates,
  online/offline indicators, follow mode, device detail panel, activity feed,
  and toast notifications.
- **GPS Data report** — Traccar-style telemetry table with timestamp, event
  badge, coordinates, speed category, heading, altitude, odometer, fuel,
  batteries, and signal quality.
- **Route replay** — animated playback with play/pause, point stepping,
  timeline scrubbing, 0.5×–64× speed, follow mode, and keyboard shortcuts.
- **Trip history** — draw a device's route for any time range.
- **Geofences** — draw polygons on the map and receive enter/exit events.
- **Events** — overspeed (> 80 km/h), geofence breaches, online/offline,
  persisted to SQLite and broadcast over WebSocket.

## Quick start

```bash
cargo build --release
./target/release/gps-tracker
```

Then open <http://localhost:8080/map>.

| Service | Address |
| --- | --- |
| HTTP API + dashboard | `0.0.0.0:8080` |
| WebSocket live stream | `ws://localhost:8080/ws` |
| Teltonika TCP listener | `0.0.0.0:8082` |

The SQLite database (`gps-tracker.db`) is created automatically on first run.
Point a Teltonika tracker (or the mock client) at TCP port `8082`.

## HTTP API

| Method | Path | Description |
| --- | --- | --- |
| GET | `/map` | Live fleet dashboard |
| GET | `/reports` | GPS Data telemetry table |
| GET | `/replay` | Route replay |
| GET | `/tracks` | Trip history |
| GET | `/geofences` | Geofence editor |
| GET | `/events` | Event log |
| GET | `/api/devices` | Devices with online status and latest position |
| GET | `/api/stats` | Aggregate platform statistics |
| GET | `/api/positions?imei=X&limit=N` | Latest positions |
| GET | `/api/tracks?imei=X&from=&to=&limit=N` | Track trace (epoch ms) |
| GET | `/api/geofences` | List geofences |
| POST | `/api/geofences` | Create geofence (`{name, geojson}`) |
| DELETE | `/api/geofences/{id}` | Delete geofence |
| GET | `/api/events?imei=X&limit=N` | Event log |
| GET | `/ws` | WebSocket position/event stream |

## Teltonika protocol

- Tracker sends its IMEI as ASCII; the server replies with `0x01` (accepted)
  or `0x00` (rejected).
- AVL packets use Codec 8 (`0x08`), with CRC-16/CCITT-FALSE.
- The parser handles the N1/N2/N4/N8 IO element groups.
- Each valid data packet is acknowledged with `0x01`.

Relevant AVL IO IDs:

| ID | Meaning | Unit |
| --- | --- | --- |
| 16 | Total odometer | m |
| 21 | GSM signal | 1–5 |
| 66 | External / vehicle voltage | mV |
| 67 | Internal battery voltage | mV |
| 201/210 | Fuel level | device units |

## Development

```bash
cargo test          # protocol, storage, and geofence tests
cargo build --release
```

Source layout:

```
src/
  main.rs          wiring and startup
  protocol/        Teltonika Codec 8 parser/encoder + CRC16
  server/          Tokio TCP tracker server
  storage/         SQLite storage and queries
  api/             Axum routes + embedded HTML pages
  ws.rs            WebSocket broadcast
  geo.rs           point-in-polygon geofence checks
```

## Notes

- The database and build output are intentionally ignored by git.
- Ports and thresholds (e.g. the 80 km/h overspeed limit) are constants in the
  source and can be changed before building.
