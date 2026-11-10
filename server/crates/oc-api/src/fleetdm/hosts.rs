//! Fleet's hosts, host counts and policies as metric points, each poll a point at the poll's minute. Fleet's hosts
//! aren't `host`s here: most are laptops and desktops, and a server that also runs the Datadog Agent keeps its own
//! host. Every series has `fleet_instance` (the Fleet server's host name).
//!
//! Per host (`GET /hosts`), tagged `fleet_host` (its display name), `fleet_host_id`, `platform`, `os_version`,
//! `osquery_version`, `team` (Fleet's team or fleet), `hardware_model`, `mdm_status` (Fleet's MDM enrollment status:
//! `On (automatic)`, `On (manual)`, `Pending`, `Off`), and `country_code` / `city` (Fleet's GeoIP location of the host)
//! when Fleet knows them:
//!
//! - `fleetdm.host.count`: 1, so a sum counts the hosts of any tag (OS version, team, …).
//! - `fleetdm.host.online`, `.offline`, `.missing`: 1 for the host's status (Fleet's `online`, `offline`, `missing`
//!   or `mia`), else 0, so a sum counts the hosts of a status; `.new` hosts are online or offline too.
//! - `fleetdm.host.seen_age`: seconds since the host last checked in.
//! - `fleetdm.host.uptime` (seconds), `fleetdm.host.memory` (bytes).
//! - `fleetdm.host.disk.available`, `.disk.total` (bytes), `.disk.available_percent`, when osquery reported them.
//! - `fleetdm.host.failing_policies`, `.critical_vulnerabilities` (Fleet Premium), `.issues`.
//! - `fleetdm.host.mdm_enrolled` (1 or 0, for hosts with an MDM status), `fleetdm.host.disk_encrypted` (1 or 0, when
//!   Fleet knows it).
//!
//! The counts (`GET /host_summary`): `fleetdm.hosts.total`, `.online`, `.offline`, `.mia`, `.missing_30_days`, `.new`,
//! `.low_disk_space`, and `fleetdm.hosts.by_platform` tagged `platform`.
//!
//! Per global policy (`GET /global/policies`), tagged `policy`, `policy_id`, `critical` and `platform`:
//! `fleetdm.policy.passing_hosts`, `fleetdm.policy.failing_hosts` (as Fleet last counted them).
//!
//! Vulnerabilities (`GET /vulnerabilities`, the [`super::api::VULNERABILITIES`] on the most hosts):
//! `fleetdm.vulnerabilities.total` (every CVE Fleet found on the hosts) and `fleetdm.vulnerability.hosts` per CVE,
//! tagged `cve`, `severity` (from its CVSS score: critical, high, medium, low; `unknown` without Fleet Premium's
//! scores) and `known_exploited` (CISA's catalog, Premium).

use serde_json::Value;

use crate::cloud::naming::Point;

/// Bytes per Fleet "gig" (osquery's bytes × 10⁻⁹).
const GIG: f64 = 1e9;

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn point(name: &str, tags: &[(String, String)], time_ms: i64, value: f64) -> Point {
    Point { name: name.to_string(), host: None, tags: tags.to_vec(), time_ms, value }
}

fn sorted(mut tags: Vec<(String, String)>) -> Vec<(String, String)> {
    tags.sort();
    tags
}

