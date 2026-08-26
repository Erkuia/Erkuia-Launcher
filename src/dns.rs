//! Just enough DNS to find a Minecraft server.
//!
//! Minecraft servers on a custom domain almost always publish an SRV record at
//! `_minecraft._tcp.<domain>` pointing at the real host and port. The vanilla
//! client looks it up; a launcher that only resolves the bare domain connects to
//! nothing and reports the server down while it is running perfectly well.
//!
//! Rust's standard library resolves names but cannot ask for SRV, so the query
//! is assembled by hand and sent to a public resolver over UDP. That is a
//! deliberate trade: it skips whatever the machine has configured, which would
//! matter for a split-horizon or intranet name, and does not matter for a public
//! game server. Every failure falls back to the plain host and port, which is
//! exactly what the launcher did before.

use std::{
    collections::HashMap,
    net::{SocketAddr, UdpSocket},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

/// Tried in order. Two of them, because one unreachable resolver should not
/// decide that a server is offline.
const RESOLVERS: [&str; 2] = ["1.1.1.1:53", "8.8.8.8:53"];

const TIMEOUT: Duration = Duration::from_secs(2);
const TYPE_SRV: u16 = 33;
const CLASS_IN: u16 = 1;

/// How long an answer — including "there is no record" — is reused.
///
/// The status poll runs every ten seconds and the address behind a server does
/// not move on that timescale. Without this, every poll pays for a DNS round
/// trip before it can even open the socket.
const CACHE_TTL: Duration = Duration::from_secs(300);

/// `NOERROR`, and `NXDOMAIN` — the name does not exist. Both are real answers;
/// anything else (SERVFAIL, REFUSED) means ask somebody else.
const RCODE_NOERROR: u8 = 0;
const RCODE_NXDOMAIN: u8 = 3;

/// Set when the reply did not fit in a datagram. The answer section may be cut
/// short, so the record we want could be missing without looking missing.
const FLAG_TRUNCATED: u8 = 0x02;

/// A label is at most 63 bytes and the two high bits mark a compression
/// pointer, so a length byte above this is never a length.
const POINTER_MASK: u8 = 0xC0;

/// Compression pointers can legally point backwards forever. Chasing more jumps
/// than a reply could possibly contain ends a malicious loop without needing to
/// track which offsets have been seen.
const MAX_JUMPS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvTarget {
    pub host: String,
    pub port: u16,
}

/// Looks up `_minecraft._tcp.<host>`, or `None` if there is no such record and
/// for every kind of failure along the way.
pub fn lookup_minecraft_srv(host: &str) -> Option<SrvTarget> {
    let name = format!("_minecraft._tcp.{}", host.trim().trim_end_matches('.'));

    if let Some(cached) = cached(&name) {
        return cached;
    }

    let answer = ask(&name);
    remember(&name, answer.clone());

    answer
}

fn ask(name: &str) -> Option<SrvTarget> {
    let query = build_query(name, TYPE_SRV)?;

    for resolver in RESOLVERS {
        let response = match exchange(resolver, &query) {
            Ok(response) => response,
            Err(error) => {
                log::debug!("{resolver} 에 SRV 를 묻지 못했습니다: {error}");
                continue;
            }
        };

        match verdict(&response) {
            Verdict::Answered => {
                let target = parse_srv(&response);

                match &target {
                    Some(found) => log::debug!("SRV {name} -> {}:{}", found.host, found.port),
                    None => log::debug!("SRV {name} 레코드 없음"),
                }

                return target;
            }
            // A resolver that failed to answer has told us nothing about the
            // name, so the next one still gets asked.
            Verdict::Unusable => log::debug!("{resolver} 의 SRV 응답을 쓸 수 없습니다"),
        }
    }

    None
}

enum Verdict {
    Answered,
    Unusable,
}

/// Rejects replies that carry no usable answer, so a broken resolver does not
/// masquerade as a domain without an SRV record.
fn verdict(data: &[u8]) -> Verdict {
    if data.len() < 12 {
        return Verdict::Unusable;
    }

    if data[2] & FLAG_TRUNCATED != 0 {
        return Verdict::Unusable;
    }

    match data[3] & 0x0F {
        RCODE_NOERROR | RCODE_NXDOMAIN => Verdict::Answered,
        _ => Verdict::Unusable,
    }
}

type Cache = HashMap<String, (Instant, Option<SrvTarget>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `Some(answer)` when the cache can speak for this name, including a cached
/// `Some(None)` meaning "asked, and there is no record".
fn cached(name: &str) -> Option<Option<SrvTarget>> {
    let guard = cache().lock().ok()?;
    let (stored, answer) = guard.get(name)?;

    (stored.elapsed() < CACHE_TTL).then(|| answer.clone())
}

fn remember(name: &str, answer: Option<SrvTarget>) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(name.to_string(), (Instant::now(), answer));
    }
}

