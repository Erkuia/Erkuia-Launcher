//! The server's own answer to "is anyone home".
//!
//! Minecraft's Server List Ping is the same handshake the multiplayer screen
//! uses, so the numbers here are exactly what the player would see there. It
//! runs over a plain socket with no authentication, which is why this can report
//! before the person has logged in — the login state gates the display, not the
//! query.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use anyhow::{bail, Context};
use serde::Deserialize;

pub const DEFAULT_PORT: u16 = 25565;

/// Long enough for a distant server on a slow link, short enough that a black
/// hole does not hold the poll loop past its next tick.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The protocol version is sent as `-1`, the documented "just asking" value.
/// Naming a real version would make a server with a version filter answer
/// differently, and this is a status query, not a join.
const PROTOCOL_UNKNOWN: i32 = -1;

/// A status response is a JSON blob of arbitrary size (MOTD, favicon, sample
/// player list). The favicon alone is routinely 20 KB, so the cap is generous —
/// but it is still a cap, because the length is a number the server chooses.
const MAX_RESPONSE: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    /// Nobody has logged in yet, so nothing has been asked.
    Idle,
    Offline,
    Online { players: u32, max: u32, ping_ms: u32 },
}

impl ServerStatus {
    pub fn is_online(&self) -> bool {
        matches!(self, Self::Online { .. })
    }

    /// Idle and offline both show zeros, but only offline is a problem — the
    /// dot needs to tell them apart.
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "대기 중",
            Self::Offline => "오프라인",
            Self::Online { .. } => "온라인",
        }
    }

    /// Always the two-number form, zeros included: a lone `0` where `12 / 100`
    /// normally sits reads as a different field rather than an empty one.
    pub fn players_text(&self) -> String {
        let (players, max) = match self {
            Self::Online { players, max, .. } => (*players, *max),
            _ => (0, 0),
        };

        format!("{players} / {max}")
    }

    pub fn ping_text(&self) -> String {
        match self {
            Self::Online { ping_ms, .. } => format!("{ping_ms}ms"),
            _ => "0ms".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    #[serde(default)]
    players: Players,
}

#[derive(Debug, Default, Deserialize)]
struct Players {
    #[serde(default)]
    online: u32,
    #[serde(default)]
    max: u32,
}

/// Splits `host:port`, defaulting the port. An address with no colon is the
/// common case and must not be treated as malformed.
pub fn split_address(address: &str) -> (String, u16) {
    let address = address.trim();

    match address.rsplit_once(':') {
        Some((host, port)) => match port.parse() {
            Ok(port) => (host.to_string(), port),
            // A trailing colon with junk after it is likelier a typo than an
            // IPv6 literal here, and the default port still gives the person a
            // working lookup instead of a hard failure.
            Err(_) => (address.to_string(), DEFAULT_PORT),
        },
        None => (address.to_string(), DEFAULT_PORT),
    }
}

/// Asks the server, returning [`ServerStatus::Offline`] for every failure.
///
/// A status query has no actionable failure mode for the person: unreachable,
/// refused, and malformed all mean the same thing on the row. The distinction
/// still reaches the log.
pub fn query(address: &str) -> ServerStatus {
    if address.trim().is_empty() {
        return ServerStatus::Idle;
    }

    let (host, port) = split_address(address);

    match probe(&host, port) {
        Ok(status) => status,
        Err(error) => {
            log::debug!("서버 상태 조회 실패 ({host}:{port}): {error:#}");
            ServerStatus::Offline
        }
    }
}

fn probe(host: &str, port: u16) -> anyhow::Result<ServerStatus> {
    let started = Instant::now();

    let target = resolve(host, port)?;
    let mut stream = TcpStream::connect_timeout(&target, TIMEOUT)
        .with_context(|| format!("{host}:{port} 에 연결하지 못했어요."))?;

    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    // The status exchange is two tiny writes; batching them into one segment
    // saves a round trip that would otherwise land inside the ping we report.
    stream.set_nodelay(true)?;

    let mut handshake = Vec::new();
    write_varint(&mut handshake, PROTOCOL_UNKNOWN);
    write_string(&mut handshake, host);
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut handshake, 1); // next state: status

    write_packet(&mut stream, 0x00, &handshake)?;
    write_packet(&mut stream, 0x00, &[])?; // status request

    let payload = read_packet(&mut stream, 0x00)?;

    // Measured around the whole exchange rather than a separate ping/pong
    // packet: it is the number that describes how long the server actually took
    // to answer, and a second round trip would only add a figure to reconcile.
    let ping_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let mut cursor = &payload[..];
    let json = read_string(&mut cursor)?;

    let response: StatusResponse =
        serde_json::from_str(&json).context("서버 상태 응답을 해석하지 못했어요.")?;

    Ok(ServerStatus::Online {
        players: response.players.online,
        max: response.players.max,
        ping_ms,
    })
}

