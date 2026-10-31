//! Synthetic API tests (Digital Experience → Synthetic Tests): HTTP, SSL, DNS and TCP checks the
//! ObserveCraft server runs on a schedule, from one location named after the install.
//!
//! - **Scheduling.** [`run_scheduler`] claims due tests every few seconds with
//!   `FOR UPDATE SKIP LOCKED`, moves each one's `next_run_at` to its next slot and holds
//!   `claimed_until` while the run lasts, so with several servers a run happens once. Results are
//!   kept in Postgres for 30 days.
//! - **Alerting.** Every test owns a monitor of type `synthetics` (`[Synthetics] <name>`), created
//!   and updated with the test. The runner keeps its state: Alert once scheduled runs have failed
//!   for the test's minimum failure duration (immediately with 0), OK on the next passing run. A
//!   state change is a monitor event with the test's rendered message; its `@handles` get
//!   notifications through the monitors' notifier (webhook, Slack, email channels), muted while a
//!   downtime applies to the monitor (by id or tags). Runs started by hand are recorded but leave
//!   the state alone. Uptime is the share of time the monitor was OK (No Data excluded).
//! - **Metrics.** Each scheduled run writes `synthetics.test_runs` (count, tagged `result`,
//!   `subtype`, `location`, `test_id`, `env` and the test's `key:value` tags) and its response time
//!   in milliseconds (`synthetics.http.response.time`, `synthetics.ssl.handshake.time`,
//!   `synthetics.dns.response.time`, `synthetics.tcp.connect.time`, distributions), and for HTTP
//!   tests the phases (`synthetics.http.{redirect,dns,connect,ssl,firstbyte,download}.time`) to the
//!   metric tables, as logs-based metrics are written, for dashboards, monitors and SLOs.
//! - **Safety.** Targets are resolved and refused when internal (see [`super::synthetics_net`]);
//!   only standard users and admins create, edit, run and send tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::downtimes::{load_downtimes, Downtime};
use super::logs::TimeRange;
use super::metrics::TimeAggregator;
use super::monitor_engine::{recipients, render_message, scope_covers, Comparator, Detection, MessageContext, MonitorOptions, MonitorStatus, Thresholds};
use super::synthetics_exec::{self as exec, SyntheticAssertion, SyntheticAssertionType, SyntheticOperator, SyntheticRequest, SyntheticRunDetail, SyntheticTestType};
use super::synthetics_settings as vars;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_metrics::{Histogram, Point, PointValue, Retention};
use crate::state::AppState;

/// The one location tests run from: the ObserveCraft server.
pub const LOCATION_ID: &str = "observecraft-server";
const MAX_TESTS: i64 = 500;
const MAX_NAME_CHARS: usize = 200;
const MAX_ASSERTIONS: usize = 20;
const MAX_HEADERS: usize = 50;
const MAX_BODY_CHARS: usize = 64 * 1024;
const MAX_URL_CHARS: usize = 2048;
const MAX_MESSAGE_CHARS: usize = 10_000;
const MAX_TAGS: usize = 50;
const MAX_RETRIES: i32 = 3;
const MAX_RETRY_INTERVAL_MS: i32 = 60_000;
const MAX_FAILURE_MINUTES: i32 = 120;
/// Every 1, 5, 15 or 30 minutes, 1, 6 or 12 hours, daily or weekly.
pub const FREQUENCIES: [i32; 9] = [60, 300, 900, 1800, 3600, 21_600, 43_200, 86_400, 604_800];
const RESULT_RETENTION_DAYS: i32 = 30;
/// Results listed for a test's detail page (newest first).
const MAX_RESULTS: i64 = 5000;
/// Runs in flight per server.
const CONCURRENT_RUNS: usize = 16;
const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

