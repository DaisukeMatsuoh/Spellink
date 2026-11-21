//! The authoritative DNS server for the LAN zone (e.g. lan.spellink.uk): `<id>.zone` → the
//! bridge's private address, `_acme-challenge.<id>.zone` → the ACME TXT value. It answers only for
//! its own zone, never recurses, and does not echo anything it was not asked for, so it cannot be
//! used to amplify traffic (interfaces.md I30).

use super::store::Names;
use crate::namesproto;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const TYPE_A: u16 = 1;
const TYPE_NS: u16 = 2;
const TYPE_SOA: u16 = 6;
const TYPE_TXT: u16 = 16;
const TYPE_OPT: u16 = 41;
const TYPE_AXFR: u16 = 252;
const TYPE_IXFR: u16 = 251;
const CLASS_IN: u16 = 1;
const TTL: u32 = 60;
const NS_TTL: u32 = 3600;

const RCODE_FORMERR: u16 = 1;
const RCODE_NXDOMAIN: u16 = 3;
const RCODE_NOTIMP: u16 = 4;
const RCODE_REFUSED: u16 = 5;

/// The zone this server is the authority for, and the names it announces for itself.
pub struct Zone {
    pub labels: Vec<String>,
    pub ns: Vec<String>,
    pub hostmaster: Vec<String>,
}

pub fn labels_of(name: &str) -> Option<Vec<String>> {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<String> = name.split('.').map(String::from).collect();
    let ok = !name.is_empty() && name.len() <= 253 && labels.iter().all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    ok.then_some(labels)
}

impl Zone {
    pub fn new(zone: &str, ns: &str, hostmaster: &str) -> Result<Zone, String> {
        let l = |what: &str, v: &str| labels_of(v).ok_or_else(|| format!("{what} {v:?} is not a DNS name"));
        let zone = Zone { labels: l("zone", zone)?, ns: l("ns", ns)?, hostmaster: l("hostmaster", hostmaster)? };
        if zone.labels.len() < 2 {
            return Err("the zone needs at least two labels".into());
        }
        Ok(zone)
    }

    pub fn name(&self) -> String {
        self.labels.join(".")
    }
}

fn encode_name(labels: &[String], out: &mut Vec<u8>) {
    for l in labels {
        out.push(l.len() as u8);
        out.extend_from_slice(l.as_bytes());
    }
    out.push(0);
}

struct Query {
    id: [u8; 2],
    rd: bool,
    /// The question exactly as it came (case is kept: resolvers randomize it to detect spoofing).
    question_raw: Vec<u8>,
    name: Vec<String>,
    qtype: u16,
    qclass: u16,
    /// The size the client says it can take over UDP, when it sent an OPT record.
    edns: Option<u16>,
}

enum Parsed {
    Drop,
    /// An error to send back: the header only (`question` when it was readable).
    Fail(u16, [u8; 2], bool),
    Ok(Query),
}

fn parse(buf: &[u8]) -> Parsed {
    if buf.len() < 12 {
        return Parsed::Drop;
    }
    let id = [buf[0], buf[1]];
    let flags = u16::from_be_bytes([buf[2], buf[3]]);
    if flags & 0x8000 != 0 {
        return Parsed::Drop; // a response: never answer a response
    }
    let rd = flags & 0x0100 != 0;
    if (flags >> 11) & 0xF != 0 {
        return Parsed::Fail(RCODE_NOTIMP, id, rd);
    }
    let count = |i: usize| u16::from_be_bytes([buf[i], buf[i + 1]]);
    let (qd, an, ns, ar) = (count(4), count(6), count(8), count(10));
    if qd != 1 || an != 0 || ns != 0 || ar > 1 {
        return Parsed::Fail(RCODE_FORMERR, id, rd);
    }
    let mut pos = 12;
    let mut name = Vec::new();
    let mut wire = 1;
    loop {
        let Some(&len) = buf.get(pos) else { return Parsed::Fail(RCODE_FORMERR, id, rd) };
        pos += 1;
        if len == 0 {
            break;
        }
        // 0b11 is a compression pointer, 0b01/0b10 are extended labels: none belongs in a question
        if len & 0xC0 != 0 {
            return Parsed::Fail(RCODE_FORMERR, id, rd);
        }
        let end = pos + len as usize;
        wire += 1 + len as usize;
        let Some(label) = buf.get(pos..end) else { return Parsed::Fail(RCODE_FORMERR, id, rd) };
        if wire > 255 {
            return Parsed::Fail(RCODE_FORMERR, id, rd);
        }
        name.push(String::from_utf8_lossy(label).to_ascii_lowercase());
        pos = end;
    }
    let Some(tc) = buf.get(pos..pos + 4) else { return Parsed::Fail(RCODE_FORMERR, id, rd) };
    let (qtype, qclass) = (u16::from_be_bytes([tc[0], tc[1]]), u16::from_be_bytes([tc[2], tc[3]]));
    pos += 4;
    let question_raw = buf[12..pos].to_vec();
    let mut edns = None;
    if ar == 1 {
        // the only additional record we accept: an OPT on the root name
        match buf.get(pos..pos + 11) {
            Some(o) if o[0] == 0 && u16::from_be_bytes([o[1], o[2]]) == TYPE_OPT => {
                edns = Some(u16::from_be_bytes([o[3], o[4]]));
                let rdlen = u16::from_be_bytes([o[9], o[10]]) as usize;
                pos += 11 + rdlen;
            }
            _ => return Parsed::Fail(RCODE_FORMERR, id, rd),
        }
    }
    if pos != buf.len() {
        return Parsed::Fail(RCODE_FORMERR, id, rd);
    }
    Parsed::Ok(Query { id, rd, question_raw, name, qtype, qclass, edns })
}