fn exchange(resolver: &str, query: &[u8]) -> std::io::Result<Vec<u8>> {
    let address: SocketAddr = resolver
        .parse()
        .map_err(|_| std::io::Error::other("resolver address"))?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    socket.set_write_timeout(Some(TIMEOUT))?;
    socket.connect(address)?;
    socket.send(query)?;

    let mut buffer = vec![0u8; 1232];
    let read = socket.recv(&mut buffer)?;
    buffer.truncate(read);

    Ok(buffer)
}

fn build_query(name: &str, qtype: u16) -> Option<Vec<u8>> {
    let mut packet = Vec::with_capacity(name.len() + 18);

    // A fixed id is fine: the socket is connected and used for exactly one
    // exchange, so there is no second reply to tell apart.
    packet.extend_from_slice(&0x4552u16.to_be_bytes()); // id
    packet.extend_from_slice(&0x0100u16.to_be_bytes()); // recursion desired
    packet.extend_from_slice(&1u16.to_be_bytes()); // one question
    packet.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // no answers, authority, extra

    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }

        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }

    packet.push(0);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&CLASS_IN.to_be_bytes());

    Some(packet)
}

/// Returns the lowest-priority SRV target in the answer section.
fn parse_srv(data: &[u8]) -> Option<SrvTarget> {
    if data.len() < 12 {
        return None;
    }

    let questions = u16::from_be_bytes([data[4], data[5]]);
    let answers = u16::from_be_bytes([data[6], data[7]]);

    let mut offset = 12;

    for _ in 0..questions {
        offset = skip_name(data, offset)?;
        offset = offset.checked_add(4)?; // qtype + qclass
    }

    let mut best: Option<(u16, SrvTarget)> = None;

    for _ in 0..answers {
        offset = skip_name(data, offset)?;

        let header = data.get(offset..offset.checked_add(10)?)?;
        let rtype = u16::from_be_bytes([header[0], header[1]]);
        let length = u16::from_be_bytes([header[8], header[9]]) as usize;

        offset += 10;
        let record = offset;
        offset = offset.checked_add(length)?;

        if rtype != TYPE_SRV || length < 7 {
            continue;
        }

        let body = data.get(record..record + 6)?;
        let priority = u16::from_be_bytes([body[0], body[1]]);
        let port = u16::from_be_bytes([body[4], body[5]]);
        let (host, _) = read_name(data, record + 6)?;

        // A target of "." means the service is explicitly not offered here.
        if host.is_empty() {
            continue;
        }

        let better = match &best {
            Some((current, _)) => priority < *current,
            None => true,
        };

        if better {
            best = Some((priority, SrvTarget { host, port }));
        }
    }

    best.map(|(_, target)| target)
}