// ================================================================ API types

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticLocation {
    pub id: String,
    pub name: String,
    /// The operator allows tests of loopback, private and link-local addresses (`OC_SYNTHETICS_ALLOW_PRIVATE`).
    pub private_targets: bool,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticRetry {
    /// Retries of a failed run before it counts as failed (0–3).
    #[serde(default)]
    pub count: i32,
    /// Milliseconds between retries.
    #[serde(default = "default_retry_interval")]
    pub interval_ms: i32,
}

fn default_retry_interval() -> i32 {
    300
}

impl Default for SyntheticRetry {
    fn default() -> Self {
        SyntheticRetry { count: 0, interval_ms: default_retry_interval() }
    }
}

fn one() -> i32 {
    1
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticTestInput {
    pub name: String,
    pub r#type: SyntheticTestType,
    pub request: SyntheticRequest,
    #[serde(default)]
    pub assertions: Vec<SyntheticAssertion>,
    /// Location ids; empty for the ObserveCraft server.
    #[serde(default)]
    pub locations: Vec<String>,
    /// 60, 300, 900, 1800, 3600, 21600, 43200, 86400 or 604800.
    pub frequency_seconds: i32,
    #[serde(default)]
    pub retry: SyntheticRetry,
    /// Alert once the test has failed for this many minutes (0: on the first failed run).
    #[serde(default)]
    pub min_failure_duration_minutes: i32,
    /// … from at least this many locations.
    #[serde(default = "one")]
    pub min_location_failed: i32,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub team: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Notification message of the test's monitor (`{{#is_alert}}…{{/is_alert}}`, `@handles`).
    #[serde(default)]
    pub message: String,
    pub priority: Option<i16>,
    #[serde(default)]
    pub paused: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticTest {
    pub id: String,
    pub name: String,
    pub r#type: SyntheticTestType,
    /// The host the test reaches (from the URL for HTTP tests).
    pub domain: String,
    pub request: SyntheticRequest,
    pub assertions: Vec<SyntheticAssertion>,
    pub locations: Vec<String>,
    pub frequency_seconds: i32,
    pub retry: SyntheticRetry,
    pub min_failure_duration_minutes: i32,
    pub min_location_failed: i32,
    pub env: String,
    pub team: String,
    pub tags: Vec<String>,
    pub message: String,
    pub priority: Option<i16>,
    pub paused: bool,
    /// The alert state of the test's monitor.
    pub status: MonitorStatus,
    /// A downtime silences the test's notifications now.
    pub muted: bool,
    pub monitor_id: Option<i64>,
    /// Percent of the requested time range the test was OK (absent without a state in it).
    pub uptime: Option<f64>,
    /// Epoch milliseconds.
    pub last_run_at: Option<i64>,
    pub next_run_at: i64,
    /// First failed scheduled run of the current failure streak.
    pub failing_since: Option<i64>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct SyntheticTestSearchRequest {
    /// Words match name, domain and tags; `type:`, `env:`, `team:` and `status:` (ok, alert, nodata, paused) filter; other `key:value` words match tags.
    #[serde(default)]
    pub query: String,
    /// The uptime is computed over this range.
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
pub struct SyntheticTestsPage {
    pub tests: Vec<SyntheticTest>,
    pub location: SyntheticLocation,
}

#[derive(Deserialize, ToSchema)]
pub struct SyntheticResultsRequest {
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SyntheticUptimeSegment {
    pub status: MonitorStatus,
    /// Epoch milliseconds.
    pub from: i64,
    pub to: i64,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticResultSummary {
    pub id: i64,
    pub location: String,
    /// Epoch milliseconds.
    pub started_at: i64,
    pub duration_ms: f64,
    pub passed: bool,
    pub status_code: Option<i32>,
    pub error: Option<String>,
    pub attempts: i32,
    /// scheduled | manual
    pub trigger: String,
    pub failed_assertions: i64,
    /// HTTP runs: where the time went.
    #[schema(value_type = Option<exec::SyntheticTimings>)]
    pub timings: Option<sqlx::types::Json<exec::SyntheticTimings>>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticResults {
    pub uptime: Option<f64>,
    /// The test's alert state over the range.
    pub segments: Vec<SyntheticUptimeSegment>,
    /// Newest first, at most 5000.
    pub results: Vec<SyntheticResultSummary>,
    /// Runs in the range.
    pub total: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticResult {
    pub id: i64,
    pub test_id: String,
    pub location: String,
    pub started_at: i64,
    pub attempts: i32,
    pub trigger: String,
    pub detail: SyntheticRunDetail,
}

#[derive(Deserialize, ToSchema)]
pub struct SyntheticTryRequest {
    pub r#type: SyntheticTestType,
    pub request: SyntheticRequest,
    #[serde(default)]
    pub assertions: Vec<SyntheticAssertion>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticTryResult {
    pub location: SyntheticLocation,
    pub result: SyntheticRunDetail,
    /// Assertions matching this response, to start the test from.
    pub suggested_assertions: Vec<SyntheticAssertion>,
}

// ================================================================ validation

fn bounded(value: &str, what: &str, max: usize) -> Result<String, String> {
    let v = value.trim();
    if v.chars().count() > max {
        return Err(format!("{what} is at most {max} characters"));
    }
    Ok(v.to_string())
}

/// A host name or IP address (IPv6 with or without brackets), trimmed.
fn check_host(raw: &str, what: &str) -> Result<String, String> {
    let h = raw.trim();
    if h.is_empty() {
        return Err(format!("{what} is required"));
    }
    // `{{ NAME }}` is checked once filled in (see `prepare_run`).
    if !vars::placeholders(h).is_empty() {
        return check_host(&vars::neutralized(h), what).map(|_| h.to_string());
    }
    let bare = h.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return Ok(bare.to_string());
    }
    let valid = h.len() <= 253 && url::Host::parse(h).is_ok_and(|p| matches!(p, url::Host::Domain(_))) && !h.contains(['/', ':', '@', ' ']);
    if valid {
        Ok(h.trim_end_matches('.').to_string())
    } else {
        Err(format!("{what} must be a host name or an IP address (no scheme, port or path)"))
    }
}

fn check_request(t: SyntheticTestType, r: &mut SyntheticRequest) -> Result<(), String> {
    if !(1..=exec::MAX_TIMEOUT_SECONDS).contains(&r.timeout_seconds) {
        return Err(format!("The timeout is 1–{} seconds", exec::MAX_TIMEOUT_SECONDS));
    }
    match t {
        SyntheticTestType::Http => {
            r.method = r.method.trim().to_ascii_uppercase();
            if !METHODS.contains(&r.method.as_str()) {
                return Err(format!("The method is one of {}", METHODS.join(", ")));
            }
            r.url = r.url.trim().to_string();
            if r.url.chars().count() > MAX_URL_CHARS {
                return Err(format!("The URL is at most {MAX_URL_CHARS} characters"));
            }
            let url = url::Url::parse(&vars::neutralized(&r.url)).map_err(|_| "Enter a full URL, such as https://shop.example.com/health".to_string())?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
                return Err("HTTP tests use an http or https URL with a host".into());
            }
            if r.headers.len() > MAX_HEADERS {
                return Err(format!("At most {MAX_HEADERS} headers"));
            }
            for h in &mut r.headers {
                h.name = h.name.trim().to_string();
                if reqwest::header::HeaderName::from_bytes(h.name.as_bytes()).is_err() {
                    return Err(format!("Invalid header name '{}'", h.name));
                }
                if h.value.len() > 4096 || reqwest::header::HeaderValue::from_str(&h.value).is_err() {
                    return Err(format!("Invalid value for header '{}'", h.name));
                }
            }
            if r.body.chars().count() > MAX_BODY_CHARS {
                return Err("The request body is at most 64 KiB".into());
            }
            r.body_type = r.body_type.trim().to_string();
            if r.body_type.len() > 200 || reqwest::header::HeaderValue::from_str(&r.body_type).is_err() {
                return Err("Invalid body type".into());
            }
            if let Some(a) = &r.basic_auth {
                if a.username.is_empty() || a.username.len() > 200 || a.password.len() > 1000 || a.username.contains(':') {
                    return Err("Basic authentication needs a username (no ':') of at most 200 characters".into());
                }
            }
        }
        SyntheticTestType::Ssl => {
            r.host = check_host(&r.host, "The host")?;
            r.port = Some(r.port.unwrap_or(443));
        }
        SyntheticTestType::Dns => {
            r.host = check_host(&r.host, "The domain")?;
            if r.host.parse::<std::net::IpAddr>().is_ok() {
                return Err("DNS tests look up a domain name, not an IP address".into());
            }
            r.dns_server = r.dns_server.trim().to_string();
            if !r.dns_server.is_empty() && vars::placeholders(&r.dns_server).is_empty() && r.dns_server.parse::<std::net::IpAddr>().is_err() {
                return Err("The DNS server must be an IP address".into());
            }
        }
        SyntheticTestType::Tcp => {
            r.host = check_host(&r.host, "The host")?;
            if r.port.is_none_or(|p| p == 0) {
                return Err("TCP tests need a port (1–65535)".into());
            }
        }
    }
    if r.port == Some(0) {
        return Err("Ports are 1–65535".into());
    }
    Ok(())
}

fn check_assertions(t: SyntheticTestType, assertions: &mut [SyntheticAssertion]) -> Result<(), String> {
    if assertions.len() > MAX_ASSERTIONS {
        return Err(format!("At most {MAX_ASSERTIONS} assertions"));
    }
    for a in assertions.iter_mut() {
        a.property = a.property.trim().to_string();
        if a.kind == SyntheticAssertionType::Header {
            a.property = a.property.to_ascii_lowercase();
        }
        exec::check_assertion(t, a)?;
    }
    Ok(())
}

/// Checks and normalizes a test definition.
fn check_input(input: &mut SyntheticTestInput) -> Result<(), String> {
    input.name = bounded(&input.name, "The name", MAX_NAME_CHARS)?;
    if input.name.is_empty() {
        return Err("The name is required".into());
    }
    check_request(input.r#type, &mut input.request)?;
    check_assertions(input.r#type, &mut input.assertions)?;
    if input.locations.is_empty() {
        input.locations = vec![LOCATION_ID.to_string()];
    }
    input.locations.dedup();
    if let Some(unknown) = input.locations.iter().find(|l| l.as_str() != LOCATION_ID) {
        return Err(format!("Unknown location '{unknown}': tests run from the ObserveCraft server ({LOCATION_ID})"));
    }
    if !FREQUENCIES.contains(&input.frequency_seconds) {
        return Err("Run every 1, 5, 15 or 30 minutes, 1, 6 or 12 hours, daily or weekly".into());
    }
    if !(0..=MAX_RETRIES).contains(&input.retry.count) || !(0..=MAX_RETRY_INTERVAL_MS).contains(&input.retry.interval_ms) {
        return Err(format!("Retry 0–{MAX_RETRIES} times, 0–{MAX_RETRY_INTERVAL_MS} ms apart"));
    }
    if !(0..=MAX_FAILURE_MINUTES).contains(&input.min_failure_duration_minutes) {
        return Err(format!("Alert after 0–{MAX_FAILURE_MINUTES} minutes of failures"));
    }
    if input.min_location_failed < 1 || input.min_location_failed as usize > input.locations.len() {
        return Err(format!("Alert when 1–{} locations fail", input.locations.len()));
    }
    input.env = bounded(&input.env, "The environment", 100)?;
    input.team = bounded(&input.team, "The team", 100)?;
    if input.env.contains([',', ' ']) || input.team.contains(',') {
        return Err("Environments have no spaces or commas, teams no commas".into());
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &input.tags {
        let t = t.trim().to_string();
        if !t.is_empty() && !tags.contains(&t) {
            tags.push(t);
        }
    }
    if tags.len() > MAX_TAGS || tags.iter().any(|t| t.len() > 200 || t.contains(',')) {
        return Err(format!("Up to {MAX_TAGS} tags of at most 200 characters, without commas"));
    }
    input.tags = tags;
    if input.message.chars().count() > MAX_MESSAGE_CHARS {
        return Err("The message is at most 10,000 characters".into());
    }
    if input.priority.is_some_and(|p| !(1..=5).contains(&p)) {
        return Err("The priority is P1–P5".into());
    }
    Ok(())
}

/// The request and assertions a run sends: global variables filled in, then checked as a test
/// definition is.
pub fn prepare_run(t: SyntheticTestType, request: &SyntheticRequest, assertions: &[SyntheticAssertion], vars: &HashMap<String, String>) -> Result<(SyntheticRequest, Vec<SyntheticAssertion>), String> {
    let (mut request, mut assertions) = vars::substitute(request, assertions, vars)?;
    check_request(t, &mut request)?;
    check_assertions(t, &mut assertions)?;
    Ok((request, assertions))
}

/// The host a test reaches.
fn domain(t: SyntheticTestType, r: &SyntheticRequest) -> String {
    match t {
        SyntheticTestType::Http => url::Url::parse(&r.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| raw_host(&r.url)),
        _ => r.host.clone(),
    }
}

/// The host part of a URL that doesn't parse (`https://{{ SHOP_HOST }}/health` → `{{ SHOP_HOST }}`).
fn raw_host(url: &str) -> String {
    url.split_once("://").and_then(|(_, rest)| rest.split(['/', '?', '#']).next()).unwrap_or_default().to_string()
}

// ================================================================ scheduling and state

/// The next slot of a test's cadence after a run due at `due` started at `now` (slots missed while
/// the server was down are skipped, not caught up).
pub fn next_due(due: i64, now: i64, frequency_ms: i64) -> i64 {
    if now < due {
        return due;
    }
    due + ((now - due) / frequency_ms + 1) * frequency_ms
}

/// The test's alert state after a scheduled run started at `at`: OK when it passed; Alert once
/// runs have failed for `min_failure_ms` since the first failure of the streak. Returns the new
/// state and the streak's start.
pub fn next_state(previous: MonitorStatus, passed: bool, failing_since: Option<i64>, at: i64, min_failure_ms: i64) -> (MonitorStatus, Option<i64>) {
    if passed {
        return (MonitorStatus::Ok, None);
    }
    let since = failing_since.unwrap_or(at);
    if at - since >= min_failure_ms {
        (MonitorStatus::Alert, Some(since))
    } else {
        (previous, Some(since))
    }
}

/// The share of `[from, to)` the test was OK, of the time it was OK or alerting, and its state over
/// that range. `events` are the monitor's transitions `(at, from, to)` in the range, oldest first;
/// `current` is the state now. Time before the test existed doesn't count.
pub fn uptime(current: MonitorStatus, created_at: i64, events: &[(i64, MonitorStatus, MonitorStatus)], from: i64, to: i64) -> (Option<f64>, Vec<SyntheticUptimeSegment>) {
    let start = from.max(created_at);
    let mut segments = Vec::new();
    if to <= start {
        return (None, segments);
    }
    let mut status = events.first().map(|e| e.1).unwrap_or(current);
    let mut t = start;
    for &(at, _, next) in events {
        if at > to {
            break;
        }
        if at > t {
            segments.push(SyntheticUptimeSegment { status, from: t, to: at });
        }
        status = next;
        t = t.max(at);
    }
    if to > t {
        segments.push(SyntheticUptimeSegment { status, from: t, to });
    }
    // Neighbours in the same state are one segment.
    let mut merged: Vec<SyntheticUptimeSegment> = Vec::new();
    for s in segments {
        match merged.last_mut() {
            Some(last) if last.status == s.status && last.to == s.from => last.to = s.to,
            _ => merged.push(s),
        }
    }
    let (mut up, mut down) = (0i64, 0i64);
    for s in &merged {
        match s.status {
            MonitorStatus::Ok => up += s.to - s.from,
            MonitorStatus::Alert | MonitorStatus::Warn => down += s.to - s.from,
            MonitorStatus::NoData => {}
        }
    }
    let pct = (up + down > 0).then(|| up as f64 * 100.0 / (up + down) as f64);
    (pct, merged)
}

fn operator_text(op: SyntheticOperator) -> &'static str {
    match op {
        SyntheticOperator::Is => "is",
        SyntheticOperator::IsNot => "is not",
        SyntheticOperator::LessThan => "is less than",
        SyntheticOperator::MoreThan => "is more than",
        SyntheticOperator::Contains => "contains",
        SyntheticOperator::DoesNotContain => "does not contain",
        SyntheticOperator::Matches => "matches",
        SyntheticOperator::DoesNotMatch => "does not match",
        SyntheticOperator::IsInMoreThan => "expires in more than",
    }
}

fn assertion_text(a: &SyntheticAssertion) -> String {
    let subject = match a.kind {
        SyntheticAssertionType::StatusCode => "status code".to_string(),
        SyntheticAssertionType::ResponseTime => "response time".to_string(),
        SyntheticAssertionType::Header => format!("header {}", a.property),
        SyntheticAssertionType::Body => "body".to_string(),
        SyntheticAssertionType::BodyJsonPath => format!("body {}", a.property),
        SyntheticAssertionType::Certificate => "certificate".to_string(),
        SyntheticAssertionType::RecordEvery => format!("every {} record", a.property),
        SyntheticAssertionType::RecordSome => format!("at least one {} record", a.property),
    };
    let unit = match a.kind {
        SyntheticAssertionType::ResponseTime => " ms",
        SyntheticAssertionType::Certificate => " days",
        _ => "",
    };
    format!("{subject} {} {}{unit}", operator_text(a.operator), a.target)
}

/// Why a run failed, in one line.
pub fn failure_summary(r: &SyntheticRunDetail) -> Option<String> {
    if r.passed {
        return None;
    }
    if let Some(e) = &r.error {
        return Some(e.clone());
    }
    let failed: Vec<String> = r
        .assertions
        .iter()
        .filter(|a| !a.passed)
        .map(|a| {
            let got = a.error.clone().or_else(|| a.actual.clone().map(|v| format!("got {v}"))).unwrap_or_else(|| "nothing found".into());
            format!("{} ({got})", assertion_text(&a.assertion))
        })
        .collect();
    Some(match failed.len() {
        0 => "The run failed".into(),
        1 => format!("Assertion failed: {}", failed[0]),
        n => format!("{n} assertions failed: {}", failed.join("; ")),
    })
}

/// The monitor message for a state change: the test's template rendered as monitor messages are
/// (`{{#is_alert}}`, `{{#is_recovery}}`, `{{monitor.name}}`, `{{value}}` = failing locations), then
/// what failed.
pub fn notification_message(template: &str, monitor_name: &str, previous: MonitorStatus, next: MonitorStatus, failing: i32, run: &SyntheticRunDetail, location: &str) -> String {
    let opts = MonitorOptions {
        detection: Detection::Threshold,
        time_aggregation: TimeAggregator::Last,
        window_minutes: 1,
        change_mode: None,
        change_minutes: None,
        comparator: Comparator::AboveOrEqual,
        thresholds: Thresholds { critical: 1.0, warning: None, critical_recovery: None, warning_recovery: None },
        notify_no_data: false,
        no_data_minutes: 10,
    };
    let group = BTreeMap::new();
    let ctx = MessageContext { status: next, previous, value: Some(f64::from(failing)), opts: &opts, monitor_name, group: &group };
    let mut out = render_message(template, &ctx).trim().to_string();
    if next == MonitorStatus::Alert {
        if let Some(why) = failure_summary(run) {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&format!("Failing from {location}: {why}"));
        }
    }
    out
}

/// What a scheduled run writes to the metric tables.
fn metric_points(org: &str, test: &TestRow, run: &SyntheticRunDetail, at: i64, retention: Retention, location: &str) -> Vec<Point> {
    let t = SyntheticTestType::parse(&test.kind);
    let mut tags: Vec<(String, String)> = vec![
        ("test_id".into(), test.id.clone()),
        ("location".into(), location.to_string()),
        ("result".into(), if run.passed { "passed" } else { "failed" }.to_string()),
        ("test_type".into(), "api".into()),
        ("subtype".into(), t.as_str().to_string()),
    ];
    if !test.env.is_empty() {
        tags.push(("env".into(), test.env.clone()));
    }
    for tag in test.tags.iter().take(20) {
        if let Some((k, v)) = tag.split_once(':') {
            if !k.is_empty() && !v.is_empty() && !tags.iter().any(|(have, _)| have == k) {
                tags.push((k.to_string(), v.to_string()));
            }
        }
    }
    let point = |metric: &str, description: &str, tags: Vec<(String, String)>, value: PointValue| Point {
        org: org.to_string(),
        metric: metric.to_string(),
        description: Arc::from(description),
        retention,
        tags,
        time_ms: at,
        value,
    };
    let mut out = vec![point("synthetics.test_runs", "Synthetic test runs", tags.clone(), PointValue::Count(1))];
    // Timings only for runs that got an answer.
    let answered = run.error.is_none() || run.status_code.is_some();
    if answered {
        let metric = match t {
            SyntheticTestType::Http => "synthetics.http.response.time",
            SyntheticTestType::Ssl => "synthetics.ssl.handshake.time",
            SyntheticTestType::Dns => "synthetics.dns.response.time",
            SyntheticTestType::Tcp => "synthetics.tcp.connect.time",
        };
        let mut h = Histogram::default();
        h.add(run.duration_ms);
        let timing_tags: Vec<(String, String)> = tags.into_iter().filter(|(k, _)| k != "result").collect();
        out.push(point(metric, "Synthetic test response time (ms)", timing_tags.clone(), PointValue::Distribution(h)));
        // HTTP phases (zero ones are left out: no TLS on http, no redirects followed).
        if let (SyntheticTestType::Http, Some(timings)) = (t, &run.timings) {
            let phases = [
                ("synthetics.http.redirect.time", "Synthetic HTTP redirect time (ms)", timings.redirect),
                ("synthetics.http.dns.time", "Synthetic HTTP DNS lookup time (ms)", timings.dns),
                ("synthetics.http.connect.time", "Synthetic HTTP connection time (ms)", timings.tcp),
                ("synthetics.http.ssl.time", "Synthetic HTTP TLS handshake time (ms)", timings.ssl),
                ("synthetics.http.firstbyte.time", "Synthetic HTTP time to first byte (ms)", timings.first_byte),
                ("synthetics.http.download.time", "Synthetic HTTP download time (ms)", timings.download),
            ];
            for (metric, description, value) in phases {
                if value > 0.0 {
                    let mut h = Histogram::default();
                    h.add(value);
                    out.push(point(metric, description, timing_tags.clone(), PointValue::Distribution(h)));
                }
            }
        }
    }
    out
}

// ================================================================ storage

#[derive(sqlx::FromRow, Clone)]
struct TestRow {
    id: String,
    org_id: String,
    name: String,
    kind: String,
    request: Value,
    assertions: Value,
    locations: Vec<String>,
    frequency_s: i32,
    retry_count: i32,
    retry_interval_ms: i32,
    min_failure_duration_s: i32,
    min_location_failed: i32,
    env: String,
    team: String,
    tags: Vec<String>,
    message: String,
    priority: Option<i16>,
    paused: bool,
    monitor_id: Option<i64>,
    monitor_status: Option<String>,
    monitor_tags: Option<Vec<String>>,
    next_run_at: i64,
    failing_since: Option<i64>,
    last_run_at: Option<i64>,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl TestRow {
    fn request(&self) -> SyntheticRequest {
        serde_json::from_value(self.request.clone()).unwrap_or_default()
    }
    fn assertions(&self) -> Vec<SyntheticAssertion> {
        serde_json::from_value(self.assertions.clone()).unwrap_or_default()
    }
    fn status(&self) -> MonitorStatus {
        self.monitor_status.as_deref().map(MonitorStatus::parse).unwrap_or(MonitorStatus::NoData)
    }
}

/// A timestamp column as epoch milliseconds.
fn ms(column: &str) -> String {
    format!("(extract(epoch FROM {column}) * 1000)::bigint")
}

fn test_select() -> String {
    format!(
        "SELECT t.id::text AS id, t.org_id, t.name, t.type AS kind, t.request, t.assertions, t.locations, t.frequency_s, t.retry_count, t.retry_interval_ms, \
         t.min_failure_duration_s, t.min_location_failed, t.env, t.team, t.tags, t.message, t.priority, t.paused, t.monitor_id, m.status AS monitor_status, \
         m.tags AS monitor_tags, {} AS next_run_at, {} AS failing_since, {} AS last_run_at, c.name AS created_by, u.name AS updated_by, {} AS created_at, {} AS updated_at \
         FROM synthetic_tests t LEFT JOIN monitors m ON m.id = t.monitor_id AND m.org_id = t.org_id \
         LEFT JOIN users c ON c.id = t.created_by LEFT JOIN users u ON u.id = t.updated_by",
        ms("t.next_run_at"),
        ms("t.failing_since"),
        ms("t.last_run_at"),
        ms("t.created_at"),
        ms("t.updated_at")
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Path ids are UUIDs; anything else can't name a test.
fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

fn not_found() -> ApiError {
    ApiError::NotFound("Synthetic test not found".into())
}

async fn test_row(st: &AppState, org: &str, id: &str) -> Result<TestRow, ApiError> {
    if !is_uuid(id) {
        return Err(not_found());
    }
    let row: Option<TestRow> = sqlx::query_as(&format!("{} WHERE t.org_id = $1 AND t.id = $2::uuid", test_select())).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.ok_or_else(not_found)
}

/// A downtime silences the monitor now (one aimed at it, or at all of its tags, for every group).
fn muted_now(monitor_id: Option<i64>, tags: &[String], downtimes: &[Downtime], now: i64) -> bool {
    let Some(id) = monitor_id else { return false };
    let group = BTreeMap::new();
    downtimes.iter().any(|d| d.applies_to(id, tags) && d.window().active_at(now).is_some() && scope_covers(&d.scope, &group))
}

fn to_api(row: TestRow, uptime: Option<f64>, muted: bool) -> SyntheticTest {
    let kind = SyntheticTestType::parse(&row.kind);
    let request = row.request();
    SyntheticTest {
        domain: domain(kind, &request),
        assertions: row.assertions(),
        status: row.status(),
        request,
        r#type: kind,
        id: row.id,
        name: row.name,
        locations: row.locations,
        frequency_seconds: row.frequency_s,
        retry: SyntheticRetry { count: row.retry_count, interval_ms: row.retry_interval_ms },
        min_failure_duration_minutes: row.min_failure_duration_s / 60,
        min_location_failed: row.min_location_failed,
        env: row.env,
        team: row.team,
        tags: row.tags,
        message: row.message,
        priority: row.priority,
        paused: row.paused,
        muted,
        monitor_id: row.monitor_id,
        uptime,
        last_run_at: row.last_run_at,
        next_run_at: row.next_run_at,
        failing_since: row.failing_since,
        created_by: row.created_by,
        updated_by: row.updated_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// The location tests run from, named after the install's address.
fn location(st: &AppState) -> SyntheticLocation {
    let host = url::Url::parse(&st.config.public_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| "local".into());
    SyntheticLocation { id: LOCATION_ID.into(), name: format!("ObserveCraft server ({host})"), private_targets: st.config.synthetics_allow_private }
}

type Transitions = HashMap<i64, Vec<(i64, MonitorStatus, MonitorStatus)>>;

/// State changes of the org's synthetics monitors in the range, oldest first.
async fn transitions(st: &AppState, org: &str, monitor: Option<i64>, range: &TimeRange) -> Result<Transitions, ApiError> {
    let rows: Vec<(i64, i64, String, String)> = sqlx::query_as(
        "SELECT e.monitor_id, (extract(epoch FROM e.created_at) * 1000)::bigint, e.from_status, e.to_status FROM monitor_events e JOIN monitors m ON m.id = e.monitor_id \
         WHERE m.org_id = $1 AND m.type = 'synthetics' AND ($2::bigint IS NULL OR m.id = $2) AND e.created_at >= to_timestamp($3) AND e.created_at <= to_timestamp($4) \
         ORDER BY e.created_at, e.id LIMIT 100000",
    )
    .bind(org)
    .bind(monitor)
    .bind(range.from as f64 / 1000.0)
    .bind(range.to as f64 / 1000.0)
    .fetch_all(&st.postgres)
    .await?;
    let mut out: Transitions = HashMap::new();
    for (id, at, from, to) in rows {
        out.entry(id).or_default().push((at, MonitorStatus::parse(&from), MonitorStatus::parse(&to)));
    }
    Ok(out)
}

/// The test's monitor: `[Synthetics] <name>`, tagged with the test's tags and env, owned by its team.
fn monitor_fields(id: &str, input: &SyntheticTestInput) -> (String, String, Value, Vec<String>, Vec<String>) {
    let name = format!("[Synthetics] {}", input.name);
    let query = format!("synthetics(\"{id}\") {} {}", input.r#type.as_str().to_uppercase(), domain(input.r#type, &input.request));
    let mut tags = input.tags.clone();
    let env = format!("env:{}", input.env);
    if !input.env.is_empty() && !tags.contains(&env) {
        tags.push(env);
    }
    let teams = if input.team.is_empty() { Vec::new() } else { vec![input.team.clone()] };
    (name, query, serde_json::json!({ "syntheticTestId": id }), tags, teams)
}

/// Creates or updates the test's monitor; returns its id.
async fn sync_monitor(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, ctx: &AuthContext, id: &str, monitor: Option<i64>, input: &SyntheticTestInput) -> Result<i64, ApiError> {
    let (name, query, definition, tags, teams) = monitor_fields(id, input);
    if let Some(mid) = monitor {
        let updated = sqlx::query(
            "UPDATE monitors SET name = $3, query = $4, definition = $5, message = $6, priority = $7, tags = $8, teams = $9, modified_at = now() \
             WHERE org_id = $1 AND id = $2 AND type = 'synthetics'",
        )
        .bind(&ctx.org_id)
        .bind(mid)
        .bind(&name)
        .bind(&query)
        .bind(&definition)
        .bind(&input.message)
        .bind(input.priority)
        .bind(&tags)
        .bind(&teams)
        .execute(&mut **tx)
        .await?;
        if updated.rows_affected() == 1 {
            return Ok(mid);
        }
    }
    let (mid,): (i64,) = sqlx::query_as(
        "INSERT INTO monitors (org_id, name, type, query, definition, status, message, priority, tags, teams, draft, created_by) \
         VALUES ($1, $2, 'synthetics', $3, $4, 'nodata', $5, $6, $7, $8, false, $9::uuid) RETURNING id",
    )
    .bind(&ctx.org_id)
    .bind(&name)
    .bind(&query)
    .bind(&definition)
    .bind(&input.message)
    .bind(input.priority)
    .bind(&tags)
    .bind(&teams)
    .bind(&ctx.user_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("UPDATE synthetic_tests SET monitor_id = $3 WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(id).bind(mid).execute(&mut **tx).await?;
    Ok(mid)
}

fn json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

/// Stores a run; passing scheduled runs keep no body.
async fn record(st: &AppState, t: &TestRow, started_at: i64, run: &SyntheticRunDetail, attempts: i32, trigger: &str) -> Result<i64, ApiError> {
    let mut kept = run.clone();
    if trigger == "scheduled" && run.passed {
        kept.body = None;
    }
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO synthetic_results (org_id, test_id, location, started_at, duration_ms, passed, status_code, error, attempts, trigger, details) \
         VALUES ($1, $2::uuid, $3, to_timestamp($4), $5, $6, $7, $8, $9, $10, $11) RETURNING id",
    )
    .bind(&t.org_id)
    .bind(&t.id)
    .bind(LOCATION_ID)
    .bind(started_at as f64 / 1000.0)
    .bind(run.duration_ms)
    .bind(run.passed)
    .bind(run.status_code.map(i32::from))
    .bind(&run.error)
    .bind(attempts)
    .bind(trigger)
    .bind(json(&kept))
    .fetch_one(&st.postgres)
    .await?;
    Ok(id)
}

/// Runs a test, retrying a failed run as configured.
async fn run_with_retries(st: &AppState, t: &TestRow, retries: i32) -> (SyntheticRunDetail, i32) {
    let kind = SyntheticTestType::parse(&t.kind);
    let (values, secrets) = match vars::load_variables(st, &t.org_id).await {
        Ok(v) => v,
        Err(e) => return (failed_run(format!("The global variables could not be loaded: {e}")), 1),
    };
    let (request, assertions) = match prepare_run(kind, &t.request(), &t.assertions(), &values) {
        Ok(p) => p,
        Err(e) => return (failed_run(e), 1),
    };
    let allow = st.config.synthetics_allow_private;
    let mut attempts = 1;
    let mut run = exec::run(kind, &request, &assertions, allow).await;
    while !run.passed && attempts <= retries {
        tokio::time::sleep(Duration::from_millis(t.retry_interval_ms.max(0) as u64)).await;
        run = exec::run(kind, &request, &assertions, allow).await;
        attempts += 1;
    }
    vars::mask(&mut run, &secrets);
    (run, attempts)
}

/// A run that failed before sending anything.
fn failed_run(error: String) -> SyntheticRunDetail {
    SyntheticRunDetail { passed: false, error: Some(error), ..Default::default() }
}

/// Moves the test's monitor to the state after a scheduled run, notifying on a change.
async fn update_state(st: &AppState, t: &TestRow, run: &SyntheticRunDetail, at: i64) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    #[derive(sqlx::FromRow)]
    struct Current {
        monitor_id: Option<i64>,
        failing_since: Option<i64>,
        status: Option<String>,
        name: Option<String>,
        tags: Option<Vec<String>>,
        min_failure_duration_s: i32,
        message: String,
    }
    let row: Option<Current> = sqlx::query_as(
        "SELECT t.monitor_id, (extract(epoch FROM t.failing_since) * 1000)::bigint AS failing_since, m.status, m.name, m.tags, t.min_failure_duration_s, t.message \
         FROM synthetic_tests t LEFT JOIN monitors m ON m.id = t.monitor_id AND m.org_id = t.org_id WHERE t.org_id = $1 AND t.id = $2::uuid FOR UPDATE OF t",
    )
    .bind(&t.org_id)
    .bind(&t.id)
    .fetch_optional(&mut *tx)
    .await?;
    // Deleted while running.
    let Some(Current { monitor_id: monitor, failing_since, status, name: monitor_name, tags: monitor_tags, min_failure_duration_s: min_failure_s, message: template }) = row else {
        return Ok(());
    };
    let previous = status.as_deref().map(MonitorStatus::parse).unwrap_or(MonitorStatus::NoData);
    let (next, since) = next_state(previous, run.passed, failing_since, at, i64::from(min_failure_s) * 1000);
    sqlx::query("UPDATE synthetic_tests SET failing_since = to_timestamp($3) WHERE org_id = $1 AND id = $2::uuid")
        .bind(&t.org_id)
        .bind(&t.id)
        .bind(since.map(|s| s as f64 / 1000.0))
        .execute(&mut *tx)
        .await?;
    let (Some(mid), Some(monitor_name)) = (monitor, monitor_name) else {
        tx.commit().await?;
        return Ok(());
    };
    let failing = i32::from(!run.passed);
    sqlx::query(
        "INSERT INTO monitor_groups (monitor_id, group_key, status, value, last_data_at, state_changed_at) VALUES ($1, '*', $2, $3, now(), now()) \
         ON CONFLICT (monitor_id, group_key) DO UPDATE SET status = EXCLUDED.status, value = EXCLUDED.value, last_data_at = now(), \
         state_changed_at = CASE WHEN monitor_groups.status = EXCLUDED.status THEN monitor_groups.state_changed_at ELSE now() END",
    )
    .bind(mid)
    .bind(next.as_str())
    .bind(f64::from(failing))
    .execute(&mut *tx)
    .await?;
    // The first OK isn't news.
    if next != previous && !(previous == MonitorStatus::NoData && next == MonitorStatus::Ok) {
        let downtimes = load_downtimes(st, &t.org_id).await?;
        let muted = muted_now(Some(mid), monitor_tags.as_deref().unwrap_or_default(), &downtimes, now_ms());
        let message = notification_message(&template, &monitor_name, previous, next, failing, run, &location(st).name);
        let who = if muted { Vec::new() } else { recipients(&message) };
        let (event,): (i64,) =
            sqlx::query_as("INSERT INTO monitor_events (monitor_id, group_key, from_status, to_status, value, muted, recipients, message) VALUES ($1, '*', $2, $3, $4, $5, $6, $7) RETURNING id")
                .bind(mid)
                .bind(previous.as_str())
                .bind(next.as_str())
                .bind(f64::from(failing))
                .bind(muted)
                .bind(&who)
                .bind(&message)
                .fetch_one(&mut *tx)
                .await?;
        for r in &who {
            sqlx::query("INSERT INTO notification_deliveries (event_id, org_id, recipient) VALUES ($1, $2, $3)").bind(event).bind(&t.org_id).bind(r).execute(&mut *tx).await?;
        }
    }
    sqlx::query("UPDATE monitors SET status = $3, evaluated_at = now(), state_changed_at = CASE WHEN status = $3 THEN state_changed_at ELSE now() END WHERE org_id = $1 AND id = $2")
        .bind(&t.org_id)
        .bind(mid)
        .bind(next.as_str())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn write_metrics(st: &AppState, t: &TestRow, run: &SyntheticRunDetail, at: i64) {
    let retention: Option<(i32, i32, i32)> =
        sqlx::query_as("SELECT COALESCE(metrics_retention_days, 0), COALESCE(metrics_5m_days, 0), COALESCE(metrics_1h_days, 0) FROM org_data_settings WHERE org_id = $1")
            .bind(&t.org_id)
            .fetch_optional(&st.postgres)
            .await
            .unwrap_or(None);
    let retention = retention.map(|(raw, rollup_5m, rollup_1h)| Retention { raw, rollup_5m, rollup_1h }).unwrap_or_default();
    let points = metric_points(&t.org_id, t, run, at, retention, LOCATION_ID);
    let (sums, histograms) = crate::log_metrics::rows_in_scope(&points, "observecraft.synthetics");
    for (table, rows) in [("otel.metrics_ingest_sum", sums), ("otel.metrics_ingest_exponential_histogram", histograms)] {
        if !rows.is_empty() {
            if let Err(e) = crate::logproc::insert_rows(st, table, rows).await {
                tracing::warn!(%table, test = %t.id, error = %e, "writing synthetic test metrics failed");
            }
        }
    }
}

async fn run_scheduled(st: &AppState, t: TestRow) {
    let at = now_ms();
    let (run, attempts) = run_with_retries(st, &t, t.retry_count).await;
    match record(st, &t, at, &run, attempts, "scheduled").await {
        Ok(_) => {
            if let Err(e) = update_state(st, &t, &run, at).await {
                tracing::warn!(test = %t.id, error = %e, "synthetic test state update failed");
            }
            write_metrics(st, &t, &run, at).await;
        }
        // The test was deleted meanwhile.
        Err(e) => tracing::debug!(test = %t.id, error = %e, "synthetic test result not recorded"),
    }
    let _ = sqlx::query("UPDATE synthetic_tests SET claimed_until = NULL, last_run_at = to_timestamp($3) WHERE org_id = $1 AND id = $2::uuid")
        .bind(&t.org_id)
        .bind(&t.id)
        .bind(at as f64 / 1000.0)
        .execute(&st.postgres)
        .await;
}

/// Claims up to `limit` due tests: their next slot is set and they're held while they run.
async fn claim(st: &AppState, limit: i64) -> Result<Vec<TestRow>, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let rows: Vec<TestRow> = sqlx::query_as(&format!(
        "{} JOIN orgs o ON o.id = t.org_id WHERE NOT t.paused AND t.next_run_at <= now() AND (t.claimed_until IS NULL OR t.claimed_until < now()) AND o.status = 'active' \
         ORDER BY t.next_run_at LIMIT $1 FOR UPDATE OF t SKIP LOCKED",
        test_select()
    ))
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let now = now_ms();
    for t in &rows {
        let next = next_due(t.next_run_at, now, i64::from(t.frequency_s) * 1000);
        let timeout = t.request().timeout_seconds.clamp(1, exec::MAX_TIMEOUT_SECONDS) as i32;
        let hold = (timeout + 5) * (t.retry_count + 1) + t.retry_interval_ms / 1000 * t.retry_count + 30;
        sqlx::query("UPDATE synthetic_tests SET next_run_at = to_timestamp($3), claimed_until = now() + make_interval(secs => $4) WHERE org_id = $1 AND id = $2::uuid")
            .bind(&t.org_id)
            .bind(&t.id)
            .bind(next as f64 / 1000.0)
            .bind(f64::from(hold))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(rows)
}

async fn prune(st: &AppState) -> Result<u64, ApiError> {
    let mut total = 0;
    loop {
        let done = sqlx::query("DELETE FROM synthetic_results WHERE id IN (SELECT id FROM synthetic_results WHERE started_at < now() - make_interval(days => $1) LIMIT 10000)")
            .bind(RESULT_RETENTION_DAYS)
            .execute(&st.postgres)
            .await?
            .rows_affected();
        total += done;
        if done < 10_000 {
            return Ok(total);
        }
    }
}

/// Runs due tests (several servers can run it: tests are claimed with SKIP LOCKED) and prunes old
/// results once an hour.
pub async fn run_scheduler(st: AppState) {
    let slots = Arc::new(tokio::sync::Semaphore::new(CONCURRENT_RUNS));
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pruned_at: Option<std::time::Instant> = None;
    loop {
        tick.tick().await;
        let free = slots.available_permits();
        if free > 0 {
            match claim(&st, free as i64).await {
                Ok(due) => {
                    for t in due {
                        let Ok(permit) = slots.clone().acquire_owned().await else { return };
                        let st = st.clone();
                        tokio::spawn(async move {
                            run_scheduled(&st, t).await;
                            drop(permit);
                        });
                    }
                }
                Err(e) => tracing::warn!(error = %e, "claiming synthetic tests failed"),
            }
        }
        if pruned_at.is_none_or(|t| t.elapsed() > Duration::from_secs(3600)) {
            pruned_at = Some(std::time::Instant::now());
            match prune(&st).await {
                Ok(n) if n > 0 => tracing::info!(deleted = n, "pruned old synthetic test results"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "pruning synthetic test results failed"),
            }
        }
    }
}

// ================================================================ search

fn status_word(t: &SyntheticTest) -> &'static str {
    if t.paused {
        "paused"
    } else {
        t.status.as_str()
    }
}

/// The search box: words match name, domain or tags; `type:`, `env:`, `team:`, `status:` filter.
fn matches_query(t: &SyntheticTest, query: &str) -> bool {
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        let tags = || t.tags.iter().map(|x| x.to_lowercase());
        match w.split_once(':') {
            Some(("type", v)) => t.r#type.as_str() == v,
            Some(("env", v)) => t.env.to_lowercase() == v,
            Some(("team", v)) => t.team.to_lowercase() == v,
            Some(("status", v)) => status_word(t) == v || (v == "no_data" && status_word(t) == "nodata"),
            Some((k, _)) if !k.is_empty() => tags().any(|x| x == w),
            _ => t.name.to_lowercase().contains(&w) || t.domain.to_lowercase().contains(&w) || tags().any(|x| x.contains(&w)),
        }
    })
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/synthetics/locations", operation_id = "list_synthetic_locations", responses((status = 200, body = Vec<SyntheticLocation>)), tag = "synthetics")]
pub async fn locations(State(st): State<AppState>, _ctx: AuthContext) -> ApiResult<Vec<SyntheticLocation>> {
    Ok(Json(vec![location(&st)]))
}

/// The org's tests, worst state first, with their uptime over the range.
#[utoipa::path(post, path = "/api/v1/synthetics/tests/search", operation_id = "search_synthetic_tests", request_body = SyntheticTestSearchRequest,
    responses((status = 200, body = SyntheticTestsPage), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn search(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<SyntheticTestSearchRequest>) -> ApiResult<SyntheticTestsPage> {
    req.range.validate()?;
    let rows: Vec<TestRow> = sqlx::query_as(&format!("{} WHERE t.org_id = $1 ORDER BY lower(t.name), t.id", test_select())).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let changes = transitions(&st, &ctx.org_id, None, &req.range).await?;
    let downtimes = load_downtimes(&st, &ctx.org_id).await?;
    let now = now_ms();
    let mut tests: Vec<SyntheticTest> = rows
        .into_iter()
        .map(|r| {
            let events = r.monitor_id.and_then(|m| changes.get(&m)).map(Vec::as_slice).unwrap_or_default();
            let (pct, _) = uptime(r.status(), r.created_at, events, req.range.from, req.range.to.min(now));
            let muted = muted_now(r.monitor_id, r.monitor_tags.as_deref().unwrap_or_default(), &downtimes, now);
            to_api(r, pct, muted)
        })
        .filter(|t| matches_query(t, &req.query))
        .collect();
    tests.sort_by_key(|t| std::cmp::Reverse(if t.paused { 0 } else { t.status.severity() + 1 }));
    Ok(Json(SyntheticTestsPage { tests, location: location(&st) }))
}

#[utoipa::path(get, path = "/api/v1/synthetics/tests/{id}", operation_id = "get_synthetic_test", params(("id" = String, Path)),
    responses((status = 200, body = SyntheticTest), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<SyntheticTest> {
    let row = test_row(&st, &ctx.org_id, &id).await?;
    let downtimes = load_downtimes(&st, &ctx.org_id).await?;
    let muted = muted_now(row.monitor_id, row.monitor_tags.as_deref().unwrap_or_default(), &downtimes, now_ms());
    Ok(Json(to_api(row, None, muted)))
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<SyntheticTest, ApiError> {
    let row = test_row(st, org, id).await?;
    Ok(to_api(row, None, false))
}

/// Creates a test and its monitor; it first runs within seconds.
#[utoipa::path(post, path = "/api/v1/synthetics/tests", operation_id = "create_synthetic_test", request_body = SyntheticTestInput,
    responses((status = 200, body = SyntheticTest), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<SyntheticTestInput>) -> ApiResult<SyntheticTest> {
    check_input(&mut input).map_err(ApiError::BadRequest)?;
    if let Some(problem) = vars::enforcement_problem(&vars::load_settings(&st, &ctx.org_id).await?, &input.tags, &input.env) {
        return Err(ApiError::BadRequest(problem));
    }
    let mut tx = st.postgres.begin().await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM synthetic_tests WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&mut *tx).await?;
    if count >= MAX_TESTS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_TESTS} synthetic tests")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO synthetic_tests (org_id, name, type, request, assertions, locations, frequency_s, retry_count, retry_interval_ms, min_failure_duration_s, \
         min_location_failed, env, team, tags, message, priority, paused, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18::uuid, $18::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(input.r#type.as_str())
    .bind(json(&input.request))
    .bind(json(&input.assertions))
    .bind(&input.locations)
    .bind(input.frequency_seconds)
    .bind(input.retry.count)
    .bind(input.retry.interval_ms)
    .bind(input.min_failure_duration_minutes * 60)
    .bind(input.min_location_failed)
    .bind(&input.env)
    .bind(&input.team)
    .bind(&input.tags)
    .bind(&input.message)
    .bind(input.priority)
    .bind(input.paused)
    .bind(&ctx.user_id)
    .fetch_one(&mut *tx)
    .await?;
    sync_monitor(&mut tx, &ctx, &id, None, &input).await?;
    tx.commit().await?;
    ctx.audit(&st, "synthetics.test_created", &id, &format!("{} ({} {})", input.name, input.r#type.as_str(), domain(input.r#type, &input.request))).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// What an update compares with: monitor, type, request, assertions, paused, frequency, tags, env.
type StoredTest = (Option<i64>, String, Value, Value, bool, i32, Vec<String>, String);

/// Replaces a test's definition (and updates its monitor). A changed request or assertions run soon.
#[utoipa::path(put, path = "/api/v1/synthetics/tests/{id}", operation_id = "update_synthetic_test", params(("id" = String, Path)), request_body = SyntheticTestInput,
    responses((status = 200, body = SyntheticTest), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<SyntheticTestInput>) -> ApiResult<SyntheticTest> {
    check_input(&mut input).map_err(ApiError::BadRequest)?;
    if !is_uuid(&id) {
        return Err(not_found());
    }
    let mut tx = st.postgres.begin().await?;
    let before: Option<StoredTest> =
        sqlx::query_as("SELECT monitor_id, type, request, assertions, paused, frequency_s, tags, env FROM synthetic_tests WHERE org_id = $1 AND id = $2::uuid FOR UPDATE")
            .bind(&ctx.org_id)
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((monitor, kind, request, assertions, paused, frequency, old_tags, old_env)) = before else { return Err(not_found()) };
    // Enforced tag rules apply when tags or env change (pausing an older test still works).
    if old_tags != input.tags || old_env != input.env {
        if let Some(problem) = vars::enforcement_problem(&vars::load_settings(&st, &ctx.org_id).await?, &input.tags, &input.env) {
            return Err(ApiError::BadRequest(problem));
        }
    }
    let changed = kind != input.r#type.as_str() || request != json(&input.request) || assertions != json(&input.assertions);
    let run_soon = changed || (paused && !input.paused) || frequency != input.frequency_seconds;
    sqlx::query(
        "UPDATE synthetic_tests SET name = $3, type = $4, request = $5, assertions = $6, locations = $7, frequency_s = $8, retry_count = $9, retry_interval_ms = $10, \
         min_failure_duration_s = $11, min_location_failed = $12, env = $13, team = $14, tags = $15, message = $16, priority = $17, paused = $18, updated_by = $19::uuid, \
         updated_at = now(), next_run_at = CASE WHEN $20 THEN now() ELSE next_run_at END, failing_since = CASE WHEN $21 THEN NULL ELSE failing_since END \
         WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(input.r#type.as_str())
    .bind(json(&input.request))
    .bind(json(&input.assertions))
    .bind(&input.locations)
    .bind(input.frequency_seconds)
    .bind(input.retry.count)
    .bind(input.retry.interval_ms)
    .bind(input.min_failure_duration_minutes * 60)
    .bind(input.min_location_failed)
    .bind(&input.env)
    .bind(&input.team)
    .bind(&input.tags)
    .bind(&input.message)
    .bind(input.priority)
    .bind(input.paused)
    .bind(&ctx.user_id)
    .bind(run_soon)
    .bind(changed)
    .execute(&mut *tx)
    .await?;
    sync_monitor(&mut tx, &ctx, &id, monitor, &input).await?;
    tx.commit().await?;
    let action = match (paused, input.paused) {
        (false, true) => "synthetics.test_paused",
        (true, false) => "synthetics.test_resumed",
        _ => "synthetics.test_updated",
    };
    ctx.audit(&st, action, &id, &format!("{} ({} {})", input.name, input.r#type.as_str(), domain(input.r#type, &input.request))).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// Deletes a test with its results and its monitor.
#[utoipa::path(delete, path = "/api/v1/synthetics/tests/{id}", operation_id = "delete_synthetic_test", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !is_uuid(&id) {
        return Err(not_found());
    }
    let mut tx = st.postgres.begin().await?;
    let row: Option<(String, Option<i64>)> =
        sqlx::query_as("DELETE FROM synthetic_tests WHERE org_id = $1 AND id = $2::uuid RETURNING name, monitor_id").bind(&ctx.org_id).bind(&id).fetch_optional(&mut *tx).await?;
    let Some((name, monitor)) = row else { return Err(not_found()) };
    if let Some(mid) = monitor {
        sqlx::query("DELETE FROM monitors WHERE org_id = $1 AND id = $2 AND type = 'synthetics'").bind(&ctx.org_id).bind(mid).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "synthetics.test_deleted", &id, &name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Runs a saved test now, once. The result is recorded; the alert state isn't changed.
#[utoipa::path(post, path = "/api/v1/synthetics/tests/{id}/run", operation_id = "run_synthetic_test", params(("id" = String, Path)),
    responses((status = 200, body = SyntheticResult), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn run_now(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<SyntheticResult> {
    let t = test_row(&st, &ctx.org_id, &id).await?;
    let at = now_ms();
    let (run, attempts) = run_with_retries(&st, &t, 0).await;
    let result_id = record(&st, &t, at, &run, attempts, "manual").await?;
    ctx.audit(&st, "synthetics.test_run", &id, &format!("{} ({})", t.name, if run.passed { "passed" } else { "failed" })).await;
    Ok(Json(SyntheticResult { id: result_id, test_id: t.id, location: LOCATION_ID.into(), started_at: at, attempts, trigger: "manual".into(), detail: run }))
}

/// Sends a request once from the editor (nothing is stored) and suggests assertions for its response.
#[utoipa::path(post, path = "/api/v1/synthetics/try", operation_id = "try_synthetic_request", request_body = SyntheticTryRequest,
    responses((status = 200, body = SyntheticTryResult), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn try_request(State(st): State<AppState>, Writer(ctx): Writer, Json(mut req): Json<SyntheticTryRequest>) -> ApiResult<SyntheticTryResult> {
    check_request(req.r#type, &mut req.request).map_err(ApiError::BadRequest)?;
    check_assertions(req.r#type, &mut req.assertions).map_err(ApiError::BadRequest)?;
    let target = domain(req.r#type, &req.request);
    let (values, secrets) = vars::load_variables(&st, &ctx.org_id).await?;
    let (request, assertions) = prepare_run(req.r#type, &req.request, &req.assertions, &values).map_err(ApiError::BadRequest)?;
    let mut result = exec::run(req.r#type, &request, &assertions, st.config.synthetics_allow_private).await;
    vars::mask(&mut result, &secrets);
    ctx.audit(&st, "synthetics.request_sent", &target, &format!("{} {}", req.r#type.as_str(), if result.passed { "passed" } else { "failed" })).await;
    let suggested_assertions = if result.error.is_none() { exec::suggest(req.r#type, &result) } else { Vec::new() };
    Ok(Json(SyntheticTryResult { location: location(&st), result, suggested_assertions }))
}

/// A test's runs, alert state and uptime over a range.
#[utoipa::path(post, path = "/api/v1/synthetics/tests/{id}/results", operation_id = "list_synthetic_results", params(("id" = String, Path)), request_body = SyntheticResultsRequest,
    responses((status = 200, body = SyntheticResults), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn results(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(req): Json<SyntheticResultsRequest>) -> ApiResult<SyntheticResults> {
    req.range.validate()?;
    let t = test_row(&st, &ctx.org_id, &id).await?;
    let (from, to) = (req.range.from as f64 / 1000.0, req.range.to as f64 / 1000.0);
    let results: Vec<SyntheticResultSummary> = sqlx::query_as(
        "SELECT id, location, (extract(epoch FROM started_at) * 1000)::bigint AS started_at, duration_ms, passed, status_code, error, attempts, trigger, \
         (SELECT count(*) FROM jsonb_array_elements(details->'assertions') a WHERE (a->>'passed') = 'false') AS failed_assertions, \
         NULLIF(details->'timings', 'null'::jsonb) AS timings \
         FROM synthetic_results WHERE org_id = $1 AND test_id = $2::uuid AND started_at >= to_timestamp($3) AND started_at <= to_timestamp($4) \
         ORDER BY started_at DESC, id DESC LIMIT $5",
    )
    .bind(&ctx.org_id)
    .bind(&t.id)
    .bind(from)
    .bind(to)
    .bind(MAX_RESULTS)
    .fetch_all(&st.postgres)
    .await?;
    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM synthetic_results WHERE org_id = $1 AND test_id = $2::uuid AND started_at >= to_timestamp($3) AND started_at <= to_timestamp($4)")
        .bind(&ctx.org_id)
        .bind(&t.id)
        .bind(from)
        .bind(to)
        .fetch_one(&st.postgres)
        .await?;
    let (pct, segments) = match t.monitor_id {
        Some(mid) => {
            let changes = transitions(&st, &ctx.org_id, Some(mid), &req.range).await?;
            uptime(t.status(), t.created_at, changes.get(&mid).map(Vec::as_slice).unwrap_or_default(), req.range.from, req.range.to.min(now_ms()))
        }
        None => (None, Vec::new()),
    };
    Ok(Json(SyntheticResults { uptime: pct, segments, results, total }))
}

#[utoipa::path(get, path = "/api/v1/synthetics/tests/{id}/results/{result_id}", operation_id = "get_synthetic_result", params(("id" = String, Path), ("result_id" = i64, Path)),
    responses((status = 200, body = SyntheticResult), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn result(State(st): State<AppState>, ctx: AuthContext, Path((id, result_id)): Path<(String, i64)>) -> ApiResult<SyntheticResult> {
    if !is_uuid(&id) {
        return Err(not_found());
    }
    let row: Option<(i64, String, String, i64, i32, String, Value)> = sqlx::query_as(
        "SELECT id, test_id::text, location, (extract(epoch FROM started_at) * 1000)::bigint, attempts, trigger, details FROM synthetic_results \
         WHERE org_id = $1 AND test_id = $2::uuid AND id = $3",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(result_id)
    .fetch_optional(&st.postgres)
    .await?;
    let Some((id, test_id, location, started_at, attempts, trigger, details)) = row else { return Err(ApiError::NotFound("Result not found".into())) };
    Ok(Json(SyntheticResult { id, test_id, location, started_at, attempts, trigger, detail: serde_json::from_value(details).unwrap_or_default() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exec::{SyntheticAssertionResult, SyntheticHeader};
    use MonitorStatus::*;

    const MIN: i64 = 60_000;

    fn input(kind: SyntheticTestType, request: SyntheticRequest) -> SyntheticTestInput {
        SyntheticTestInput {
            name: " Storefront health ".into(),
            r#type: kind,
            request,
            assertions: vec![],
            locations: vec![],
            frequency_seconds: 300,
            retry: SyntheticRetry::default(),
            min_failure_duration_minutes: 0,
            min_location_failed: 1,
            env: "prod".into(),
            team: "storefront".into(),
            tags: vec![" service:storefront-api ".into(), "service:storefront-api".into(), "".into()],
            message: "".into(),
            priority: None,
            paused: false,
        }
    }

    fn http(url: &str) -> SyntheticRequest {
        SyntheticRequest { url: url.into(), ..Default::default() }
    }

    fn host(h: &str, port: Option<u16>) -> SyntheticRequest {
        SyntheticRequest { host: h.into(), port, ..Default::default() }
    }

    fn assertion(kind: SyntheticAssertionType, operator: SyntheticOperator, property: &str, target: &str) -> SyntheticAssertion {
        SyntheticAssertion { kind, operator, property: property.into(), target: target.into() }
    }

    #[test]
    fn inputs_are_normalized() {
        let mut i = input(SyntheticTestType::Http, SyntheticRequest { method: "post".into(), ..http(" https://shop.example.com/health ") });
        i.assertions = vec![assertion(SyntheticAssertionType::Header, SyntheticOperator::Is, " Content-Type ", "application/json")];
        assert_eq!(check_input(&mut i), Result::Ok(()));
        assert_eq!(i.name, "Storefront health");
        assert_eq!(i.request.method, "POST");
        assert_eq!(i.request.url, "https://shop.example.com/health");
        assert_eq!(i.locations, vec![LOCATION_ID.to_string()]);
        assert_eq!(i.tags, vec!["service:storefront-api".to_string()]);
        assert_eq!(i.assertions[0].property, "content-type");
        let mut ssl = input(SyntheticTestType::Ssl, host(" shop.example.com. ", None));
        assert_eq!(check_input(&mut ssl), Result::Ok(()));
        assert_eq!((ssl.request.host.as_str(), ssl.request.port), ("shop.example.com", Some(443)));
        let mut v6 = input(SyntheticTestType::Tcp, host("[2606:4700::1]", Some(5432)));
        assert_eq!(check_input(&mut v6), Result::Ok(()));
        assert_eq!(v6.request.host, "2606:4700::1");
    }

    #[test]
    fn invalid_inputs_are_refused_with_clear_errors() {
        let check = |mut i: SyntheticTestInput| check_input(&mut i).unwrap_err();
        let mut nameless = input(SyntheticTestType::Http, http("https://shop.example.com"));
        nameless.name = "  ".into();
        assert_eq!(check(nameless), "The name is required");
        assert!(check(input(SyntheticTestType::Http, http("ftp://shop.example.com"))).contains("http or https"));
        assert!(check(input(SyntheticTestType::Http, http("shop.example.com/health"))).contains("full URL"));
        assert!(check(input(SyntheticTestType::Http, SyntheticRequest { method: "TRACE".into(), ..http("https://shop.example.com") })).contains("method"));
        let bad_header = SyntheticRequest { headers: vec![SyntheticHeader { name: "bad header".into(), value: "x".into() }], ..http("https://shop.example.com") };
        assert!(check(input(SyntheticTestType::Http, bad_header)).contains("Invalid header name"));
        assert!(check(input(SyntheticTestType::Http, SyntheticRequest { timeout_seconds: 61, ..http("https://shop.example.com") })).contains("1–60 seconds"));
        assert!(check(input(SyntheticTestType::Ssl, host("https://shop.example.com", None))).contains("no scheme"));
        assert!(check(input(SyntheticTestType::Dns, host("93.184.215.14", None))).contains("domain name"));
        assert!(check(input(SyntheticTestType::Dns, SyntheticRequest { dns_server: "dns.example.com".into(), ..host("shop.example.com", None) })).contains("IP address"));
        assert!(check(input(SyntheticTestType::Tcp, host("db.example.com", None))).contains("port"));
        let mut freq = input(SyntheticTestType::Tcp, host("db.example.com", Some(5432)));
        freq.frequency_seconds = 30;
        assert!(check(freq).contains("Run every"));
        let mut where_ = input(SyntheticTestType::Tcp, host("db.example.com", Some(5432)));
        where_.locations = vec!["aws:eu-central-1".into()];
        assert!(check(where_).contains("Unknown location"));
        let mut many = input(SyntheticTestType::Tcp, host("db.example.com", Some(5432)));
        many.min_location_failed = 2;
        assert!(check(many).contains("1–1 locations"));
        let mut retries = input(SyntheticTestType::Tcp, host("db.example.com", Some(5432)));
        retries.retry.count = 4;
        assert!(check(retries).contains("Retry 0–3"));
        let mut wrong = input(SyntheticTestType::Tcp, host("db.example.com", Some(5432)));
        wrong.assertions = vec![assertion(SyntheticAssertionType::StatusCode, SyntheticOperator::Is, "", "200")];
        assert!(check(wrong).contains("don't apply to TCP tests"));
    }

    #[test]
    fn tests_may_use_global_variables() {
        let mut i = input(SyntheticTestType::Http, http("https://{{ SHOP_HOST }}/health"));
        i.request.headers = vec![SyntheticHeader { name: "Authorization".into(), value: "Bearer {{ TOKEN }}".into() }];
        assert_eq!(check_input(&mut i), Result::Ok(()));
        assert_eq!(domain(SyntheticTestType::Http, &i.request), "{{ SHOP_HOST }}");
        let mut ssl = input(SyntheticTestType::Ssl, host("{{ SHOP_HOST }}", None));
        assert_eq!(check_input(&mut ssl), Result::Ok(()));
        assert!(check_input(&mut input(SyntheticTestType::Http, http("{{ BASE }}/health"))).unwrap_err().contains("full URL"), "the scheme stays in the test");
        let vars: HashMap<String, String> = [("SHOP_HOST".to_string(), "shop.example.com".to_string()), ("TOKEN".to_string(), "t0k".to_string())].into();
        let (r, _) = prepare_run(SyntheticTestType::Http, &i.request, &[], &vars).unwrap();
        assert_eq!((r.url.as_str(), r.headers[0].value.as_str()), ("https://shop.example.com/health", "Bearer t0k"));
        assert_eq!(prepare_run(SyntheticTestType::Http, &i.request, &[], &HashMap::new()).unwrap_err(), "Unknown global variables: SHOP_HOST, TOKEN");
        let bad: HashMap<String, String> = [("SHOP_HOST".to_string(), "shop example".to_string()), ("TOKEN".to_string(), "x".to_string())].into();
        assert!(prepare_run(SyntheticTestType::Ssl, &ssl.request, &[], &bad).unwrap_err().contains("host name"), "values are checked once filled in");
    }

    #[test]
    fn runs_keep_their_cadence_and_skip_missed_slots() {
        let due = 1_000 * MIN;
        assert_eq!(next_due(due, due, 5 * MIN), due + 5 * MIN);
        assert_eq!(next_due(due, due + 2 * MIN, 5 * MIN), due + 5 * MIN, "a late claim keeps the slot");
        assert_eq!(next_due(due, due + 17 * MIN, 5 * MIN), due + 20 * MIN, "missed slots are skipped");
        assert_eq!(next_due(due, due + 5 * MIN, 5 * MIN), due + 10 * MIN);
        assert_eq!(next_due(due, due - MIN, 5 * MIN), due);
    }

    #[test]
    fn the_state_alerts_after_the_failure_duration_and_recovers_on_a_pass() {
        let t0 = 1_000 * MIN;
        assert_eq!(next_state(NoData, true, None, t0, 0), (Ok, None));
        assert_eq!(next_state(Ok, false, None, t0, 0), (Alert, Some(t0)), "immediately without a duration");
        assert_eq!(next_state(Ok, false, None, t0, 3 * MIN), (Ok, Some(t0)));
        assert_eq!(next_state(Ok, false, Some(t0), t0 + 2 * MIN, 3 * MIN), (Ok, Some(t0)));
        assert_eq!(next_state(Ok, false, Some(t0), t0 + 3 * MIN, 3 * MIN), (Alert, Some(t0)));
        assert_eq!(next_state(Alert, false, Some(t0), t0 + 9 * MIN, 3 * MIN), (Alert, Some(t0)));
        assert_eq!(next_state(Alert, true, Some(t0), t0 + 10 * MIN, 3 * MIN), (Ok, None));
        assert_eq!(next_state(NoData, false, None, t0, 3 * MIN), (NoData, Some(t0)), "a new test stays No Data until it alerts");
    }

    #[test]
    fn uptime_is_the_share_of_time_ok() {
        let h = 60 * MIN;
        // OK the first 3 hours, alerting 1 hour.
        let events = [(3 * h, Ok, Alert)];
        let (pct, segs) = uptime(Alert, 0, &events, 0, 4 * h);
        assert_eq!(pct, Some(75.0));
        assert_eq!(segs, vec![SyntheticUptimeSegment { status: Ok, from: 0, to: 3 * h }, SyntheticUptimeSegment { status: Alert, from: 3 * h, to: 4 * h }]);
        // No change in the range: the current state throughout.
        assert_eq!(uptime(Ok, 0, &[], h, 2 * h).0, Some(100.0));
        assert_eq!(uptime(Alert, 0, &[], h, 2 * h).0, Some(0.0));
        // No Data doesn't count; time before the test existed neither.
        let events = [(2 * h, NoData, Ok), (3 * h, Ok, Alert), (3 * h + 30 * MIN, Alert, Ok)];
        let (pct, segs) = uptime(Ok, h, &events, 0, 4 * h);
        assert_eq!(pct, Some(75.0));
        assert_eq!(segs.first().map(|s| (s.status, s.from)), Some((NoData, h)));
        assert_eq!(uptime(NoData, 0, &[], 0, h).0, None);
        assert_eq!(uptime(Ok, 5 * h, &[], 0, 4 * h), (None, vec![]));
        // Back-to-back segments in the same state merge.
        let events = [(h, Ok, Ok)];
        assert_eq!(uptime(Ok, 0, &events, 0, 2 * h).1.len(), 1);
    }

    #[test]
    fn failures_are_summarized() {
        let failed = |a: SyntheticAssertion, actual: Option<&str>| SyntheticAssertionResult { assertion: a, passed: false, actual: actual.map(Into::into), error: None };
        let run = SyntheticRunDetail {
            assertions: vec![
                failed(assertion(SyntheticAssertionType::StatusCode, SyntheticOperator::Is, "", "200"), Some("503")),
                SyntheticAssertionResult { assertion: assertion(SyntheticAssertionType::ResponseTime, SyntheticOperator::LessThan, "", "1000"), passed: true, actual: Some("80".into()), error: None },
            ],
            ..Default::default()
        };
        assert_eq!(failure_summary(&run).unwrap(), "Assertion failed: status code is 200 (got 503)");
        let two = SyntheticRunDetail {
            assertions: vec![
                failed(assertion(SyntheticAssertionType::Header, SyntheticOperator::Contains, "content-type", "json"), None),
                failed(assertion(SyntheticAssertionType::Certificate, SyntheticOperator::IsInMoreThan, "", "30"), Some("12.0")),
            ],
            ..Default::default()
        };
        assert_eq!(failure_summary(&two).unwrap(), "2 assertions failed: header content-type contains json (nothing found); certificate expires in more than 30 days (got 12.0)");
        let error = SyntheticRunDetail { error: Some("Timed out after 30 s".into()), ..Default::default() };
        assert_eq!(failure_summary(&error).unwrap(), "Timed out after 30 s");
        assert_eq!(failure_summary(&SyntheticRunDetail { passed: true, ..Default::default() }), None);
    }

    #[test]
    fn notifications_render_the_template_and_say_what_failed() {
        let template = "{{#is_alert}}Checkout is down @slack-storefront{{/is_alert}}{{#is_recovery}}{{monitor.name}} recovered @slack-storefront{{/is_recovery}}";
        let run = SyntheticRunDetail { error: Some("Connection refused".into()), ..Default::default() };
        let alert = notification_message(template, "[Synthetics] Checkout", Ok, Alert, 1, &run, "ObserveCraft server (observecraft.example.com)");
        assert_eq!(alert, "Checkout is down @slack-storefront\n\nFailing from ObserveCraft server (observecraft.example.com): Connection refused");
        assert_eq!(recipients(&alert), vec!["slack-storefront".to_string()]);
        let ok = SyntheticRunDetail { passed: true, ..Default::default() };
        assert_eq!(notification_message(template, "[Synthetics] Checkout", Alert, Ok, 0, &ok, "x"), "[Synthetics] Checkout recovered @slack-storefront");
        assert_eq!(notification_message("", "m", Ok, Alert, 1, &run, "here"), "Failing from here: Connection refused");
    }

    fn row(kind: &str, env: &str, tags: &[&str]) -> TestRow {
        TestRow {
            id: "7d3c1d0e-0000-4000-8000-000000000001".into(),
            org_id: "default".into(),
            name: "Checkout".into(),
            kind: kind.into(),
            request: json(&http("https://shop.example.com")),
            assertions: Value::Array(vec![]),
            locations: vec![LOCATION_ID.into()],
            frequency_s: 60,
            retry_count: 0,
            retry_interval_ms: 300,
            min_failure_duration_s: 0,
            min_location_failed: 1,
            env: env.into(),
            team: String::new(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            message: String::new(),
            priority: None,
            paused: false,
            monitor_id: None,
            monitor_status: None,
            monitor_tags: None,
            next_run_at: 0,
            failing_since: None,
            last_run_at: None,
            created_by: None,
            updated_by: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn runs_become_datadog_style_metrics() {
        let t = row("http", "prod", &["service:storefront-api", "critical", "env:staging"]);
        let run = SyntheticRunDetail { passed: false, status_code: Some(503), duration_ms: 240.0, ..Default::default() };
        let points = metric_points("default", &t, &run, 1_000, Retention::default(), LOCATION_ID);
        assert_eq!(points.len(), 2);
        assert_eq!((points[0].metric.as_str(), &points[0].value), ("synthetics.test_runs", &PointValue::Count(1)));
        let tag = |p: &Point, k: &str| p.tags.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(tag(&points[0], "result").as_deref(), Some("failed"));
        assert_eq!(tag(&points[0], "subtype").as_deref(), Some("http"));
        assert_eq!(tag(&points[0], "env").as_deref(), Some("prod"), "the test's env wins over an env tag");
        assert_eq!(tag(&points[0], "service").as_deref(), Some("storefront-api"));
        assert_eq!(tag(&points[0], "critical"), None, "only key:value tags");
        assert_eq!(points[1].metric, "synthetics.http.response.time");
        assert_eq!(tag(&points[1], "result"), None);
        assert!(matches!(&points[1].value, PointValue::Distribution(h) if h.count == 1 && h.sum == 240.0));
        let timed = SyntheticRunDetail { timings: Some(exec::SyntheticTimings { dns: 7.0, tcp: 4.0, first_byte: 10.0, download: 0.4, total: 21.4, ..Default::default() }), ..run.clone() };
        let names: Vec<String> = metric_points("default", &t, &timed, 1_000, Retention::default(), LOCATION_ID).into_iter().map(|p| p.metric).collect();
        assert_eq!(names[2..], ["synthetics.http.dns.time", "synthetics.http.connect.time", "synthetics.http.firstbyte.time", "synthetics.http.download.time"], "no ssl or redirect phase");
        // Unanswered runs count, without a time.
        let refused = SyntheticRunDetail { error: Some("Connection refused".into()), duration_ms: 3.0, ..Default::default() };
        let points = metric_points("default", &row("tcp", "", &[]), &refused, 1_000, Retention::default(), LOCATION_ID);
        assert_eq!(points.iter().map(|p| p.metric.as_str()).collect::<Vec<_>>(), vec!["synthetics.test_runs"]);
        let dns = SyntheticRunDetail { passed: true, duration_ms: 9.0, ..Default::default() };
        assert_eq!(metric_points("default", &row("dns", "", &[]), &dns, 1_000, Retention::default(), LOCATION_ID)[1].metric, "synthetics.dns.response.time");
    }

    #[test]
    fn the_search_box_filters_tests() {
        let mut t = to_api(row("http", "prod", &["service:storefront-api", "team:web"]), None, false);
        t.status = Alert;
        for q in ["", "check", "shop.example", "type:http", "env:prod", "status:alert", "service:storefront-api", "storefront", "CHECKOUT type:http"] {
            assert!(matches_query(&t, q), "{q}");
        }
        for q in ["type:ssl", "env:staging", "status:ok", "service:billing", "inventory"] {
            assert!(!matches_query(&t, q), "{q}");
        }
        t.paused = true;
        assert!(matches_query(&t, "status:paused") && !matches_query(&t, "status:alert"));
    }

    #[test]
    fn the_monitor_carries_the_tests_name_tags_and_team() {
        let mut i = input(SyntheticTestType::Http, http("https://shop.example.com/health"));
        check_input(&mut i).unwrap();
        let (name, query, definition, tags, teams) = monitor_fields("abc", &i);
        assert_eq!(name, "[Synthetics] Storefront health");
        assert_eq!(query, "synthetics(\"abc\") HTTP shop.example.com");
        assert_eq!(definition, serde_json::json!({"syntheticTestId": "abc"}));
        assert_eq!(tags, vec!["service:storefront-api".to_string(), "env:prod".to_string()]);
        assert_eq!(teams, vec!["storefront".to_string()]);
    }
}