struct Answer {
    rcode: u16,
    authoritative: bool,
    /// (type, ttl, rdata) of records for the question's name.
    answers: Vec<(u16, u32, Vec<u8>)>,
    /// Put the zone's SOA in the authority section (an answer that says "nothing here").
    soa: bool,
    /// NS records for the zone apex (answers to NS questions).
    apex_ns: bool,
}

fn refuse() -> Answer {
    Answer { rcode: RCODE_REFUSED, authoritative: false, answers: vec![], soa: false, apex_ns: false }
}

fn nothing(rcode: u16) -> Answer {
    Answer { rcode, authoritative: true, answers: vec![], soa: true, apex_ns: false }
}

fn soa_rdata(zone: &Zone, now: i64) -> Vec<u8> {
    let mut r = Vec::new();
    encode_name(&zone.ns, &mut r);
    encode_name(&zone.hostmaster, &mut r);
    // the serial changes once a day: nothing here is ever transferred, so it only has to be valid
    for v in [(now / 86_400) as u32, 3600, 600, 86_400, TTL] {
        r.extend_from_slice(&v.to_be_bytes());
    }
    r
}

fn decide(zone: &Zone, names: &Names, q: &Query, now: i64) -> Answer {
    if q.qclass != CLASS_IN || q.qtype == TYPE_AXFR || q.qtype == TYPE_IXFR {
        return refuse();
    }
    let n = q.name.len();
    let z = zone.labels.len();
    if n < z || q.name[n - z..] != zone.labels[..] {
        return refuse(); // not ours; and we never recurse
    }
    let below = &q.name[..n - z];
    // "ANY" gets the least it can: "nothing" (RFC 8482), never a list of everything
    let t = q.qtype;
    match below {
        [] => match t {
            TYPE_SOA => Answer { rcode: 0, authoritative: true, answers: vec![(TYPE_SOA, NS_TTL, soa_rdata(zone, now))], soa: false, apex_ns: true },
            TYPE_NS => Answer { rcode: 0, authoritative: true, answers: vec![], soa: false, apex_ns: true },
            _ => nothing(0),
        },
        [id] if namesproto::valid_id(id) => {
            if !names.exists(id) {
                return nothing(RCODE_NXDOMAIN);
            }
            match (t, names.address(id)) {
                (TYPE_A, Some(ip)) => Answer { rcode: 0, authoritative: true, answers: vec![(TYPE_A, TTL, ip.octets().to_vec())], soa: false, apex_ns: false },
                _ => nothing(0),
            }
        }
        [c, id] if c == "_acme-challenge" && namesproto::valid_id(id) => {
            if !names.exists(id) {
                return nothing(RCODE_NXDOMAIN);
            }
            if t != TYPE_TXT {
                return nothing(0);
            }
            let answers: Vec<_> = names
                .txt(id, now)
                .into_iter()
                .map(|v| {
                    let mut r = vec![v.len() as u8];
                    r.extend_from_slice(v.as_bytes());
                    (TYPE_TXT, TTL, r)
                })
                .collect();
            let empty = answers.is_empty();
            Answer { rcode: 0, authoritative: true, answers, soa: empty, apex_ns: false }
        }
        _ => nothing(RCODE_NXDOMAIN),
    }
}

