//! SQLite storage for devices, positions, geofences, and events (Phase 3+).
//! File-based, zero-config persistence via rusqlite (bundled SQLite).

use chrono::{NaiveDateTime, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::Path;

/// Path to the SQLite database file.
pub const DB_PATH: &str = "gps-tracker.db";

/// A GPS position record.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Position {
    pub imei: String,
    pub timestamp_ms: u64,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: i16,
    pub speed: u16,
    pub angle: u16,
    pub satellites: u8,
    /// AVL ID 16 — total odometer, meters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub odometer_m: Option<i64>,
    /// AVL ID 201/210/203 — fuel level (device units, usually liters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuel_level: Option<f64>,
    /// AVL ID 67 — internal battery voltage, millivolts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_battery_mv: Option<i64>,
    /// AVL ID 66 — external / vehicle voltage, millivolts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vehicle_battery_mv: Option<i64>,
    /// AVL ID 21 — GSM signal level (1–5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gsm_signal: Option<i64>,
}

/// A device with online/offline status.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Device {
    pub imei: String,
    pub first_seen: String,
    pub last_seen: String,
    pub online: bool,
    /// Most recent position, if the device has ever reported.
    pub latest: Option<Position>,
}

/// Aggregate platform statistics for the dashboard.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Stats {
    pub devices_total: i64,
    pub devices_online: i64,
    pub positions_total: i64,
    pub events_total: i64,
    pub positions_today: i64,
    pub events_today: i64,
}

/// A geofence zone.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Geofence {
    pub id: Option<i64>,
    pub name: String,
    pub geojson: serde_json::Value,
    pub created_at: String,
}

/// A tracking event / alert.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Event {
    pub id: Option<i64>,
    pub imei: String,
    pub r#type: String,
    pub message: String,
    pub timestamp_ms: u64,
    pub created_at: String,
}

/// Add a column to an existing table if it is missing (simple migration).
fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> rusqlite::Result<()> {
    let exists = {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
        let mut rows = stmt.query([])?;
        let mut found = false;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                found = true;
                break;
            }
        }
        found
    };

    if !exists {
        conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, decl),
            [],
        )?;
    }
    Ok(())
}

