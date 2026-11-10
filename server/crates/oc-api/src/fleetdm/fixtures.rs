//! Fleet API answers (fictional `example.com` users and hosts) for the FleetDM integration's tests.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};

use super::api::{FleetApi, FleetError, ACTIVITIES_PAGE_SIZE};

/// An activity as `GET /api/v1/fleet/activities` lists it.
pub fn activity(id: u64, kind: &str, created_at: &str, details: Value) -> Value {
    json!({
        "created_at": created_at,
        "id": id,
        "actor_full_name": "Jordan Patel",
        "actor_id": 3,
        "actor_gravatar": "",
        "actor_email": "jordan.patel@example.com",
        "type": kind,
        "fleet_initiated": false,
        "details": details
    })
}

/// A policy created by a user.
pub fn created_policy(id: u64, created_at: &str) -> Value {
    activity(id, "created_policy", created_at, json!({ "policy_id": 12, "policy_name": "Disk encryption enabled" }))
}

/// A page of activities.
pub fn page(activities: Vec<Value>, has_next: bool) -> Value {
    json!({ "activities": activities, "meta": { "has_next_results": has_next, "has_previous_results": false } })
}

/// Three hosts: an online laptop enrolled in MDM with a failing policy, an offline Linux server, a missing desktop.
pub fn hosts() -> Vec<Value> {
    vec![
        json!({
            "id": 1, "hostname": "jordans-mbp.local", "display_name": "Jordan's MacBook Pro", "computer_name": "Jordan's MacBook Pro",
            "platform": "darwin", "os_version": "macOS 15.1", "osquery_version": "5.14.1", "status": "online",
            "seen_time": "2026-11-06T11:59:50Z", "uptime": 86_400_000_000_000u64, "memory": 34_359_738_368u64,
            "hardware_model": "MacBookPro18,3", "team_id": 2, "team_name": "Workstations",
            "gigs_disk_space_available": 120.5, "percent_disk_space_available": 24.0, "gigs_total_disk_space": 494.38,
            "disk_encryption_enabled": true,
            "issues": { "failing_policies_count": 1, "critical_vulnerabilities_count": 2, "total_issues_count": 3 },
            "mdm": { "enrollment_status": "On (automatic)", "name": "Fleet" },
            "geolocation": { "country_iso": "ro", "city_name": "Bucharest", "geometry": { "type": "point", "coordinates": [26.1, 44.43] } }
        }),
        json!({
            "id": 2, "hostname": "i-0a1b2c3d4e5f60001", "display_name": "i-0a1b2c3d4e5f60001", "platform": "ubuntu",
            "os_version": "Ubuntu 24.04.1 LTS", "osquery_version": "5.14.1", "status": "offline", "seen_time": "2026-11-06T10:00:00Z",
            "uptime": 3_600_000_000_000u64, "memory": 8_589_934_592u64, "team_id": null, "team_name": null,
            "gigs_disk_space_available": 40.0, "percent_disk_space_available": 50.0, "gigs_total_disk_space": 80.0,
            "issues": { "failing_policies_count": 0, "total_issues_count": 0 },
            "mdm": { "enrollment_status": null }
        }),
        json!({
            "id": 3, "hostname": "desktop-sam", "display_name": "desktop-sam", "platform": "windows", "os_version": "Windows 11 Pro 23H2",
            "osquery_version": "5.13.0", "status": "missing", "seen_time": "2026-10-01T09:00:00Z", "fleet_id": 4, "fleet_name": "Office",
            "issues": { "failing_policies_count": 2, "total_issues_count": 2 },
            "mdm": { "enrollment_status": "Off" }
        }),
    ]
}

/// The host counts `GET /api/v1/fleet/host_summary` answers.
pub fn host_summary() -> Value {
    json!({
        "team_id": null, "totals_hosts_count": 3, "online_count": 1, "offline_count": 1, "mia_count": 1, "missing_30_days_count": 1,
        "new_count": 0, "all_linux_count": 1, "low_disk_space_count": 0,
        "platforms": [{ "platform": "darwin", "hosts_count": 1 }, { "platform": "ubuntu", "hosts_count": 1 }, { "platform": "windows", "hosts_count": 1 }]
    })
}

