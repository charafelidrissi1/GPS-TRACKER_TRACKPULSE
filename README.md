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
- **Events** — overspeed, geofence breaches, online/offline, persisted to
  SQLite and broadcast over WebSocket.
- **Accounts and roles** — real login with Argon2-hashed passwords, HTTP-only
  sessions, first-run admin setup, and `admin` / `viewer` roles. Viewers get
  read-only access to dashboards and reports.
- **Settings** — admin-configurable platform name, overspeed threshold, and
  online timeout, applied to live tracker packets without a restart.

## Quick start

```bash
cargo build --release
./target/release/gps-tracker
```

Then open <http://localhost:8080/setup> to create the first administrator
account. There are no default credentials. After setup, sign in at
<http://localhost:8080/login>.

| Service | Address |
| --- | --- |
| HTTP API + dashboard | `0.0.0.0:8080` |
| WebSocket live stream | `ws://localhost:8080/ws` |
| Teltonika TCP listener | `0.0.0.0:8082` |

The SQLite database (`gps-tracker.db`) is created automatically on first run.
Point a Teltonika tracker (or the mock client) at TCP port `8082`.

### Configuration

Environment variables override the defaults (useful for tests and multiple
instances):

| Variable | Default | Description |
| --- | --- | --- |
| `GPS_TRACKER_DB` | `gps-tracker.db` | SQLite database path |
| `GPS_TRACKER_HTTP_ADDR` | `0.0.0.0:8080` | HTTP + WebSocket bind address |
| `GPS_TRACKER_TCP_ADDR` | `0.0.0.0:8082` | Teltonika TCP bind address |

## Authentication

- The first visit to any page redirects to `/setup` until an admin exists.
- Passwords are hashed with Argon2; sessions are stored in SQLite and sent as
  an HttpOnly, SameSite=Lax `tp_session` cookie (7-day expiry).
- `admin` accounts manage users and settings; `viewer` accounts have read-only
  access to dashboards, reports, replay, trips, geofences, and events.
- All pages, API routes, and the WebSocket require a valid session (except the
  login/setup flow and public branding).
- Users can change their own password from the user menu. Admins can create,
  disable, delete, re-role, and reset passwords for other users. The last
  active admin cannot be disabled, demoted, or deleted.

## HTTP API

| Method | Path | Description |
| --- | --- | --- |
| GET | `/setup` | First-run admin setup page |
| GET | `/login` | Sign-in page |
| POST | `/api/auth/setup` | Create the first admin (`{username, password}`) |
| POST | `/api/auth/login` | Sign in (`{username, password}`) |
| POST | `/api/auth/logout` | Sign out |
| GET | `/api/auth/me` | Current user |
| GET | `/api/auth/status` | Setup/authentication status |
| POST | `/api/auth/password` | Change own password (`{current_password, new_password}`) |
| GET | `/map` | Live fleet dashboard |
| GET | `/reports` | GPS Data telemetry table |
| GET | `/replay` | Route replay |
| GET | `/tracks` | Trip history |
| GET | `/geofences` | Geofence editor |
| GET | `/events` | Event log |
| GET | `/users` | User management (admin only) |
| GET | `/settings` | Platform settings (admin only) |
| GET | `/api/devices` | Devices with online status and latest position |
| GET | `/api/stats` | Aggregate platform statistics |
| GET | `/api/positions?imei=X&limit=N` | Latest positions |
| GET | `/api/tracks?imei=X&from=&to=&limit=N` | Track trace (epoch ms) |
| GET | `/api/geofences` | List geofences |
| POST | `/api/geofences` | Create geofence (`{name, geojson}`) |
| DELETE | `/api/geofences/{id}` | Delete geofence |
| GET | `/api/events?imei=X&limit=N` | Event log |
| GET | `/api/users` | List users (admin only) |
| POST | `/api/users` | Create user (`{username, password, role}`) |
| PATCH | `/api/users/{id}` | Update role/active/password (admin only) |
| DELETE | `/api/users/{id}` | Delete user (admin only) |
| GET | `/api/settings` | Read settings (admin only) |
| POST | `/api/settings` | Update settings (admin only) |
| GET | `/api/settings/public` | Public branding (`platform_name`) |
| GET | `/ws` | Authenticated WebSocket position/event stream |

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
cargo test          # protocol, storage, geofence, users/sessions/settings tests
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
- Overspeed threshold, online timeout, and platform name are managed in the
  Settings page and stored in SQLite; the defaults are 80 km/h, 5 minutes, and
  `TrackPulse`.
- Bind addresses and the database path can be overridden with the environment
  variables listed above.
