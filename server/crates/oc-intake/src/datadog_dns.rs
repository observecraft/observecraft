//! The Datadog Agent's DNS resolution tables in `CollectorConnections` (which IPs the host resolved
//! from which domain names): `encodedDNS` (version 1) and `encodedDnsLookups` +
//! `encodedDomainDatabase` (version 2).
//!
//! Both are bucketed hash tables keyed by IP (layout from `DataDog/agent-payload`,
//! `process/dns_v1.go` and `process/dns_v2.go`, BSD-3-Clause, Copyright (c) 2017 Datadog, Inc.).
//! The tables are walked bucket by bucket rather than looked up by hash, so every entry is read
//! without the Agent's hash function.

use std::collections::HashMap;

/// Reads one unsigned LEB128 varint (Go's `binary.Uvarint`).
fn varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// A length-prefixed string at `offset`.
fn name_at(buf: &[u8], offset: usize) -> Option<String> {
    let mut pos = offset;
    let len = varint(buf, &mut pos)? as usize;
    let end = pos.checked_add(len)?;
    if len == 0 || end > buf.len() {
        return None;
    }
    String::from_utf8(buf[pos..end].to_vec()).ok()
}

/// Every bucket entry: (ip, name offsets), starting at the bucket block.
fn entries(buf: &[u8], mut pos: usize, buckets: usize) -> Option<Vec<(String, Vec<usize>)>> {
    let mut out = Vec::new();
    for _ in 0..buckets {
        let n = varint(buf, &mut pos)?;
        for _ in 0..n {
            let ip_len = varint(buf, &mut pos)? as usize;
            let end = pos.checked_add(ip_len)?;
            let ip = String::from_utf8(buf.get(pos..end)?.to_vec()).ok()?;
            pos = end;
            let names = varint(buf, &mut pos)?;
            let offsets = (0..names).map(|_| varint(buf, &mut pos).map(|o| o as usize)).collect::<Option<Vec<_>>>()?;
            out.push((ip, offsets));
        }
    }
    Some(out)
}

/// `encodedDNS` (version 1): names live in a block at the end of the buffer.
pub fn decode_v1(buf: &[u8]) -> HashMap<String, Vec<String>> {
    let parse = || -> Option<HashMap<String, Vec<String>>> {
        if *buf.first()? != 1 || buf.len() < 3 {
            return None;
        }
        let buckets = u16::from_le_bytes([buf[1], buf[2]]) as usize;
        let mut pos = 3;
        let position_len = varint(buf, &mut pos)? as usize;
        let name_len = varint(buf, &mut pos)? as usize;
        let _middle = varint(buf, &mut pos)?;
        let names = buf.get(buf.len().checked_sub(name_len)?..)?;
        let list = entries(buf, pos.checked_add(position_len)?, buckets)?;
        Some(list.into_iter().map(|(ip, offs)| (ip, offs.into_iter().filter_map(|o| name_at(names, o)).collect())).collect())
    };
    parse().unwrap_or_default()
}

/// `encodedDnsLookups` (version 2), whose name offsets point into `encodedDomainDatabase`.
pub fn decode_v2(lookups: &[u8], domains: &[u8]) -> HashMap<String, Vec<String>> {
    let parse = || -> Option<HashMap<String, Vec<String>>> {
        if *lookups.first()? != 2 || lookups.len() < 3 {
            return None;
        }
        let buckets = u16::from_le_bytes([lookups[1], lookups[2]]) as usize;
        let mut pos = 3;
        let position_len = varint(lookups, &mut pos)? as usize;
        let _middle = varint(lookups, &mut pos)?;
        let list = entries(lookups, pos.checked_add(position_len)?, buckets)?;
        Some(list.into_iter().map(|(ip, offs)| (ip, offs.into_iter().filter_map(|o| name_at(domains, o)).collect())).collect())
    };
    parse().unwrap_or_default()
}