/// The points of every host at `time_ms` (the poll's minute); `instance`: the Fleet server's host.
pub fn host_points(hosts: &[Value], instance: &str, time_ms: i64) -> Vec<Point> {
    let mut out = Vec::with_capacity(hosts.len() * 14);
    for host in hosts {
        let Some(id) = host.get("id").and_then(Value::as_u64) else { continue };
        let name = text(host, "display_name").or_else(|| text(host, "computer_name")).or_else(|| text(host, "hostname")).map_or_else(|| id.to_string(), str::to_string);
        let mut tags: Vec<(String, String)> = vec![("fleet_instance".into(), instance.into()), ("fleet_host".into(), name), ("fleet_host_id".into(), id.to_string())];
        for (tag, field) in [("platform", "platform"), ("os_version", "os_version"), ("osquery_version", "osquery_version"), ("hardware_model", "hardware_model")] {
            if let Some(v) = text(host, field) {
                tags.push((tag.into(), v.into()));
            }
        }
        if let Some(team) = text(host, "fleet_name").or_else(|| text(host, "team_name")) {
            tags.push(("team".into(), team.into()));
        }
        if let Some(mdm) = host.pointer("/mdm/enrollment_status").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            tags.push(("mdm_status".into(), mdm.into()));
        }
        // Where Fleet located the host (its GeoIP database): the dashboard's world map.
        let geo = host.get("geolocation").cloned().unwrap_or(Value::Null);
        if let Some(iso) = text(&geo, "country_iso").filter(|c| c.len() == 2) {
            tags.push(("country_code".into(), iso.to_ascii_uppercase()));
        }
        if let Some(city) = text(&geo, "city_name") {
            tags.push(("city".into(), city.into()));
        }
        let tags = sorted(tags);
        let status = text(host, "status").unwrap_or_default();
        let flag = |on: bool| if on { 1.0 } else { 0.0 };
        let mut add = |name: &str, value: f64| out.push(point(name, &tags, time_ms, value));
        add("fleetdm.host.count", 1.0);
        add("fleetdm.host.online", flag(status == "online"));
        add("fleetdm.host.offline", flag(status == "offline"));
        add("fleetdm.host.missing", flag(status == "missing" || status == "mia"));
        if let Some(seen) = text(host, "seen_time").and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) {
            add("fleetdm.host.seen_age", ((time_ms - seen.timestamp_millis()).max(0) / 1000) as f64);
        }
        // Fleet answers the uptime in nanoseconds.
        if let Some(ns) = num(host, "uptime").filter(|n| *n > 0.0) {
            add("fleetdm.host.uptime", (ns / 1e9).floor());
        }
        if let Some(bytes) = num(host, "memory").filter(|n| *n > 0.0) {
            add("fleetdm.host.memory", bytes);
        }
        // A host osquery hasn't reported its disks for yet answers zeros.
        if let Some(total) = num(host, "gigs_total_disk_space").filter(|n| *n > 0.0) {
            add("fleetdm.host.disk.total", total * GIG);
            if let Some(free) = num(host, "gigs_disk_space_available") {
                add("fleetdm.host.disk.available", free * GIG);
            }
            if let Some(pct) = num(host, "percent_disk_space_available") {
                add("fleetdm.host.disk.available_percent", pct);
            }
        }
        let issues = host.get("issues").cloned().unwrap_or(Value::Null);
        add("fleetdm.host.failing_policies", num(&issues, "failing_policies_count").unwrap_or(0.0));
        add("fleetdm.host.critical_vulnerabilities", num(&issues, "critical_vulnerabilities_count").unwrap_or(0.0));
        add("fleetdm.host.issues", num(&issues, "total_issues_count").unwrap_or(0.0));
        if let Some(enrollment) = host.pointer("/mdm/enrollment_status").and_then(Value::as_str) {
            add("fleetdm.host.mdm_enrolled", flag(enrollment.starts_with("On")));
        }
        if let Some(encrypted) = host.get("disk_encryption_enabled").and_then(Value::as_bool) {
            add("fleetdm.host.disk_encrypted", flag(encrypted));
        }
    }
    out
}

/// The host counts of `GET /host_summary`.
pub fn summary_points(summary: &Value, instance: &str, time_ms: i64) -> Vec<Point> {
    let tags = vec![("fleet_instance".to_string(), instance.to_string())];
    let mut out: Vec<Point> = [
        ("fleetdm.hosts.total", "totals_hosts_count"),
        ("fleetdm.hosts.online", "online_count"),
        ("fleetdm.hosts.offline", "offline_count"),
        ("fleetdm.hosts.mia", "mia_count"),
        ("fleetdm.hosts.missing_30_days", "missing_30_days_count"),
        ("fleetdm.hosts.new", "new_count"),
        ("fleetdm.hosts.low_disk_space", "low_disk_space_count"),
    ]
    .iter()
    .filter_map(|(name, field)| num(summary, field).map(|v| point(name, &tags, time_ms, v)))
    .collect();
    for p in summary["platforms"].as_array().into_iter().flatten() {
        if let (Some(platform), Some(count)) = (text(p, "platform"), num(p, "hosts_count")) {
            out.push(point("fleetdm.hosts.by_platform", &sorted(vec![tags[0].clone(), ("platform".into(), platform.into())]), time_ms, count));
        }
    }
    out
}