/// Walks past a name without expanding it, which is all that is needed to find
/// the field after it. A pointer always ends the name.
fn skip_name(data: &[u8], mut offset: usize) -> Option<usize> {
    loop {
        let length = *data.get(offset)?;

        if length & POINTER_MASK == POINTER_MASK {
            return offset.checked_add(2);
        }

        offset = offset.checked_add(1)?;

        if length == 0 {
            return Some(offset);
        }

        offset = offset.checked_add(length as usize)?;
    }
}

/// Expands a name, following compression pointers.
fn read_name(data: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut offset = start;
    let mut after: Option<usize> = None;
    let mut jumps = 0;

    loop {
        let length = *data.get(offset)?;

        if length & POINTER_MASK == POINTER_MASK {
            let low = *data.get(offset.checked_add(1)?)?;
            let target = (((length & 0x3F) as usize) << 8) | low as usize;

            after.get_or_insert(offset + 2);

            jumps += 1;
            if jumps > MAX_JUMPS {
                return None;
            }

            offset = target;
            continue;
        }

        offset = offset.checked_add(1)?;

        if length == 0 {
            break;
        }

        let end = offset.checked_add(length as usize)?;
        let label = data.get(offset..end)?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        offset = end;
    }

    Some((labels.join("."), after.unwrap_or(offset)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(parts: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for part in parts {
            out.push(part.len() as u8);
            out.extend_from_slice(part.as_bytes());
        }
        out.push(0);
        out
    }

    /// question + one SRV answer, with the target written out in full.
    fn reply(records: &[(u16, u16, &[&str])]) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend_from_slice(&0x4552u16.to_be_bytes());
        packet.extend_from_slice(&0x8180u16.to_be_bytes());
        packet.extend_from_slice(&1u16.to_be_bytes());
        packet.extend_from_slice(&(records.len() as u16).to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]);

        packet.extend_from_slice(&name(&["_minecraft", "_tcp", "erkuia", "kr"]));
        packet.extend_from_slice(&TYPE_SRV.to_be_bytes());
        packet.extend_from_slice(&CLASS_IN.to_be_bytes());

        for (priority, port, target) in records {
            packet.extend_from_slice(&name(&["_minecraft", "_tcp", "erkuia", "kr"]));
            packet.extend_from_slice(&TYPE_SRV.to_be_bytes());
            packet.extend_from_slice(&CLASS_IN.to_be_bytes());
            packet.extend_from_slice(&300u32.to_be_bytes());

            let mut body = Vec::new();
            body.extend_from_slice(&priority.to_be_bytes());
            body.extend_from_slice(&0u16.to_be_bytes()); // weight
            body.extend_from_slice(&port.to_be_bytes());
            body.extend_from_slice(&name(target));

            packet.extend_from_slice(&(body.len() as u16).to_be_bytes());
            packet.extend_from_slice(&body);
        }

        packet
    }

    #[test]
    fn a_query_is_shaped_like_a_question() {
        let query = build_query("_minecraft._tcp.erkuia.kr", TYPE_SRV).unwrap();

        assert_eq!(u16::from_be_bytes([query[4], query[5]]), 1, "one question");
        assert_eq!(u16::from_be_bytes([query[6], query[7]]), 0, "no answers");
        assert!(query.ends_with(&[0, 33, 0, 1]), "SRV/IN terminated");

        // Labels are length-prefixed, not dot-separated, on the wire.
        let labels: &[u8] = b"\x0a_minecraft\x04_tcp\x06erkuia\x02kr\x00";
        assert!(
            query.windows(labels.len()).any(|window| window == labels),
            "{query:?}"
        );
    }

    #[test]
    fn a_label_that_cannot_be_encoded_is_refused() {
        assert!(build_query("", TYPE_SRV).is_none());
        assert!(build_query(&"a".repeat(64), TYPE_SRV).is_none());
    }

    #[test]
    fn a_single_record_is_read() {
        let target = parse_srv(&reply(&[(0, 25577, &["node1", "erkuia", "kr"])])).unwrap();

        assert_eq!(
            target,
            SrvTarget {
                host: "node1.erkuia.kr".to_string(),
                port: 25577,
            }
        );
    }

    /// Priority is "lower wins", and getting it backwards would send everyone to
    /// the backup node.
    #[test]
    fn the_lowest_priority_wins() {
        let data = reply(&[
            (20, 1111, &["backup", "erkuia", "kr"]),
            (5, 2222, &["main", "erkuia", "kr"]),
        ]);

        assert_eq!(parse_srv(&data).unwrap().port, 2222);
    }

    #[test]
    fn an_empty_answer_section_yields_nothing() {
        assert!(parse_srv(&reply(&[])).is_none());
    }

    #[test]
    fn a_root_target_means_no_service() {
        assert!(parse_srv(&reply(&[(0, 25565, &[])])).is_none());
    }

    #[test]
    fn a_truncated_reply_is_refused_rather_than_panicking() {
        let full = reply(&[(0, 25565, &["node1", "erkuia", "kr"])]);

        for cut in 0..full.len() {
            let _ = parse_srv(&full[..cut]);
        }
    }

    #[test]
    fn a_compression_pointer_is_followed() {
        let mut data = reply(&[(0, 25565, &["node1", "erkuia", "kr"])]);
        // The question name sits at offset 12 and is what a real server would
        // point at rather than repeating.
        let pointer = [POINTER_MASK, 12];

        let start = data.len();
        data.extend_from_slice(&pointer);

        assert_eq!(
            read_name(&data, start).unwrap().0,
            "_minecraft._tcp.erkuia.kr"
        );
    }

    fn flags(data: &mut [u8], value: u16) {
        data[2..4].copy_from_slice(&value.to_be_bytes());
    }

    #[test]
    fn a_clean_reply_counts_as_an_answer() {
        let data = reply(&[(0, 25565, &["node1", "erkuia", "kr"])]);

        assert!(matches!(verdict(&data), Verdict::Answered));
    }

    /// NXDOMAIN is a real answer: the name is not there, and asking a second
    /// resolver would only hear the same thing more slowly.
    #[test]
    fn a_missing_name_counts_as_an_answer() {
        let mut data = reply(&[]);
        flags(&mut data, 0x8183);

        assert!(matches!(verdict(&data), Verdict::Answered));
    }

    /// The bug this guards: a resolver that failed was being read as "the domain
    /// has no SRV record", so the second resolver never got asked.
    #[test]
    fn a_failed_resolver_is_not_mistaken_for_an_empty_domain() {
        for rcode in [2u16, 5] {
            let mut data = reply(&[]);
            flags(&mut data, 0x8180 | rcode);

            assert!(matches!(verdict(&data), Verdict::Unusable), "rcode {rcode}");
        }
    }

    #[test]
    fn a_truncated_reply_is_not_trusted() {
        let mut data = reply(&[(0, 25565, &["node1", "erkuia", "kr"])]);
        flags(&mut data, 0x8380);

        assert!(matches!(verdict(&data), Verdict::Unusable));
    }

    #[test]
    fn a_runt_reply_is_not_trusted() {
        assert!(matches!(verdict(&[0u8; 4]), Verdict::Unusable));
    }

    #[test]
    fn the_cache_answers_for_both_a_hit_and_a_known_miss() {
        let found = SrvTarget { host: "node1.erkuia.kr".to_string(), port: 25577 };

        remember("_minecraft._tcp.hit.test", Some(found.clone()));
        remember("_minecraft._tcp.miss.test", None);

        assert_eq!(cached("_minecraft._tcp.hit.test"), Some(Some(found)));
        assert_eq!(cached("_minecraft._tcp.miss.test"), Some(None));
        assert_eq!(cached("_minecraft._tcp.unasked.test"), None);
    }

    #[test]
    fn a_pointer_loop_terminates() {
        let mut data = vec![0u8; 12];
        data.extend_from_slice(&[POINTER_MASK, 12]);

        assert!(read_name(&data, 12).is_none());
    }
}