/// The domain at a byte offset of `encodedDomainDatabase` (keys of `dnsStatsByDomainOffsetByQueryType`).
pub fn domain_at(domains: &[u8], offset: usize) -> Option<String> {
    name_at(domains, offset)
}

/// DNS query type names (RFC 1035 and successors) for the common types.
pub fn query_type(t: i32) -> String {
    match t {
        1 => "A".into(),
        2 => "NS".into(),
        5 => "CNAME".into(),
        6 => "SOA".into(),
        12 => "PTR".into(),
        15 => "MX".into(),
        16 => "TXT".into(),
        28 => "AAAA".into(),
        33 => "SRV".into(),
        65 => "HTTPS".into(),
        other => format!("TYPE{other}"),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn put(buf: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                buf.push(b);
                return;
            }
            buf.push(b | 0x80);
        }
    }

    /// Domain database and lookups for `entries` (ip → names), one bucket per entry, laid out as
    /// the Agent's V2 encoder does.
    pub fn encode_v2(entries: &[(&str, &[&str])]) -> (Vec<u8>, Vec<u8>) {
        let mut names: Vec<&str> = entries.iter().flat_map(|(_, n)| n.iter().copied()).collect();
        names.dedup();
        let mut db = Vec::new();
        put(&mut db, names.len() as u64);
        put(&mut db, 0);
        let mut offsets = HashMap::new();
        for n in &names {
            offsets.insert(*n, db.len() as u64);
            put(&mut db, n.len() as u64);
            db.extend_from_slice(n.as_bytes());
        }
        let mut buckets: Vec<Vec<u8>> = Vec::new();
        for (ip, ns) in entries {
            let mut b = Vec::new();
            put(&mut b, 1);
            put(&mut b, ip.len() as u64);
            b.extend_from_slice(ip.as_bytes());
            put(&mut b, ns.len() as u64);
            for n in *ns {
                put(&mut b, offsets[n]);
            }
            buckets.push(b);
        }
        let mut positions = Vec::new();
        let mut at = 0u64;
        for b in &buckets {
            put(&mut positions, at);
            at += b.len() as u64;
        }
        let mut out = vec![2u8];
        out.extend_from_slice(&(buckets.len() as u16).to_le_bytes());
        put(&mut out, positions.len() as u64);
        put(&mut out, 0);
        out.extend_from_slice(&positions);
        for b in buckets {
            out.extend_from_slice(&b);
        }
        (out, db)
    }

    #[test]
    fn v2_round_trip() {
        let (lookups, db) = encode_v2(&[("203.0.113.10", &["payments.example.net"]), ("198.51.100.25", &["smtp.example.net", "mail.example.net"])]);
        let m = decode_v2(&lookups, &db);
        assert_eq!(m["203.0.113.10"], ["payments.example.net"]);
        assert_eq!(m["198.51.100.25"], ["smtp.example.net", "mail.example.net"]);
        assert_eq!(domain_at(&db, 2).as_deref(), Some("payments.example.net"));
    }

    #[test]
    fn v1_layout() {
        // One bucket with one entry: 10.0.0.1 → "a.example" (offset 0 in the name block).
        let mut names = Vec::new();
        put(&mut names, 9);
        names.extend_from_slice(b"a.example");
        let mut bucket = Vec::new();
        put(&mut bucket, 1);
        put(&mut bucket, 8);
        bucket.extend_from_slice(b"10.0.0.1");
        put(&mut bucket, 1);
        put(&mut bucket, 0);
        let mut buf = vec![1u8, 1, 0];
        put(&mut buf, 1); // position block length
        put(&mut buf, names.len() as u64);
        put(&mut buf, 0); // middle
        put(&mut buf, 0); // position of bucket 0
        buf.extend_from_slice(&bucket);
        buf.extend_from_slice(&names);
        assert_eq!(decode_v1(&buf)["10.0.0.1"], ["a.example"]);
    }

    #[test]
    fn garbage_is_empty() {
        assert!(decode_v2(&[2, 5, 0, 200], &[]).is_empty());
        assert!(decode_v1(&[9]).is_empty());
    }
}