/// The global policies' passing and failing host counts.
pub fn policy_points(policies: &Value, instance: &str, time_ms: i64) -> Vec<Point> {
    let mut out = Vec::new();
    for p in policies["policies"].as_array().into_iter().flatten() {
        let (Some(id), Some(name)) = (p.get("id").and_then(Value::as_u64), text(p, "name")) else { continue };
        let mut tags = vec![
            ("fleet_instance".to_string(), instance.to_string()),
            ("policy".into(), name.into()),
            ("policy_id".into(), id.to_string()),
            ("critical".into(), p["critical"].as_bool().unwrap_or(false).to_string()),
        ];
        if let Some(platform) = text(p, "platform") {
            tags.push(("platform".into(), platform.into()));
        }
        let tags = sorted(tags);
        out.push(point("fleetdm.policy.passing_hosts", &tags, time_ms, num(p, "passing_host_count").unwrap_or(0.0)));
        out.push(point("fleetdm.policy.failing_hosts", &tags, time_ms, num(p, "failing_host_count").unwrap_or(0.0)));
    }
    out
}

/// A CVSS score's severity, as NVD rates it.
fn severity(cvss: Option<f64>) -> &'static str {
    match cvss {
        Some(s) if s >= 9.0 => "critical",
        Some(s) if s >= 7.0 => "high",
        Some(s) if s >= 4.0 => "medium",
        Some(s) if s > 0.0 => "low",
        _ => "unknown",
    }
}

