//! Teltonika AVL protocol implementation.
//!
//! This module implements a minimal subset of the Teltonika AVL TCP protocol
//! used by Teltonika GPS trackers (FMB, FMX, etc.).
//!
//! Packet structure (TCP stream):
//!   [Preamble: 1 byte 0x00]
//!   [Data Length: 4 bytes big-endian u32]  (covers everything after this field up to & including CRC)
//!   [Codec ID: 1 byte]                     (0x08 = Codec 8 extended)
//!   [Number of Records: 1 byte]
//!   [Records: variable]
//!   [CRC16: 2 bytes]                       (CRC-16/CCITT-FALSE, poly 0x1021, init 0xFFFF)
//!
//! Login sequence:
//!   Tracker connects and sends its IMEI as ASCII (e.g. "356913032041875").
//!   Server replies with 1 byte: 0x01 = OK, 0x00 = reject.

use bytes::BufMut;
use std::collections::BTreeMap;

/// A parsed GPS position coming from a tracker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Position {
    pub imei: String,
    pub timestamp_ms: u64,      // milliseconds since Unix epoch
    pub latitude: f64,          // degrees
    pub longitude: f64,         // degrees
    pub altitude: i16,          // meters
    pub speed: u16,             // km/h
    pub angle: u16,             // degrees
    pub satellites: u8,
    /// Raw Teltonika AVL IO elements (AVL ID -> value), e.g. 16=odometer,
    /// 21=GSM signal, 66=external voltage, 67=battery voltage.
    #[serde(skip)]
    pub io: BTreeMap<u8, i64>,
}

/// Login acknowledgment byte sent by the server.
pub const LOGIN_ACK_OK: u8 = 0x01;
pub const LOGIN_ACK_FAIL: u8 = 0x00;

/// Codec IDs.
pub const CODEC_8: u8 = 0x08;

/// Teltonika protocol preamble byte.
pub const PREAMBLE: u8 = 0x00;

/// Errors returned while parsing AVL data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Not enough bytes in the buffer to complete a field.
    Incomplete,
    /// Bad preamble byte.
    BadPreamble,
    /// Unsupported codec id.
    UnsupportedCodec(u8),
    /// CRC mismatch between computed and stored value.
    CrcMismatch { expected: u16, found: u16 },
    /// Malformed record (bad length, truncated, etc.).
    MalformedRecord,
}

/// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflection).
pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Encode a Codec-8 AVL packet containing one GPS record.
/// Used to build synthetic test packets for a mock tracker.
pub fn encode_codec8_packet(pos: &Position) -> Vec<u8> {
    let mut payload = bytes::BytesMut::new();

    // Codec ID
    payload.put_u8(CODEC_8);
    // Number of records
    payload.put_u8(1);

    // ---- Record ----
    // Timestamp (8 bytes, ms since epoch)
    payload.put_u64(pos.timestamp_ms);
    // Priority (1 byte)
    payload.put_u8(0);
    // GPS element length (1 byte) = bytes following this length byte
    let gps_len: u8 = 15; // lon(4)+lat(4)+alt(2)+angle(2)+sats(1)+speed(2)
    payload.put_u8(gps_len);
    // Longitude (i32, 1e-7 deg)
    payload.put_i32((pos.longitude * 1e7) as i32);
    // Latitude (i32, 1e-7 deg)
    payload.put_i32((pos.latitude * 1e7) as i32);
    // Altitude (i16, meters)
    payload.put_i16(pos.altitude);
    // Angle (u16, degrees)
    payload.put_u16(pos.angle);
    // Satellites (u8)
    payload.put_u8(pos.satellites);
    // Speed (u16, km/h)
    payload.put_u16(pos.speed);

    // IO element (Codec 8): event IO id, total count, then four size groups.
    // All values are encoded as 4-byte (N4) elements for simplicity.
    payload.put_u8(0); // event IO id (0 = periodic record)
    payload.put_u8(pos.io.len() as u8); // total IO count
    payload.put_u8(0); // N1: 1-byte values
    payload.put_u8(0); // N2: 2-byte values
    payload.put_u8(pos.io.len() as u8); // N4: 4-byte values
    for (id, value) in &pos.io {
        payload.put_u8(*id);
        payload.put_u32(*value as u32);
    }
    payload.put_u8(0); // N8: 8-byte values

    let crc = crc16_ccitt(&payload);
    payload.put_u16(crc);

    // ---- Wrap with preamble + data length ----
    let mut packet = bytes::BytesMut::new();
    packet.put_u8(PREAMBLE);
    // data length = payload bytes (codec + records + crc)
    packet.put_u32(payload.len() as u32);
    packet.extend_from_slice(&payload);

    packet.to_vec()
}

