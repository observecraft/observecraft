//! NetBird's peers (`GET /api/peers`) as metric points, one series per peer and metric, each poll a point at the
//! poll's minute. Peers aren't hosts here (no `host`): a laptop joining the network is not infrastructure. Tags:
//! `netbird_instance` (the management server's host), `peer` (its name), `peer_id`, `os`, `netbird_version` (the
//! client's), `country_code` and `city` when NetBird located it.
//!
//! - `netbird.peer.count`: 1 per peer (summed: the peers, by OS, client version, country).
//! - `netbird.peer.connected`: 1 while the peer is connected to the management server, else 0.
//! - `netbird.peer.last_seen_age`: seconds since it was last seen (0 while connected).
//! - `netbird.peer.login_expired`, `netbird.peer.approval_required`, `netbird.peer.ssh_enabled`: 1 or 0.
//! - `netbird.peer.accessible_peers`: the peers its policies let it reach.

use serde_json::Value;

use crate::cloud::naming::Point;

/// The metrics a peer reports, by name.
pub const METRICS: [&str; 7] = ["netbird.peer.count", "netbird.peer.connected", "netbird.peer.last_seen_age", "netbird.peer.login_expired", "netbird.peer.approval_required", "netbird.peer.ssh_enabled", "netbird.peer.accessible_peers"];

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn flag(v: &Value, key: &str) -> f64 {
    if v.get(key).and_then(Value::as_bool).unwrap_or(false) {
        1.0
    } else {
        0.0
    }
}

/// The points of every peer at `time_ms` (the poll's minute); `instance`: the management server's host.
pub fn points_of(peers: &[Value], instance: &str, time_ms: i64) -> Vec<Point> {
    let mut out = Vec::with_capacity(peers.len() * METRICS.len());
    for peer in peers {
        let Some(id) = text(peer, "id") else { continue };
        let name = text(peer, "name").or_else(|| text(peer, "hostname")).unwrap_or(id);
        let mut tags: Vec<(String, String)> = vec![("netbird_instance".into(), instance.into()), ("peer".into(), name.into()), ("peer_id".into(), id.into())];
        for (tag, field) in [("os", "os"), ("netbird_version", "version"), ("country_code", "country_code"), ("city", "city_name")] {
            if let Some(v) = text(peer, field) {
                tags.push((tag.into(), v.into()));
            }
        }
        tags.sort();
        let connected = flag(peer, "connected");
        let last_seen_age = if connected > 0.0 {
            0.0
        } else {
            text(peer, "last_seen").and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map_or(0.0, |t| ((time_ms - t.timestamp_millis()).max(0) / 1000) as f64)
        };
        let values = [
            1.0,
            connected,
            last_seen_age,
            flag(peer, "login_expired"),
            flag(peer, "approval_required"),
            flag(peer, "ssh_enabled"),
            peer.get("accessible_peers_count").and_then(Value::as_f64).unwrap_or(0.0),
        ];
        for (name, value) in METRICS.iter().zip(values) {
            out.push(Point { name: name.to_string(), host: None, tags: tags.clone(), time_ms, value });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netbird::fixtures;

    fn value(points: &[Point], peer: &str, metric: &str) -> f64 {
        points.iter().find(|p| p.name == metric && p.tags.iter().any(|(k, v)| k == "peer" && v == peer)).unwrap().value
    }

    #[test]
    fn every_peer_reports_its_state_with_its_tags() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-06T12:00:00Z").unwrap().timestamp_millis();
        let points = points_of(&fixtures::peers(), "netbird.example.com", now);
        assert_eq!(points.len(), 3 * METRICS.len());
        assert_eq!(value(&points, "laptop-jordan", "netbird.peer.connected"), 1.0);
        assert_eq!(value(&points, "laptop-jordan", "netbird.peer.last_seen_age"), 0.0, "connected: seen now");
        assert_eq!(value(&points, "laptop-jordan", "netbird.peer.accessible_peers"), 2.0);
        assert_eq!(value(&points, "build-runner-1", "netbird.peer.connected"), 0.0);
        assert_eq!(value(&points, "build-runner-1", "netbird.peer.last_seen_age"), 7200.0);
        assert_eq!(value(&points, "build-runner-1", "netbird.peer.login_expired"), 1.0);
        assert_eq!(value(&points, "i-0a1b2c3d4e5f60001", "netbird.peer.ssh_enabled"), 1.0);
        let laptop = &points.iter().find(|p| p.tags.iter().any(|(_, v)| v == "laptop-jordan")).unwrap().tags;
        let expected: Vec<(String, String)> = [
            ("city", "Bucharest"),
            ("country_code", "RO"),
            ("netbird_instance", "netbird.example.com"),
            ("netbird_version", "0.59.1"),
            ("os", "Darwin 15.1"),
            ("peer", "laptop-jordan"),
            ("peer_id", "cs1tnh0hhcjnqoiuebf0"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(laptop, &expected);
        assert!(points.iter().all(|p| p.host.is_none() && p.time_ms == now));
    }

    #[test]
    fn peers_without_an_id_are_skipped_and_missing_fields_are_zero() {
        let points = points_of(&[serde_json::json!({ "name": "ghost" }), serde_json::json!({ "id": "p1" })], "netbird.example.com", 0);
        assert_eq!(points.len(), METRICS.len());
        assert!(points.iter().all(|p| p.value == if p.name == "netbird.peer.count" { 1.0 } else { 0.0 }));
        assert!(points[0].tags.contains(&("peer".into(), "p1".into())), "named by its id when it has no name");
    }
}