/// The vulnerabilities of `GET /vulnerabilities`: the total and the hosts of each listed CVE.
pub fn vulnerability_points(answer: &Value, instance: &str, time_ms: i64) -> Vec<Point> {
    let base = ("fleet_instance".to_string(), instance.to_string());
    let mut out: Vec<Point> = num(answer, "count").map(|n| point("fleetdm.vulnerabilities.total", std::slice::from_ref(&base), time_ms, n)).into_iter().collect();
    for v in answer["vulnerabilities"].as_array().into_iter().flatten() {
        let Some(cve) = text(v, "cve") else { continue };
        let mut tags = vec![base.clone(), ("cve".into(), cve.into()), ("severity".into(), severity(num(v, "cvss_score")).into())];
        if let Some(known) = v.get("cisa_known_exploit").and_then(Value::as_bool) {
            tags.push(("known_exploited".into(), known.to_string()));
        }
        out.push(point("fleetdm.vulnerability.hosts", &sorted(tags), time_ms, num(v, "hosts_count").unwrap_or(0.0)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleetdm::fixtures;

    const NOW: &str = "2026-11-06T12:00:00Z";

    fn now() -> i64 {
        chrono::DateTime::parse_from_rfc3339(NOW).unwrap().timestamp_millis()
    }

    fn value(points: &[Point], host: &str, metric: &str) -> Option<f64> {
        points.iter().find(|p| p.name == metric && p.tags.iter().any(|(k, v)| k == "fleet_host" && v == host)).map(|p| p.value)
    }

    #[test]
    fn every_host_reports_its_state_with_its_tags() {
        let points = host_points(&fixtures::hosts(), "fleet.example.com", now());
        let mbp = "Jordan's MacBook Pro";
        assert_eq!(value(&points, mbp, "fleetdm.host.count"), Some(1.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.online"), Some(1.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.missing"), Some(0.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.seen_age"), Some(10.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.uptime"), Some(86_400.0), "nanoseconds to seconds");
        assert_eq!(value(&points, mbp, "fleetdm.host.memory"), Some(34_359_738_368.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.disk.available"), Some(120.5e9));
        assert_eq!(value(&points, mbp, "fleetdm.host.disk.available_percent"), Some(24.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.failing_policies"), Some(1.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.critical_vulnerabilities"), Some(2.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.mdm_enrolled"), Some(1.0));
        assert_eq!(value(&points, mbp, "fleetdm.host.disk_encrypted"), Some(1.0));
        let tags = &points.iter().find(|p| p.tags.iter().any(|(_, v)| v == mbp)).unwrap().tags;
        let expected: Vec<(String, String)> = [
            ("city", "Bucharest"),
            ("country_code", "RO"),
            ("fleet_host", mbp),
            ("fleet_host_id", "1"),
            ("fleet_instance", "fleet.example.com"),
            ("hardware_model", "MacBookPro18,3"),
            ("mdm_status", "On (automatic)"),
            ("os_version", "macOS 15.1"),
            ("osquery_version", "5.14.1"),
            ("platform", "darwin"),
            ("team", "Workstations"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(tags, &expected);
        assert!(points.iter().all(|p| p.host.is_none() && p.time_ms == now()));

        // Offline: no MDM status (Linux), no encryption status known.
        let server = "i-0a1b2c3d4e5f60001";
        assert_eq!((value(&points, server, "fleetdm.host.offline"), value(&points, server, "fleetdm.host.seen_age")), (Some(1.0), Some(7200.0)));
        assert_eq!((value(&points, server, "fleetdm.host.mdm_enrolled"), value(&points, server, "fleetdm.host.disk_encrypted")), (None, None));
        // Missing, with its fleet's name as the team, and no disk reported.
        assert_eq!((value(&points, "desktop-sam", "fleetdm.host.missing"), value(&points, "desktop-sam", "fleetdm.host.mdm_enrolled")), (Some(1.0), Some(0.0)));
        assert_eq!(value(&points, "desktop-sam", "fleetdm.host.disk.total"), None);
        assert!(points.iter().any(|p| p.tags.contains(&("team".into(), "Office".into()))));
        assert!(!points.iter().any(|p| p.tags.iter().any(|(k, _)| k == "country_code") && p.tags.contains(&("fleet_host".into(), "desktop-sam".into()))), "no location, no country");
    }

    #[test]
    fn hosts_without_an_id_are_skipped_and_named_by_their_id_otherwise() {
        let points = host_points(&[serde_json::json!({ "hostname": "ghost" }), serde_json::json!({ "id": 9 })], "fleet.example.com", now());
        assert!(points.iter().all(|p| p.tags.contains(&("fleet_host".into(), "9".into()))));
        assert_eq!(points.iter().filter(|p| p.name == "fleetdm.host.online").count(), 1);
    }

    #[test]
    fn counts_and_policies() {
        let summary = summary_points(&fixtures::host_summary(), "fleet.example.com", now());
        let get = |name: &str| summary.iter().find(|p| p.name == name).map(|p| p.value);
        assert_eq!((get("fleetdm.hosts.total"), get("fleetdm.hosts.online"), get("fleetdm.hosts.mia")), (Some(3.0), Some(1.0), Some(1.0)));
        assert_eq!(summary.iter().filter(|p| p.name == "fleetdm.hosts.by_platform").count(), 3);
        let policies = policy_points(&fixtures::policies(), "fleet.example.com", now());
        assert_eq!(policies.len(), 4);
        let failing = policies.iter().find(|p| p.name == "fleetdm.policy.failing_hosts" && p.tags.contains(&("policy_id".into(), "12".into()))).unwrap();
        assert_eq!(failing.value, 1.0);
        assert!(failing.tags.contains(&("critical".into(), "true".into())) && failing.tags.contains(&("policy".into(), "Disk encryption enabled".into())));
        assert!(!policies.iter().any(|p| p.tags.iter().any(|(k, v)| k == "platform" && v.is_empty())), "an empty platform is no tag");
    }

    #[test]
    fn vulnerabilities_by_hosts_with_their_severity() {
        let points = vulnerability_points(&fixtures::vulnerabilities(), "fleet.example.com", now());
        assert_eq!((points[0].name.as_str(), points[0].value), ("fleetdm.vulnerabilities.total", 123.0));
        let cve = |id: &str| points.iter().find(|p| p.tags.contains(&("cve".into(), id.into()))).unwrap();
        let xz = cve("CVE-2024-3094");
        assert_eq!((xz.name.as_str(), xz.value), ("fleetdm.vulnerability.hosts", 7.0));
        assert!(xz.tags.contains(&("severity".into(), "critical".into())) && xz.tags.contains(&("known_exploited".into(), "true".into())));
        assert!(cve("CVE-2023-4863").tags.contains(&("severity".into(), "high".into())));
        let free = cve("CVE-2022-30190");
        assert!(free.tags.contains(&("severity".into(), "unknown".into())) && !free.tags.iter().any(|(k, _)| k == "known_exploited"), "Fleet Free has no scores");
        assert_eq!(vulnerability_points(&serde_json::json!({ "vulnerabilities": null, "count": 0 }), "fleet.example.com", now()).len(), 1);
    }
}