fn resolve(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    (host, port)
        .to_socket_addrs()
        .with_context(|| format!("{host} 주소를 찾지 못했어요."))?
        .next()
        .with_context(|| format!("{host} 에 대한 주소가 없어요."))
}

fn write_packet(stream: &mut TcpStream, id: i32, body: &[u8]) -> anyhow::Result<()> {
    let mut packet = Vec::with_capacity(body.len() + 5);
    write_varint(&mut packet, id);
    packet.extend_from_slice(body);

    let mut framed = Vec::with_capacity(packet.len() + 5);
    write_varint(&mut framed, packet.len() as i32);
    framed.extend_from_slice(&packet);

    stream.write_all(&framed)?;
    stream.flush()?;

    Ok(())
}

fn read_packet(stream: &mut TcpStream, expected_id: i32) -> anyhow::Result<Vec<u8>> {
    let length = read_varint_from(stream)?;

    if length <= 0 || length as usize > MAX_RESPONSE {
        bail!("서버가 이상한 크기의 응답을 보냈어요: {length}");
    }

    let mut body = vec![0u8; length as usize];
    stream.read_exact(&mut body)?;

    let mut cursor = &body[..];
    let id = read_varint(&mut cursor)?;

    if id != expected_id {
        bail!("예상하지 못한 패킷이에요: {id}");
    }

    Ok(cursor.to_vec())
}

fn write_varint(buffer: &mut Vec<u8>, value: i32) {
    let mut remaining = value as u32;

    loop {
        let mut byte = (remaining & 0x7F) as u8;
        remaining >>= 7;

        if remaining != 0 {
            byte |= 0x80;
        }

        buffer.push(byte);

        if remaining == 0 {
            return;
        }
    }
}

fn write_string(buffer: &mut Vec<u8>, value: &str) {
    write_varint(buffer, value.len() as i32);
    buffer.extend_from_slice(value.as_bytes());
}

/// Accumulated as `u32` and reinterpreted at the end. A negative VarInt fills
/// all five bytes, and the last one shifted into an `i32` overflows the sign bit
/// — which panics in a debug build rather than producing the value the protocol
/// asked for.
fn read_varint(cursor: &mut &[u8]) -> anyhow::Result<i32> {
    let mut value: u32 = 0;

    for shift in 0..5 {
        let (byte, rest) = cursor.split_first().context("VarInt 가 중간에 끊겼어요.")?;
        *cursor = rest;

        value |= u32::from(byte & 0x7F) << (shift * 7);

        if byte & 0x80 == 0 {
            return Ok(value as i32);
        }
    }

    bail!("VarInt 가 너무 길어요.")
}

/// The framing length arrives before the body, so it has to be read a byte at a
/// time straight off the socket rather than from a buffer.
fn read_varint_from(stream: &mut TcpStream) -> anyhow::Result<i32> {
    let mut value: u32 = 0;

    for shift in 0..5 {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;

        value |= u32::from(byte[0] & 0x7F) << (shift * 7);

        if byte[0] & 0x80 == 0 {
            return Ok(value as i32);
        }
    }

    bail!("VarInt 가 너무 길어요.")
}