/// SQLite-backed storage.
pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    /// Open (or create) the database file and run the schema migration.
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;

        conn.pragma_update(None, "journal_mode", &"WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    fn migrate(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS devices (
                imei TEXT PRIMARY KEY,
                first_seen TEXT NOT NULL DEFAULT (datetime('now')),
                last_seen TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS positions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                imei TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                latitude REAL NOT NULL,
                longitude REAL NOT NULL,
                altitude INTEGER NOT NULL,
                speed INTEGER NOT NULL,
                angle INTEGER NOT NULL,
                satellites INTEGER NOT NULL,
                odometer_m INTEGER,
                fuel_level REAL,
                device_battery_mv INTEGER,
                vehicle_battery_mv INTEGER,
                gsm_signal INTEGER,
                received_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (imei) REFERENCES devices(imei)
            );

            CREATE INDEX IF NOT EXISTS idx_positions_imei_time
                ON positions (imei, timestamp_ms DESC);

            CREATE TABLE IF NOT EXISTS geofences (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                geojson TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                imei TEXT NOT NULL,
                type TEXT NOT NULL,
                message TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_events_imei_time
                ON events (imei, timestamp_ms DESC);
            ",
        )?;

        // Migrate databases created before the telemetry columns existed.
        ensure_column(conn, "positions", "odometer_m", "INTEGER")?;
        ensure_column(conn, "positions", "fuel_level", "REAL")?;
        ensure_column(conn, "positions", "device_battery_mv", "INTEGER")?;
        ensure_column(conn, "positions", "vehicle_battery_mv", "INTEGER")?;
        ensure_column(conn, "positions", "gsm_signal", "INTEGER")?;

        Ok(())
    }

    /// Insert a parsed position, upserting the device record (heartbeat).
    pub fn insert_position(&self, pos: &Position) -> rusqlite::Result<()> {
        self.conn.execute(
            "
            INSERT INTO devices (imei, first_seen, last_seen)
            VALUES (?1, datetime('now'), datetime('now'))
            ON CONFLICT(imei) DO UPDATE SET last_seen = datetime('now')
            ",
            params![pos.imei],
        )?;

        self.conn.execute(
            "
            INSERT INTO positions
                (imei, timestamp_ms, latitude, longitude, altitude, speed, angle, satellites,
                 odometer_m, fuel_level, device_battery_mv, vehicle_battery_mv, gsm_signal)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ",
            params![
                pos.imei,
                pos.timestamp_ms,
                pos.latitude,
                pos.longitude,
                pos.altitude,
                pos.speed,
                pos.angle,
                pos.satellites,
                pos.odometer_m,
                pos.fuel_level,
                pos.device_battery_mv,
                pos.vehicle_battery_mv,
                pos.gsm_signal,
            ],
        )?;

        Ok(())
    }

    /// Return all devices with online/offline status and their latest position.
    /// A device is "online" if it sent a position within the last ONLINE_WINDOW_SECONDS.
    pub fn get_devices(&self) -> rusqlite::Result<Vec<Device>> {
        const ONLINE_WINDOW_SECONDS: i64 = 300; // 5 minutes
        let now = Utc::now().timestamp();

        let mut stmt = self.conn.prepare(
            "
            SELECT d.imei, d.first_seen, d.last_seen,
                   p.timestamp_ms, p.latitude, p.longitude,
                   p.altitude, p.speed, p.angle, p.satellites,
                   p.odometer_m, p.fuel_level, p.device_battery_mv,
                   p.vehicle_battery_mv, p.gsm_signal
            FROM devices d
            LEFT JOIN positions p ON p.id = (
                SELECT id FROM positions
                WHERE imei = d.imei
                ORDER BY timestamp_ms DESC, id DESC
                LIMIT 1
            )
            ORDER BY d.last_seen DESC
            ",
        )?;

        let rows = stmt.query_map([], |row| {
            let imei: String = row.get(0)?;
            let first_seen: String = row.get(1)?;
            let last_seen: String = row.get(2)?;

            let online = NaiveDateTime::parse_from_str(&last_seen, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|ndt| now - ndt.and_utc().timestamp() <= ONLINE_WINDOW_SECONDS)
                .unwrap_or(false);

            let latest = match row.get::<_, Option<i64>>(3)? {
                Some(ts) => Some(Position {
                    imei: imei.clone(),
                    timestamp_ms: ts as u64,
                    latitude: row.get(4)?,
                    longitude: row.get(5)?,
                    altitude: row.get(6)?,
                    speed: row.get(7)?,
                    angle: row.get(8)?,
                    satellites: row.get(9)?,
                    odometer_m: row.get(10)?,
                    fuel_level: row.get(11)?,
                    device_battery_mv: row.get(12)?,
                    vehicle_battery_mv: row.get(13)?,
                    gsm_signal: row.get(14)?,
                }),
                None => None,
            };

            Ok(Device {
                imei,
                first_seen,
                last_seen,
                online,
                latest,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Aggregate platform statistics for the dashboard.
    pub fn get_stats(&self) -> rusqlite::Result<Stats> {
        let devices_total = self
            .conn
            .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))?;
        let devices_online = self.conn.query_row(
            "SELECT COUNT(*) FROM devices
             WHERE (strftime('%s','now') - strftime('%s', last_seen)) <= 300",
            [],
            |r| r.get(0),
        )?;
        let positions_total = self
            .conn
            .query_row("SELECT COUNT(*) FROM positions", [], |r| r.get(0))?;
        let events_total = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        let positions_today = self.conn.query_row(
            "SELECT COUNT(*) FROM positions WHERE date(received_at) = date('now')",
            [],
            |r| r.get(0),
        )?;
        let events_today = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE date(created_at) = date('now')",
            [],
            |r| r.get(0),
        )?;

        Ok(Stats {
            devices_total,
            devices_online,
            positions_total,
            events_total,
            positions_today,
            events_today,
        })
    }

    /// Return positions for a given IMEI (newest first), limited to `limit`.
    pub fn get_positions(&self, imei: &str, limit: usize) -> rusqlite::Result<Vec<Position>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT timestamp_ms, latitude, longitude, altitude, speed, angle, satellites,
                   odometer_m, fuel_level, device_battery_mv, vehicle_battery_mv, gsm_signal
            FROM positions
            WHERE imei = ?1
            ORDER BY timestamp_ms DESC
            LIMIT ?2
            ",
        )?;

        let rows = stmt.query_map(params![imei, limit as i64], |row| {
            Ok(Position {
                imei: imei.to_string(),
                timestamp_ms: row.get(0)?,
                latitude: row.get(1)?,
                longitude: row.get(2)?,
                altitude: row.get(3)?,
                speed: row.get(4)?,
                angle: row.get(5)?,
                satellites: row.get(6)?,
                odometer_m: row.get(7)?,
                fuel_level: row.get(8)?,
                device_battery_mv: row.get(9)?,
                vehicle_battery_mv: row.get(10)?,
                gsm_signal: row.get(11)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Return positions for a given IMEI within a time range (for track/trace).
    /// `from_ms` and `to_ms` are epoch milliseconds.
    pub fn get_tracks(
        &self,
        imei: &str,
        from_ms: Option<u64>,
        to_ms: Option<u64>,
        limit: usize,
    ) -> rusqlite::Result<Vec<Position>> {
        let mut query = String::from(
            "
            SELECT timestamp_ms, latitude, longitude, altitude, speed, angle, satellites,
                   odometer_m, fuel_level, device_battery_mv, vehicle_battery_mv, gsm_signal
            FROM positions
            WHERE imei = ?1
            ",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(imei.to_string())];

        if let Some(from) = from_ms {
            query.push_str(" AND timestamp_ms >= ?");
            params.push(Box::new(from as i64));
        }
        if let Some(to) = to_ms {
            query.push_str(" AND timestamp_ms <= ?");
            params.push(Box::new(to as i64));
        }

        // Fetch the newest `limit` records, then return them ascending so
        // callers can draw a track in chronological order.
        query.push_str(" ORDER BY timestamp_ms DESC LIMIT ?");
        params.push(Box::new(limit as i64));

        let mut stmt = self.conn.prepare(&query)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(&param_refs[..], |row| {
            Ok(Position {
                imei: imei.to_string(),
                timestamp_ms: row.get(0)?,
                latitude: row.get(1)?,
                longitude: row.get(2)?,
                altitude: row.get(3)?,
                speed: row.get(4)?,
                angle: row.get(5)?,
                satellites: row.get(6)?,
                odometer_m: row.get(7)?,
                fuel_level: row.get(8)?,
                device_battery_mv: row.get(9)?,
                vehicle_battery_mv: row.get(10)?,
                gsm_signal: row.get(11)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    /// Return the list of known device IMEIs.
    pub fn list_devices(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT imei FROM devices ORDER BY last_seen DESC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn len(&self) -> rusqlite::Result<usize> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM positions")?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn is_empty(&self) -> rusqlite::Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Get the most recent position for a device.
    pub fn latest_position(&self, imei: &str) -> rusqlite::Result<Option<Position>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT timestamp_ms, latitude, longitude, altitude, speed, angle, satellites,
                   odometer_m, fuel_level, device_battery_mv, vehicle_battery_mv, gsm_signal
            FROM positions
            WHERE imei = ?1
            ORDER BY timestamp_ms DESC
            LIMIT 1
            ",
        )?;
        let row = stmt
            .query_row(params![imei], |row| {
                Ok(Position {
                    imei: imei.to_string(),
                    timestamp_ms: row.get(0)?,
                    latitude: row.get(1)?,
                    longitude: row.get(2)?,
                    altitude: row.get(3)?,
                    speed: row.get(4)?,
                    angle: row.get(5)?,
                    satellites: row.get(6)?,
                    odometer_m: row.get(7)?,
                    fuel_level: row.get(8)?,
                    device_battery_mv: row.get(9)?,
                    vehicle_battery_mv: row.get(10)?,
                    gsm_signal: row.get(11)?,
                })
            })
            .optional()?;
        Ok(row)
    }

    // ------------------------------------------------------------------
    // Geofences
    // ------------------------------------------------------------------

    pub fn create_geofence(
        &self,
        name: &str,
        geojson: &serde_json::Value,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO geofences (name, geojson) VALUES (?1, ?2)",
            params![name, geojson.to_string()],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(id)
    }

    pub fn get_geofences(&self) -> rusqlite::Result<Vec<Geofence>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, geojson, created_at FROM geofences ORDER BY id DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Geofence {
                id: row.get(0)?,
                name: row.get(1)?,
                geojson: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or(serde_json::json!({})),
                created_at: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn delete_geofence(&self, id: i64) -> rusqlite::Result<bool> {
        let changes = self
            .conn
            .execute("DELETE FROM geofences WHERE id = ?1", params![id])?;
        Ok(changes > 0)
    }

    // ------------------------------------------------------------------
    // Events
    // ------------------------------------------------------------------

    pub fn create_event(
        &self,
        imei: &str,
        etype: &str,
        message: &str,
        timestamp_ms: u64,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO events (imei, type, message, timestamp_ms) VALUES (?1, ?2, ?3, ?4)",
            params![imei, etype, message, timestamp_ms as i64],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(id)
    }

    pub fn get_events(
        &self,
        imei: Option<&str>,
        limit: usize,
    ) -> rusqlite::Result<Vec<Event>> {
        let (query, params_vec): (String, Vec<Box<dyn rusqlite::ToSql>>) =
            if let Some(imei) = imei {
                (
                    "SELECT id, imei, type, message, timestamp_ms, created_at FROM events WHERE imei = ?1 ORDER BY timestamp_ms DESC LIMIT ?2".to_string(),
                    vec![Box::new(imei.to_string()), Box::new(limit as i64)],
                )
            } else {
                (
                    "SELECT id, imei, type, message, timestamp_ms, created_at FROM events ORDER BY timestamp_ms DESC LIMIT ?1".to_string(),
                    vec![Box::new(limit as i64)],
                )
            };

        let mut stmt = self.conn.prepare(&query)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(&param_refs[..], |row| {
            Ok(Event {
                id: row.get(0)?,
                imei: row.get(1)?,
                r#type: row.get(2)?,
                message: row.get(3)?,
                timestamp_ms: row.get::<_, i64>(4)? as u64,
                created_at: row.get(5)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

impl Drop for SqliteStorage {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_position() -> Position {
        Position {
            imei: "356913032041875".to_string(),
            timestamp_ms: 1_700_000_000_000,
            latitude: 52.5200,
            longitude: 13.4050,
            altitude: 45,
            speed: 60,
            angle: 90,
            satellites: 8,
            odometer_m: None,
            fuel_level: None,
            device_battery_mv: None,
            vehicle_battery_mv: None,
            gsm_signal: None,
        }
    }

    #[test]
    fn insert_and_retrieve() {
        let storage = SqliteStorage::open(":memory:").unwrap();
        let mut pos = sample_position();
        pos.odometer_m = Some(115_488_039);
        pos.fuel_level = Some(30.0);
        pos.device_battery_mv = Some(4_000);
        pos.vehicle_battery_mv = Some(14_250);
        pos.gsm_signal = Some(4);
        storage.insert_position(&pos).unwrap();

        let positions = storage.get_positions("356913032041875", 10).unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].latitude, 52.5200);
        assert_eq!(positions[0].longitude, 13.4050);
        assert_eq!(positions[0].odometer_m, Some(115_488_039));
        assert_eq!(positions[0].fuel_level, Some(30.0));
        assert_eq!(positions[0].device_battery_mv, Some(4_000));
        assert_eq!(positions[0].vehicle_battery_mv, Some(14_250));
        assert_eq!(positions[0].gsm_signal, Some(4));

        let devices = storage.get_devices().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].imei, "356913032041875");
    }

    #[test]
    fn geofence_crud() {
        let storage = SqliteStorage::open(":memory:").unwrap();
        let geojson = serde_json::json!({
            "type": "Feature",
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[13.4, 52.5], [13.41, 52.5], [13.41, 52.51], [13.4, 52.51], [13.4, 52.5]]]
            }
        });
        let id = storage.create_geofence("Zone A", &geojson).unwrap();
        let geofences = storage.get_geofences().unwrap();
        assert_eq!(geofences.len(), 1);
        assert_eq!(geofences[0].id, Some(id));
        assert_eq!(geofences[0].name, "Zone A");

        assert!(storage.delete_geofence(id).unwrap());
        assert_eq!(storage.get_geofences().unwrap().len(), 0);
    }

    #[test]
    fn events_crud() {
        let storage = SqliteStorage::open(":memory:").unwrap();
        storage
            .create_event("356913032041875", "overspeed", "Speed 95 km/h exceeds limit 80", 1_700_000_000_000)
            .unwrap();
        storage
            .create_event("356913032041875", "online", "Device came online", 1_700_000_001_000)
            .unwrap();

        let events = storage.get_events(Some("356913032041875"), 10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].r#type, "online");
        assert_eq!(events[1].r#type, "overspeed");
    }
}