/// Build the login acknowledgment byte.
pub fn build_login_ack(ok: bool) -> Vec<u8> {
    vec![if ok { LOGIN_ACK_OK } else { LOGIN_ACK_FAIL }]
}

/// Build the data acknowledgment sent after receiving AVL records.
/// Returns the number of records as a single byte (Teltonika expects this).
pub fn build_data_ack(num_records: u8) -> Vec<u8> {
    vec![num_records]
}

/// Parse a single AVL packet from the front of `buf`.
///
/// Returns the parsed `Position` and the number of bytes consumed from `buf`.
/// If the buffer is incomplete, returns `Err(ProtocolError::Incomplete)`.
pub fn parse_avl_packet(buf: &[u8], imei: &str) -> Result<(Position, usize), ProtocolError> {
    // Need at least preamble + data length
    if buf.len() < 5 {
        return Err(ProtocolError::Incomplete);
    }

    if buf[0] != PREAMBLE {
        return Err(ProtocolError::BadPreamble);
    }

    let data_len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    // Full packet = preamble(1) + length(4) + data_len
    let total = 1 + 4 + data_len;
    if buf.len() < total {
        return Err(ProtocolError::Incomplete);
    }

    let payload = &buf[5..5 + data_len];
    // payload = [codec(1)][num_records(1)][records...][crc(2)]
    if payload.len() < 4 {
        return Err(ProtocolError::MalformedRecord);
    }

    let codec = payload[0];
    if codec != CODEC_8 {
        return Err(ProtocolError::UnsupportedCodec(codec));
    }

    let num_records = payload[1] as usize;

    // CRC is the last 2 bytes of payload; CRC is computed over everything before it.
    let crc_region = &payload[..payload.len() - 2];
    let stored_crc = u16::from_be_bytes([payload[payload.len() - 2], payload[payload.len() - 1]]);
    let computed_crc = crc16_ccitt(crc_region);
    if computed_crc != stored_crc {
        return Err(ProtocolError::CrcMismatch {
            expected: computed_crc,
            found: stored_crc,
        });
    }

    // Parse records (Codec 8)
    let mut offset = 2; // skip codec + num_records
    let mut position: Option<Position> = None;

    for _ in 0..num_records {
        // Need timestamp(8) + priority(1) at minimum
        if offset + 9 > payload.len() - 2 {
            return Err(ProtocolError::MalformedRecord);
        }
        let timestamp_ms = u64::from_be_bytes([
            payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3],
            payload[offset + 4], payload[offset + 5], payload[offset + 6], payload[offset + 7],
        ]);
        let _priority = payload[offset + 8];
        offset += 9;

        // GPS element
        if offset + 1 > payload.len() - 2 {
            return Err(ProtocolError::MalformedRecord);
        }
        let gps_len = payload[offset] as usize;
        offset += 1;
        if offset + gps_len > payload.len() - 2 {
            return Err(ProtocolError::MalformedRecord);
        }
        let gps = &payload[offset..offset + gps_len];
        offset += gps_len;

        // Parse GPS element (fixed 15-byte layout)
        if gps.len() < 15 {
            return Err(ProtocolError::MalformedRecord);
        }
        let longitude = i32::from_be_bytes([gps[0], gps[1], gps[2], gps[3]]) as f64 / 1e7;
        let latitude = i32::from_be_bytes([gps[4], gps[5], gps[6], gps[7]]) as f64 / 1e7;
        let altitude = i16::from_be_bytes([gps[8], gps[9]]);
        let angle = u16::from_be_bytes([gps[10], gps[11]]);
        let satellites = gps[12];
        let speed = u16::from_be_bytes([gps[13], gps[14]]);

        // IO element (Codec 8):
        //   event IO id (1) + total count (1) + N1/N2/N4/N8 groups.
        if offset + 2 > payload.len() - 2 {
            return Err(ProtocolError::MalformedRecord);
        }
        let _event_io_id = payload[offset];
        offset += 1;
        let _total_io = payload[offset];
        offset += 1;

        let mut io: BTreeMap<u8, i64> = BTreeMap::new();
        for size in [1usize, 2, 4, 8] {
            if offset + 1 > payload.len() - 2 {
                return Err(ProtocolError::MalformedRecord);
            }
            let count = payload[offset] as usize;
            offset += 1;

            for _ in 0..count {
                if offset + 1 + size > payload.len() - 2 {
                    return Err(ProtocolError::MalformedRecord);
                }
                let id = payload[offset];
                offset += 1;
                let value: u64 = match size {
                    1 => payload[offset] as u64,
                    2 => u16::from_be_bytes([payload[offset], payload[offset + 1]]) as u64,
                    4 => u32::from_be_bytes([
                        payload[offset],
                        payload[offset + 1],
                        payload[offset + 2],
                        payload[offset + 3],
                    ]) as u64,
                    8 => u64::from_be_bytes([
                        payload[offset],
                        payload[offset + 1],
                        payload[offset + 2],
                        payload[offset + 3],
                        payload[offset + 4],
                        payload[offset + 5],
                        payload[offset + 6],
                        payload[offset + 7],
                    ]),
                    _ => unreachable!(),
                };
                offset += size;
                io.insert(id, value as i64);
            }
        }

        position = Some(Position {
            imei: imei.to_string(),
            timestamp_ms,
            latitude,
            longitude,
            altitude,
            speed,
            angle,
            satellites,
            io,
        });
    }

    let position = position.ok_or(ProtocolError::MalformedRecord)?;
    Ok((position, total))
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
            io: BTreeMap::new(),
        }
    }

    #[test]
    fn io_elements_roundtrip() {
        let mut pos = sample_position();
        pos.io.insert(16, 115_488_039); // total odometer, meters
        pos.io.insert(21, 4); // GSM signal
        pos.io.insert(66, 14_250); // external voltage, mV
        pos.io.insert(67, 4_000); // battery voltage, mV
        pos.io.insert(201, 30); // fuel level

        let packet = encode_codec8_packet(&pos);
        let (decoded, _) = parse_avl_packet(&packet, &pos.imei).unwrap();

        assert_eq!(decoded.io.len(), 5);
        assert_eq!(decoded.io.get(&16), Some(&115_488_039));
        assert_eq!(decoded.io.get(&21), Some(&4));
        assert_eq!(decoded.io.get(&66), Some(&14_250));
        assert_eq!(decoded.io.get(&67), Some(&4_000));
        assert_eq!(decoded.io.get(&201), Some(&30));
    }

    #[test]
    fn crc16_known_values() {
        // CRC-16/CCITT-FALSE of "123456789" is 0x29B1
        assert_eq!(crc16_ccitt(b"123456789"), 0x29B1);
        assert_eq!(crc16_ccitt(b""), 0xFFFF);
    }

    #[test]
    fn encode_then_decode_roundtrip() {
        let pos = sample_position();
        let packet = encode_codec8_packet(&pos);

        let (decoded, consumed) = parse_avl_packet(&packet, &pos.imei).unwrap();
        assert_eq!(consumed, packet.len());
        assert_eq!(decoded.imei, pos.imei);
        assert_eq!(decoded.timestamp_ms, pos.timestamp_ms);
        assert!((decoded.latitude - pos.latitude).abs() < 1e-6);
        assert!((decoded.longitude - pos.longitude).abs() < 1e-6);
        assert_eq!(decoded.altitude, pos.altitude);
        assert_eq!(decoded.speed, pos.speed);
        assert_eq!(decoded.angle, pos.angle);
        assert_eq!(decoded.satellites, pos.satellites);
    }

    #[test]
    fn bad_preamble_rejected() {
        let mut buf = vec![0xAA, 0, 0, 0, 0];
        assert_eq!(
            parse_avl_packet(&buf, "imei").unwrap_err(),
            ProtocolError::BadPreamble
        );
    }

    #[test]
    fn incomplete_buffer() {
        let packet = encode_codec8_packet(&sample_position());
        // Feed only the first half
        let half = &packet[..packet.len() / 2];
        assert_eq!(
            parse_avl_packet(half, "imei").unwrap_err(),
            ProtocolError::Incomplete
        );
    }

    #[test]
    fn crc_mismatch_rejected() {
        let pos = sample_position();
        let mut packet = encode_codec8_packet(&pos);
        // Corrupt the CRC (last byte)
        *packet.last_mut().unwrap() ^= 0xFF;
        assert!(matches!(
            parse_avl_packet(&packet, &pos.imei).unwrap_err(),
            ProtocolError::CrcMismatch { .. }
        ));
    }

    #[test]
    fn login_ack_bytes() {
        assert_eq!(build_login_ack(true), vec![0x01]);
        assert_eq!(build_login_ack(false), vec![0x00]);
        assert_eq!(build_data_ack(3), vec![0x03]);
    }
}