fn read_string(cursor: &mut &[u8]) -> anyhow::Result<String> {
    let length = read_varint(cursor)?;

    if length < 0 || length as usize > cursor.len() {
        bail!("문자열 길이가 올바르지 않아요: {length}");
    }

    let (text, rest) = cursor.split_at(length as usize);
    *cursor = rest;

    String::from_utf8(text.to_vec()).context("서버 응답이 UTF-8 이 아니에요.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_gets_the_default_port() {
        assert_eq!(split_address("erkuia.kr"), ("erkuia.kr".to_string(), 25565));
    }

    #[test]
    fn an_explicit_port_wins() {
        assert_eq!(
            split_address("erkuia.kr:25566"),
            ("erkuia.kr".to_string(), 25566)
        );
    }

    #[test]
    fn surrounding_space_is_ignored() {
        assert_eq!(
            split_address("  erkuia.kr\n"),
            ("erkuia.kr".to_string(), 25565)
        );
    }

    #[test]
    fn an_unparseable_port_falls_back_rather_than_failing() {
        assert_eq!(
            split_address("erkuia.kr:port"),
            ("erkuia.kr:port".to_string(), 25565)
        );
    }

    #[test]
    fn an_empty_address_is_idle_not_offline() {
        assert_eq!(query("   "), ServerStatus::Idle);
    }

    #[test]
    fn varints_round_trip() {
        for value in [0, 1, 127, 128, 255, 25565, i32::MAX, -1, i32::MIN] {
            let mut buffer = Vec::new();
            write_varint(&mut buffer, value);

            let mut cursor = &buffer[..];
            assert_eq!(read_varint(&mut cursor).unwrap(), value, "{value}");
            assert!(cursor.is_empty(), "{value} left trailing bytes");
        }
    }

    #[test]
    fn a_negative_varint_uses_the_full_five_bytes() {
        let mut buffer = Vec::new();
        write_varint(&mut buffer, PROTOCOL_UNKNOWN);

        assert_eq!(buffer.len(), 5);

        let mut cursor = &buffer[..];
        assert_eq!(read_varint(&mut cursor).unwrap(), PROTOCOL_UNKNOWN);
    }

    #[test]
    fn strings_round_trip() {
        let mut buffer = Vec::new();
        write_string(&mut buffer, "erkuia.kr");

        let mut cursor = &buffer[..];
        assert_eq!(read_string(&mut cursor).unwrap(), "erkuia.kr");
    }

    #[test]
    fn a_truncated_varint_is_refused() {
        let mut cursor = &[0x80u8][..];

        assert!(read_varint(&mut cursor).is_err());
    }

    #[test]
    fn a_string_longer_than_its_buffer_is_refused() {
        let mut cursor = &[0x7Fu8, b'a'][..];

        assert!(read_string(&mut cursor).is_err());
    }

    #[test]
    fn missing_player_counts_default_to_zero() {
        let response: StatusResponse = serde_json::from_str(r#"{"description":"hi"}"#).unwrap();

        assert_eq!(response.players.online, 0);
        assert_eq!(response.players.max, 0);
    }

    #[test]
    fn player_counts_are_read() {
        let response: StatusResponse =
            serde_json::from_str(r#"{"players":{"online":12,"max":100}}"#).unwrap();

        assert_eq!(response.players.online, 12);
        assert_eq!(response.players.max, 100);
    }

    #[test]
    fn the_row_reads_zero_when_there_is_nothing_to_show() {
        for status in [ServerStatus::Idle, ServerStatus::Offline] {
            assert_eq!(status.players_text(), "0 / 0");
            assert_eq!(status.ping_text(), "0ms");
            assert!(!status.is_online());
        }

        assert_eq!(ServerStatus::Idle.label(), "대기 중");
        assert_eq!(ServerStatus::Offline.label(), "오프라인");
        assert!(ServerStatus::Idle.is_idle());
        assert!(!ServerStatus::Offline.is_idle());
    }

    #[test]
    fn an_online_server_shows_its_numbers() {
        let status = ServerStatus::Online {
            players: 12,
            max: 100,
            ping_ms: 24,
        };

        assert_eq!(status.label(), "온라인");
        assert_eq!(status.players_text(), "12 / 100");
        assert_eq!(status.ping_text(), "24ms");
        assert!(status.is_online());
    }
}