fn header(q_id: [u8; 2], rd: bool, aa: bool, tc: bool, rcode: u16, counts: [u16; 4]) -> Vec<u8> {
    let flags: u16 = 0x8000 | if aa { 0x0400 } else { 0 } | if tc { 0x0200 } else { 0 } | if rd { 0x0100 } else { 0 } | rcode;
    let mut h = q_id.to_vec();
    h.extend_from_slice(&flags.to_be_bytes());
    for c in counts {
        h.extend_from_slice(&c.to_be_bytes());
    }
    h
}

fn push_rr(out: &mut Vec<u8>, name_ptr: bool, owner: &[String], ty: u16, ttl: u32, rdata: &[u8]) {
    if name_ptr {
        out.extend_from_slice(&[0xC0, 0x0C]); // the question's name
    } else {
        encode_name(owner, out);
    }
    out.extend_from_slice(&ty.to_be_bytes());
    out.extend_from_slice(&CLASS_IN.to_be_bytes());
    out.extend_from_slice(&ttl.to_be_bytes());
    out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
    out.extend_from_slice(rdata);
}

/// The reply to one query, or None to say nothing. `udp_limit` is 0 for TCP (no size limit).
pub fn respond(zone: &Zone, names: &Names, query: &[u8], tcp: bool, now: i64) -> Option<Vec<u8>> {
    let q = match parse(query) {
        Parsed::Drop => return None,
        Parsed::Fail(rcode, id, rd) => return Some(header(id, rd, false, false, rcode, [0; 4])),
        Parsed::Ok(q) => q,
    };
    let a = decide(zone, names, &q, now);
    let mut apex_ns_rdata = Vec::new();
    encode_name(&zone.ns, &mut apex_ns_rdata);
    let mut answers: Vec<(bool, u16, u32, Vec<u8>)> = a.answers.iter().map(|(t, ttl, r)| (true, *t, *ttl, r.clone())).collect();
    if a.apex_ns {
        // the apex's NS: an answer to an NS question, otherwise it goes after the SOA answer
        answers.push((true, TYPE_NS, NS_TTL, apex_ns_rdata));
    }
    let mut authority = Vec::new();
    if a.soa {
        push_rr(&mut authority, false, &zone.labels, TYPE_SOA, TTL, &soa_rdata(zone, now));
    }
    let mut body = Vec::new();
    let mut an_count = 0u16;
    for (ptr, t, ttl, r) in &answers {
        // for an SOA question the NS record is extra and would only make the reply longer
        if q.qtype == TYPE_SOA && *t == TYPE_NS {
            continue;
        }
        if q.qtype == TYPE_NS && *t == TYPE_SOA {
            continue;
        }
        push_rr(&mut body, *ptr, &q.name, *t, *ttl, r);
        an_count += 1;
    }
    let mut out = header(q.id, q.rd, a.authoritative, false, a.rcode, [1, an_count, u16::from(a.soa), u16::from(q.edns.is_some())]);
    out.extend_from_slice(&q.question_raw);
    out.extend_from_slice(&body);
    out.extend_from_slice(&authority);
    if q.edns.is_some() {
        // an empty OPT, advertising the size we are willing to send
        out.extend_from_slice(&[0, 0, 41, 0x04, 0xD0, 0, 0, 0, 0, 0, 0]);
    }
    let limit = match (tcp, q.edns) {
        (true, _) => usize::MAX,
        (false, Some(s)) => (s as usize).clamp(512, 1232),
        (false, None) => 512,
    };
    if out.len() > limit {
        // too big for UDP: say so, with the question and nothing else; the client asks again over TCP
        let mut cut = header(q.id, q.rd, a.authoritative, true, a.rcode, [1, 0, 0, 0]);
        cut.extend_from_slice(&q.question_raw);
        return Some(cut);
    }
    Some(out)
}