/// Two global policies.
pub fn policies() -> Value {
    json!({ "policies": [
        { "id": 12, "name": "Disk encryption enabled", "critical": true, "platform": "darwin,windows", "passing_host_count": 1, "failing_host_count": 1 },
        { "id": 13, "name": "Firewall enabled", "critical": false, "platform": "", "passing_host_count": 2, "failing_host_count": 0 }
    ] })
}

/// The vulnerabilities on the most hosts: Premium's scores on the first two, none on the third (Fleet Free).
pub fn vulnerabilities() -> Value {
    json!({ "vulnerabilities": [
        { "cve": "CVE-2024-3094", "created_at": "2024-03-29T17:15:00Z", "hosts_count": 7, "cvss_score": 10.0, "epss_probability": 0.86, "cisa_known_exploit": true },
        { "cve": "CVE-2023-4863", "created_at": "2023-09-12T15:15:00Z", "hosts_count": 4, "cvss_score": 8.8, "epss_probability": 0.42, "cisa_known_exploit": false },
        { "cve": "CVE-2022-30190", "created_at": "2022-06-01T00:15:00Z", "hosts_count": 1 }
    ], "count": 123, "meta": { "has_next_results": true, "has_previous_results": false } })
}

/// Fleet answering from a table of paths; records requests; `refuse` answers 401 to every request.
#[derive(Default)]
pub struct FakeFleet {
    pub answers: StdMutex<HashMap<String, Value>>,
    /// The server's activities (by id), answered to `activities_path` filtered and paged as Fleet does.
    pub activities: StdMutex<Option<Vec<Value>>>,
    pub requests: StdMutex<Vec<String>>,
    pub refuse: bool,
}

impl FakeFleet {
    pub fn answer(&self, path: impl Into<String>, body: Value) {
        self.answers.lock().unwrap().insert(path.into(), body);
    }

    pub fn log(&self, activities: &[Value]) {
        *self.activities.lock().unwrap() = Some(activities.to_vec());
    }

    /// `/activities?page=…&start_created_at=…` over [`Self::log`].
    fn list_activities(&self, path: &str) -> Option<Value> {
        let log = self.activities.lock().unwrap().clone()?;
        let query = path.strip_prefix("/activities?")?;
        let param = |k: &str| query.split('&').find_map(|p| p.strip_prefix(&format!("{k}=")).map(str::to_string));
        let page: usize = param("page")?.parse().ok()?;
        let start = param("start_created_at").map_or(i64::MIN, |t| chrono::DateTime::parse_from_rfc3339(&t).unwrap().timestamp_millis());
        let created = |a: &Value| chrono::DateTime::parse_from_rfc3339(a["created_at"].as_str().unwrap()).unwrap().timestamp_millis();
        let mut matching: Vec<Value> = log.into_iter().filter(|a| created(a) >= start).collect();
        matching.sort_by_key(|a| a["id"].as_u64());
        let rows: Vec<Value> = matching.iter().skip(page * ACTIVITIES_PAGE_SIZE).take(ACTIVITIES_PAGE_SIZE).cloned().collect();
        let next = matching.len() > (page + 1) * ACTIVITIES_PAGE_SIZE;
        // Fleet answers `null` when there are none.
        Some(json!({ "activities": if rows.is_empty() { Value::Null } else { Value::Array(rows) }, "meta": { "has_next_results": next, "has_previous_results": page > 0 } }))
    }

    pub fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl FleetApi for FakeFleet {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, FleetError> {
        assert_eq!((base, token), ("https://fleet.example.com", "fleet-test-token"));
        self.requests.lock().unwrap().push(path.to_string());
        if self.refuse {
            return Err(FleetError::Auth("Authentication required".into()));
        }
        if let Some(answer) = self.list_activities(path) {
            return Ok(answer);
        }
        self.answers.lock().unwrap().get(path).cloned().ok_or_else(|| FleetError::NotFound(format!("no answer for {path}")))
    }
}