pub struct Dns {
    pub zone: Zone,
    pub names: Arc<Names>,
    pub limiter: crate::relay::Limiter,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Dns {
    fn allowed(&self, peer: &SocketAddr) -> bool {
        self.limiter.allow(&peer.ip().to_string(), std::time::Instant::now())
    }
}

pub async fn serve_udp(dns: Arc<Dns>, socket: tokio::net::UdpSocket, cancel: tokio_util::sync::CancellationToken) {
    let mut buf = vec![0u8; 1500];
    loop {
        let (n, peer) = tokio::select! {
            _ = cancel.cancelled() => return,
            r = socket.recv_from(&mut buf) => match r { Ok(v) => v, Err(_) => continue },
        };
        if !dns.allowed(&peer) {
            continue; // silence, not an answer: a spoofed source gets nothing
        }
        if let Some(reply) = respond(&dns.zone, &dns.names, &buf[..n], false, now()) {
            let _ = socket.send_to(&reply, peer).await; // UDP: a lost reply is the client's retry
        }
    }
}

pub async fn serve_tcp(dns: Arc<Dns>, listener: tokio::net::TcpListener, cancel: tokio_util::sync::CancellationToken) {
    let slots = Arc::new(tokio::sync::Semaphore::new(256));
    loop {
        let (mut sock, peer) = tokio::select! {
            _ = cancel.cancelled() => return,
            r = listener.accept() => match r { Ok(v) => v, Err(_) => continue },
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
        let dns = dns.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(Duration::from_secs(15), async {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                for _ in 0..16 {
                    let mut len = [0u8; 2];
                    match tokio::time::timeout(Duration::from_secs(5), sock.read_exact(&mut len)).await {
                        Ok(Ok(_)) => {}
                        _ => return,
                    }
                    let len = u16::from_be_bytes(len) as usize;
                    if len == 0 || len > 1232 || !dns.allowed(&peer) {
                        return;
                    }
                    let mut msg = vec![0u8; len];
                    if !matches!(tokio::time::timeout(Duration::from_secs(5), sock.read_exact(&mut msg)).await, Ok(Ok(_))) {
                        return;
                    }
                    let Some(reply) = respond(&dns.zone, &dns.names, &msg, true, now()) else { return };
                    let mut framed = (reply.len() as u16).to_be_bytes().to_vec();
                    framed.extend_from_slice(&reply);
                    if sock.write_all(&framed).await.is_err() {
                        return;
                    }
                }
            })
            .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::store::Limits;
    use crate::namesproto::Identity;

    const T: i64 = 1_800_000_000;

    fn zone() -> Zone {
        Zone::new("lan.example.org", "ns1.example.org", "hostmaster.example.org").unwrap()
    }

    fn query(name: &str, qtype: u16, edns: Option<u16>) -> Vec<u8> {
        let mut q = vec![0xAB, 0xCD, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, edns.is_some() as u8];
        for l in name.split('.') {
            q.push(l.len() as u8);
            q.extend_from_slice(l.as_bytes());
        }
        q.push(0);
        q.extend_from_slice(&qtype.to_be_bytes());
        q.extend_from_slice(&1u16.to_be_bytes());
        if let Some(s) = edns {
            q.extend_from_slice(&[0, 0, 41]);
            q.extend_from_slice(&s.to_be_bytes());
            q.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        }
        q
    }

    fn rcode(r: &[u8]) -> u16 {
        u16::from_be_bytes([r[2], r[3]]) & 0xF
    }
    fn counts(r: &[u8]) -> (u16, u16, u16, u16) {
        let c = |i: usize| u16::from_be_bytes([r[i], r[i + 1]]);
        (c(4), c(6), c(8), c(10))
    }

    fn world() -> (Zone, Names, Identity, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let me = Identity::load_or_create(d.path()).unwrap();
        let names = Names::new(None, Limits::default());
        names.register(&me.register("192.168.1.20", T), T).unwrap();
        names.acme(&me.acme("set", &"x".repeat(43), T), T).unwrap();
        (zone(), names, me, d)
    }

    #[test]
    fn the_bridges_name_answers_with_its_address() {
        let (z, n, me, _d) = world();
        let r = respond(&z, &n, &query(&format!("{}.lan.example.org", me.id()), TYPE_A, None), false, T).unwrap();
        assert_eq!(rcode(&r), 0);
        assert_eq!(counts(&r), (1, 1, 0, 0));
        assert_ne!(r[2] & 0x04, 0, "authoritative");
        assert_eq!(&r[r.len() - 4..], &[192, 168, 1, 20]);
        assert_eq!(&r[..2], &[0xAB, 0xCD], "the query's ID comes back");
    }

    #[test]
    fn the_case_of_the_question_is_kept_and_matching_ignores_it() {
        let (z, n, me, _d) = world();
        let name = format!("{}.LAN.Example.ORG", me.id());
        let r = respond(&z, &n, &query(&name, TYPE_A, None), false, T).unwrap();
        assert_eq!(counts(&r).1, 1);
        assert!(r.windows(3).any(|w| w == b"LAN"), "0x20 randomization is echoed");
    }

    #[test]
    fn the_challenge_name_answers_with_the_txt_value_and_only_while_it_lives() {
        let (z, n, me, _d) = world();
        let q = query(&format!("_acme-challenge.{}.lan.example.org", me.id()), TYPE_TXT, None);
        let r = respond(&z, &n, &q, false, T).unwrap();
        assert_eq!((rcode(&r), counts(&r).1), (0, 1));
        assert!(r.windows(43).any(|w| w == "x".repeat(43).as_bytes()));
        let later = respond(&z, &n, &q, false, T + super::super::store::TXT_TTL_SECS).unwrap();
        assert_eq!((rcode(&later), counts(&later).1, counts(&later).2), (0, 0, 1), "no data, with the SOA");
    }

    #[test]
    fn unknown_names_do_not_exist_and_other_zones_are_refused() {
        let (z, n, _me, _d) = world();
        let unknown = respond(&z, &n, &query(&format!("{}.lan.example.org", "0".repeat(32)), TYPE_A, None), false, T).unwrap();
        assert_eq!((rcode(&unknown), counts(&unknown).2), (RCODE_NXDOMAIN, 1), "NXDOMAIN with the SOA, so it can be cached");
        let other = respond(&z, &n, &query("anything.lan.example.org", TYPE_A, None), false, T).unwrap();
        assert_eq!(rcode(&other), RCODE_NXDOMAIN);
        let outside = respond(&z, &n, &query("www.example.com", TYPE_A, None), false, T).unwrap();
        assert_eq!((rcode(&outside), counts(&outside).1), (RCODE_REFUSED, 0), "never recurse");
        assert_eq!(outside[2] & 0x04, 0, "and not authoritative there");
        let sibling = respond(&z, &n, &query("lan.example.org.evil.test", TYPE_A, None), false, T).unwrap();
        assert_eq!(rcode(&sibling), RCODE_REFUSED, "the zone name must be the end of the name");
    }

    #[test]
    fn the_apex_has_ns_and_soa_and_a_known_name_has_nothing_of_other_types() {
        let (z, n, me, _d) = world();
        let ns = respond(&z, &n, &query("lan.example.org", TYPE_NS, None), false, T).unwrap();
        assert_eq!((rcode(&ns), counts(&ns).1), (0, 1));
        let soa = respond(&z, &n, &query("lan.example.org", TYPE_SOA, None), false, T).unwrap();
        assert_eq!((rcode(&soa), counts(&soa).1), (0, 1));
        let caa = respond(&z, &n, &query(&format!("{}.lan.example.org", me.id()), 257, None), false, T).unwrap();
        assert_eq!((rcode(&caa), counts(&caa).1, counts(&caa).2), (0, 0, 1), "NODATA, not NXDOMAIN: the name exists");
        let any = respond(&z, &n, &query(&format!("{}.lan.example.org", me.id()), 255, None), false, T).unwrap();
        assert_eq!(counts(&any).1, 0, "ANY lists nothing");
        let axfr = respond(&z, &n, &query("lan.example.org", TYPE_AXFR, None), true, T).unwrap();
        assert_eq!(rcode(&axfr), RCODE_REFUSED, "no zone transfers");
    }

    #[test]
    fn a_service_without_an_address_has_a_txt_name_but_no_a_record() {
        let d = tempfile::tempdir().unwrap();
        let svc = Identity::load_or_create(d.path()).unwrap();
        let n = Names::new(None, Limits::default());
        n.add_local(svc.id(), svc.public_key(), T);
        let r = respond(&zone(), &n, &query(&format!("{}.lan.example.org", svc.id()), TYPE_A, None), false, T).unwrap();
        assert_eq!((rcode(&r), counts(&r).1), (0, 0));
    }

    #[test]
    fn malformed_queries_get_an_error_or_silence_never_a_crash() {
        let (z, n, _me, _d) = world();
        let good = query("lan.example.org", TYPE_SOA, None);
        // every cut-off of a good query
        for cut in 0..good.len() {
            let _ = respond(&z, &n, &good[..cut], false, T);
        }
        // a response is never answered
        let mut resp = good.clone();
        resp[2] |= 0x80;
        assert!(respond(&z, &n, &resp, false, T).is_none());
        // a compression pointer in the question
        let mut ptr = good.clone();
        ptr[12] = 0xC0;
        assert_eq!(rcode(&respond(&z, &n, &ptr, false, T).unwrap()), RCODE_FORMERR);
        // a 64-byte "label" (length bits 01 are an extended label type, not a length)
        let mut ext = good[..12].to_vec();
        ext.push(0x40);
        ext.extend(std::iter::repeat_n(b'a', 64));
        ext.extend_from_slice(&[0, 0, 1, 0, 1]);
        assert_eq!(rcode(&respond(&z, &n, &ext, false, T).unwrap()), RCODE_FORMERR);
        // two questions
        let mut two = good.clone();
        two[5] = 2;
        assert_eq!(rcode(&respond(&z, &n, &two, false, T).unwrap()), RCODE_FORMERR);
        // trailing bytes
        let mut trail = good.clone();
        trail.push(0);
        assert_eq!(rcode(&respond(&z, &n, &trail, false, T).unwrap()), RCODE_FORMERR);
        // not a query
        let mut op = good.clone();
        op[2] |= 0x28; // opcode 5 (UPDATE)
        assert_eq!(rcode(&respond(&z, &n, &op, false, T).unwrap()), RCODE_NOTIMP);
        // a name of 300 bytes
        let long = vec!["a".repeat(60); 6].join(".");
        assert_eq!(rcode(&respond(&z, &n, &query(&long, TYPE_A, None), false, T).unwrap()), RCODE_FORMERR);
        // random bytes
        let mut x = 0x1234_5678u32;
        for _ in 0..2000 {
            let len = (x % 80) as usize;
            let junk: Vec<u8> = (0..len).map(|_| { x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (x >> 24) as u8 }).collect();
            let _ = respond(&z, &n, &junk, false, T);
        }
    }

    #[test]
    fn a_reply_is_never_larger_than_the_query_allows_over_udp() {
        let (z, n, me, _d) = world();
        for qtype in [TYPE_A, TYPE_TXT, TYPE_SOA, TYPE_NS, 255, 1] {
            for name in ["lan.example.org".to_string(), format!("{}.lan.example.org", me.id()), format!("_acme-challenge.{}.lan.example.org", me.id())] {
                let q = query(&name, qtype, None);
                let r = respond(&z, &n, &q, false, T).unwrap();
                assert!(r.len() <= 512);
                // the reply is not an order of magnitude bigger than the question: no amplification
                assert!(r.len() <= q.len() * 4, "{name} type {qtype}: {} bytes for a {} byte query", r.len(), q.len());
            }
        }
    }

    #[tokio::test]
    async fn over_real_udp_and_tcp() {
        let (z, n, me, _d) = world();
        let dns = Arc::new(Dns { zone: z, names: Arc::new(n), limiter: crate::relay::Limiter::new(1000, Duration::from_secs(1)) });
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (ua, ta) = (udp.local_addr().unwrap(), tcp.local_addr().unwrap());
        let cancel = tokio_util::sync::CancellationToken::new();
        tokio::spawn(serve_udp(dns.clone(), udp, cancel.clone()));
        tokio::spawn(serve_tcp(dns, tcp, cancel.clone()));
        let q = query(&format!("{}.lan.example.org", me.id()), TYPE_A, None);
        let c = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        c.send_to(&q, ua).await.unwrap();
        let mut buf = [0u8; 600];
        let (n1, _) = tokio::time::timeout(Duration::from_secs(2), c.recv_from(&mut buf)).await.unwrap().unwrap();
        assert_eq!(&buf[n1 - 4..n1], &[192, 168, 1, 20]);
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(ta).await.unwrap();
        let mut framed = (q.len() as u16).to_be_bytes().to_vec();
        framed.extend_from_slice(&q);
        s.write_all(&framed).await.unwrap();
        let mut len = [0u8; 2];
        s.read_exact(&mut len).await.unwrap();
        let mut reply = vec![0u8; u16::from_be_bytes(len) as usize];
        s.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[reply.len() - 4..], &[192, 168, 1, 20]);
        cancel.cancel();
    }
}
