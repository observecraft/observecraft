//! Dashboards → Reports: a dashboard summarized over a time frame and sent on a schedule.
//!
//! A report is a dashboard, its template variable values, a time frame (`1h` … `1mo`, ending when
//! the report is sent), a schedule (daily, weekly or monthly at a local time in an IANA zone) and
//! recipients: member emails and notification channel handles (`slack-…`, `webhook-…`), delivered
//! through the monitor notifier's [`send`]. Members who subscribe also receive it by email.
//!
//! The message holds the dashboard title, the time frame, one line per widget and a link to the
//! dashboard. Timeseries and query value widgets get their current value and the minimum and
//! maximum over the frame, computed with the metric query engine; other widgets are listed.
//! Email needs `OC_SMTP_URL`; without it emails are reported as not sent and channels still work.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::dashboards::TemplateVariable;
use super::logs::TimeRange;
use super::notifications::{send, Outcome, Outgoing};
use super::workflow_cron::{zone, Cron};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_RECIPIENTS: usize = 50;
const MAX_MESSAGE_CHARS: usize = 5_000;
/// Series summarized per widget query (the busiest groups first).
const MAX_SERIES_PER_QUERY: usize = 5;
const CLAIM_SECONDS: f64 = 300.0;

/// The time frames a report covers, ending at send time.
pub(crate) const TIME_FRAMES: &[(&str, &str, i64)] = &[
    ("1h", "Past 1 Hour", 3_600_000),
    ("4h", "Past 4 Hours", 4 * 3_600_000),
    ("1d", "Past 1 Day", 86_400_000),
    ("2d", "Past 2 Days", 2 * 86_400_000),
    ("1w", "Past 1 Week", 7 * 86_400_000),
    ("1mo", "Past 1 Month", 30 * 86_400_000),
];

fn time_frame(key: &str) -> Option<(&'static str, i64)> {
    TIME_FRAMES.iter().find(|(k, _, _)| *k == key).map(|(_, label, ms)| (*label, *ms))
}

// ================================================================ schedule

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReportSchedule {
    /// daily | weekly | monthly
    pub frequency: String,
    /// Local time `HH:MM` (24 h).
    pub time: String,
    /// Weekly: 0 = Sunday … 6 = Saturday.
    pub weekday: Option<u8>,
    /// Monthly: 1–28.
    pub month_day: Option<u8>,
    /// IANA zone (`UTC`, `Europe/Paris`).
    pub timezone: String,
}

impl ReportSchedule {
    /// The schedule as a cron expression (`minute hour day-of-month month day-of-week`).
    fn cron(&self) -> Result<Cron, String> {
        let (h, m) = self.time.split_once(':').ok_or_else(|| format!("{} isn't a time (HH:MM)", self.time))?;
        let (h, m): (u32, u32) = match (h.parse(), m.parse()) {
            (Ok(h), Ok(m)) if h < 24 && m < 60 && self.time.len() == 5 => (h, m),
            _ => return Err(format!("{} isn't a time (HH:MM)", self.time)),
        };
        let expr = match self.frequency.as_str() {
            "daily" => format!("{m} {h} * * *"),
            "weekly" => match self.weekday {
                Some(d) if d <= 6 => format!("{m} {h} * * {d}"),
                _ => return Err("A weekly report needs a weekday (0 = Sunday … 6 = Saturday)".into()),
            },
            "monthly" => match self.month_day {
                Some(d) if (1..=28).contains(&d) => format!("{m} {h} {d} * *"),
                _ => return Err("A monthly report needs a day of the month from 1 to 28".into()),
            },
            other => return Err(format!("Unknown frequency {other} (daily, weekly or monthly)")),
        };
        Cron::parse(&expr)
    }

    /// The first send strictly after `after_ms` (Unix ms).
    pub(crate) fn next_after(&self, after_ms: i64) -> Result<i64, String> {
        let tz = zone(&self.timezone)?;
        self.cron()?.next_after(after_ms, tz).ok_or_else(|| "The schedule never fires".into())
    }

    /// "Weekly on Monday at 09:00 (Europe/Paris)".
    pub(crate) fn describe(&self) -> String {
        const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
        let when = match self.frequency.as_str() {
            "weekly" => format!("Weekly on {}", DAYS.get(self.weekday.unwrap_or(0) as usize).unwrap_or(&"Sunday")),
            "monthly" => format!("Monthly on day {}", self.month_day.unwrap_or(1)),
            _ => "Daily".to_string(),
        };
        let tz = if self.timezone.trim().is_empty() { "UTC" } else { self.timezone.trim() };
        format!("{when} at {} ({tz})", self.time)
    }
}

// ================================================================ widgets → queries

/// Replaces the `$var` filters of a metric query with `prefix:value` (dropped for `*`), and
/// `$var.value` anywhere with the value, as the web app does (web/src/lib/metricQuery.ts).
pub(crate) fn resolve_vars(query: &str, vars: &[TemplateVariable], values: &HashMap<String, String>) -> String {
    let value_of = |name: &str| -> Option<String> {
        let tv = vars.iter().find(|t| t.name == name)?;
        Some(values.get(name).cloned().unwrap_or_else(|| tv.default.clone()))
    };
    // `$name.value` → the raw value.
    let mut text = String::with_capacity(query.len());
    let mut rest = query;
    while let Some(i) = rest.find('$') {
        text.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let name: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let tail = &after[name.len()..];
        match (tail.strip_prefix(".value"), value_of(&name)) {
            (Some(t), Some(v)) if !t.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') => {
                text.push_str(&v);
                rest = t;
            }
            _ => {
                text.push('$');
                rest = after;
            }
        }
    }
    text.push_str(rest);
    // `{…, $name, …}` → `prefix:value`.
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else { break };
        out.push_str(&rest[..=open]);
        let mut parts: Vec<String> = Vec::new();
        for raw in rest[open + 1..close].split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let Some(name) = raw.strip_prefix('$') else {
                parts.push(raw.to_string());
                continue;
            };
            let Some(tv) = vars.iter().find(|t| t.name == name) else { continue };
            let v = values.get(name).cloned().unwrap_or_else(|| tv.default.clone());
            if v.is_empty() || v == "*" {
                continue;
            }
            parts.push(if tv.prefix.is_empty() { v } else { format!("{}:{v}", tv.prefix) });
        }
        let concrete: Vec<&String> = parts.iter().filter(|p| p.as_str() != "*").collect();
        if concrete.is_empty() {
            out.push('*');
        } else {
            out.push_str(&concrete.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(","));
        }
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// What a report says about one widget: the metric queries to summarize, or none to list it.
#[derive(Debug, PartialEq)]
pub(crate) struct WidgetTarget {
    pub title: String,
    /// The widget type (`timeseries`, `query_value`, `note`, …; integration widgets their original's).
    pub kind: String,
    pub queries: Vec<String>,
}

const LISTED_TITLES: &[(&str, &str)] = &[
    ("toplist", "Top list"),
    ("table", "Table"),
    ("bar_chart", "Bar chart"),
    ("change", "Change"),
    ("heatmap", "Heatmap"),
    ("distribution", "Distribution"),
    ("pie", "Pie chart"),
    ("treemap", "Treemap"),
    ("note", "Note"),
    ("image", "Image"),
    ("monitor_summary", "Monitor summary"),
    ("log_stream", "Log stream"),
    ("timeseries", "Timeseries"),
    ("query_value", "Query value"),
];

fn default_title(kind: &str) -> String {
    LISTED_TITLES.iter().find(|(k, _)| *k == kind).map(|(_, t)| t.to_string()).unwrap_or_else(|| kind.replace('_', " "))
}

/// A Datadog-format query without the display functions the summary doesn't need.
fn strip_display_functions(q: &str) -> String {
    let mut q = q.trim().to_string();
    loop {
        let before = q.len();
        for f in [".as_rate()", ".as_count()", ".weighted()"] {
            if let Some(s) = q.strip_suffix(f) {
                q = s.to_string();
            }
        }
        if let Some(i) = q.rfind(".fill(") {
            if q.ends_with(')') && !q[i..].contains('}') {
                q.truncate(i);
            }
        }
        if q.len() == before {
            return q;
        }
    }
}

/// The metric queries of an integration (Datadog-format) widget request: its formulas with the
/// named queries substituted, or the queries themselves.
fn integration_queries(original: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for r in original["requests"].as_array().into_iter().flatten() {
        if let Some(q) = r["q"].as_str() {
            out.push(strip_display_functions(q));
            continue;
        }
        let named: Vec<(String, String)> = r["queries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|q| q["data_source"].as_str().unwrap_or("metrics") == "metrics")
            .filter_map(|q| Some((q["name"].as_str()?.to_string(), strip_display_functions(q["query"].as_str()?))))
            .collect();
        let formulas: Vec<&str> = r["formulas"].as_array().into_iter().flatten().filter_map(|f| f["formula"].as_str()).collect();
        if formulas.is_empty() {
            out.extend(named.into_iter().map(|(_, q)| q));
            continue;
        }
        for f in formulas {
            out.push(substitute_names(f, &named));
        }
    }
    out
}

/// Replaces whole-word query names in a formula with their queries.
fn substitute_names(formula: &str, named: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        match named.iter().find(|(n, _)| n == word) {
            Some((_, q)) => out.push_str(q),
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in formula.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The report's lines for a dashboard's widgets (groups flattened), with template variables resolved.
pub(crate) fn widget_targets(widgets: &[Value], vars: &[TemplateVariable], values: &HashMap<String, String>) -> Vec<WidgetTarget> {
    let mut out = Vec::new();
    for w in widgets {
        let d = &w["definition"];
        let kind = d["type"].as_str().unwrap_or("widget");
        if kind == "group" {
            out.extend(widget_targets(d["widgets"].as_array().map(Vec::as_slice).unwrap_or(&[]), vars, values));
            continue;
        }
        let (kind, queries, title) = match kind {
            "timeseries" => ("timeseries".to_string(), d["requests"].as_array().into_iter().flatten().filter_map(|r| r["query"].as_str()).map(str::to_string).collect(), d["title"].as_str()),
            "query_value" => ("query_value".to_string(), d["request"]["query"].as_str().map(|q| vec![q.to_string()]).unwrap_or_default(), d["title"].as_str()),
            "integration" => {
                let o = &d["original"];
                let okind = o["type"].as_str().unwrap_or("widget").to_string();
                let queries = if matches!(okind.as_str(), "timeseries" | "query_value") { integration_queries(o) } else { Vec::new() };
                let title = d["title"].as_str().filter(|t| !t.is_empty()).or_else(|| o["title"].as_str());
                (okind, queries, title)
            }
            other => (other.to_string(), Vec::new(), d["title"].as_str()),
        };
        let title = title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| default_title(&kind));
        let queries = queries.iter().map(|q| resolve_vars(q, vars, values)).collect();
        out.push(WidgetTarget { title, kind, queries });
    }
    out
}

// ================================================================ summary

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ReportSeriesSummary {
    /// The group (`service:billing`) or, ungrouped, the query.
    pub label: String,
    /// The last value in the frame.
    pub current: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ReportWidgetSummary {
    pub title: String,
    pub kind: String,
    /// Empty for widgets that are only listed.
    pub series: Vec<ReportSeriesSummary>,
    /// Why the widget's queries couldn't be summarized.
    pub error: Option<String>,
}

/// Current (last), minimum and maximum of a series' points.
pub(crate) fn series_stats(label: String, points: &[(i64, f64)]) -> ReportSeriesSummary {
    let finite: Vec<f64> = points.iter().map(|p| p.1).filter(|v| v.is_finite()).collect();
    ReportSeriesSummary { label, current: finite.last().copied(), min: finite.iter().copied().reduce(f64::min), max: finite.iter().copied().reduce(f64::max) }
}

fn number(v: Option<f64>) -> String {
    let trim = |s: String| s.trim_end_matches('0').trim_end_matches('.').to_string();
    match v {
        None => "–".into(),
        Some(v) if v.abs() >= 1e6 => format!("{v:.3e}"),
        Some(v) if v != 0.0 && v.abs() < 0.01 => trim(format!("{v:.6}")),
        Some(v) => trim(format!("{v:.2}")),
    }
}

/// The rendered report: email subject and body, Slack text and webhook payload.
pub(crate) struct RenderedReport {
    pub subject: String,
    pub body: String,
    pub slack: String,
}

pub(crate) struct ReportContext<'a> {
    pub report_name: &'a str,
    pub dashboard_title: &'a str,
    pub frame_label: &'a str,
    pub link: &'a str,
    pub message: &'a str,
}

pub(crate) fn render(ctx: &ReportContext, widgets: &[ReportWidgetSummary]) -> RenderedReport {
    let subject = format!("[Report] {} · {}", ctx.dashboard_title, ctx.frame_label);
    let mut lines = Vec::new();
    for w in widgets {
        if let Some(e) = &w.error {
            lines.push(format!("• {} ({}): couldn't be summarized: {e}", w.title, default_title(&w.kind).to_lowercase()));
        } else if w.series.is_empty() {
            lines.push(format!("• {} ({}): see the dashboard", w.title, default_title(&w.kind).to_lowercase()));
        } else {
            lines.push(format!("• {}", w.title));
            for s in &w.series {
                lines.push(format!("    {}: current {} · min {} · max {}", s.label, number(s.current), number(s.min), number(s.max)));
            }
        }
    }
    let message = ctx.message.trim();
    let intro = if message.is_empty() { String::new() } else { format!("{message}\n\n") };
    let body = format!(
        "{name}\n\nDashboard: {title}\nTime frame: {frame}\n\n{intro}{widgets}\n\nOpen the dashboard: {link}\n",
        name = ctx.report_name,
        title = ctx.dashboard_title,
        frame = ctx.frame_label,
        widgets = if lines.is_empty() { "This dashboard has no widgets.".to_string() } else { lines.join("\n") },
        link = ctx.link,
    );
    let slack = format!("*{subject}*\n{intro}{}\n<{}|Open the dashboard>", lines.join("\n"), ctx.link);
    RenderedReport { subject, body, slack }
}

// ================================================================ model

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardReport {
    pub id: String,
    pub name: String,
    pub dashboard_id: String,
    pub dashboard_title: String,
    pub template_values: BTreeMap<String, String>,
    /// 1h | 4h | 1d | 2d | 1w | 1mo
    pub time_frame: String,
    pub schedule: ReportSchedule,
    /// "Weekly on Monday at 09:00 (Europe/Paris)".
    pub schedule_label: String,
    pub recipients: Vec<String>,
    pub message: String,
    pub author: Option<super::dashboards::UserRef>,
    /// The current user receives it by email.
    pub subscribed: bool,
    pub subscriber_count: i64,
    pub next_run_at: Option<i64>,
    pub last_sent_at: Option<i64>,
    /// sent | partial | failed
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct ReportRow {
    id: String,
    name: String,
    dashboard_id: String,
    dashboard_title: String,
    template_values: Value,
    time_frame: String,
    schedule: Value,
    recipients: Vec<String>,
    message: String,
    author_id: Option<String>,
    author_name: Option<String>,
    subscribed: bool,
    subscriber_count: i64,
    next_run_at: Option<i64>,
    last_sent_at: Option<i64>,
    last_status: Option<String>,
    last_error: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl ReportRow {
    fn into_report(self) -> DashboardReport {
        let schedule: ReportSchedule =
            serde_json::from_value(self.schedule).unwrap_or(ReportSchedule { frequency: "daily".into(), time: "09:00".into(), weekday: None, month_day: None, timezone: "UTC".into() });
        DashboardReport {
            id: self.id,
            name: self.name,
            dashboard_id: self.dashboard_id,
            dashboard_title: self.dashboard_title,
            template_values: serde_json::from_value(self.template_values).unwrap_or_default(),
            time_frame: self.time_frame,
            schedule_label: schedule.describe(),
            schedule,
            recipients: self.recipients,
            message: self.message,
            author: self.author_id.map(|id| super::dashboards::UserRef { id, name: self.author_name.unwrap_or_default() }),
            subscribed: self.subscribed,
            subscriber_count: self.subscriber_count,
            next_run_at: self.next_run_at,
            last_sent_at: self.last_sent_at,
            last_status: self.last_status,
            last_error: self.last_error,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// `$2` is the current user (for `subscribed`); `$1` the org.
const SELECT: &str = "SELECT r.id::text, r.name, r.dashboard_id, d.title AS dashboard_title, r.template_values, r.time_frame, r.schedule, r.recipients, r.message, \
    u.id::text AS author_id, u.name AS author_name, \
    EXISTS (SELECT 1 FROM dashboard_report_subscriptions s WHERE s.report_id = r.id AND s.org_id = $1 AND s.user_id = $2::uuid) AS subscribed, \
    (SELECT count(*) FROM dashboard_report_subscriptions s WHERE s.report_id = r.id AND s.org_id = $1) AS subscriber_count, \
    (extract(epoch FROM r.next_run_at) * 1000)::bigint AS next_run_at, (extract(epoch FROM r.last_sent_at) * 1000)::bigint AS last_sent_at, r.last_status, r.last_error, \
    (extract(epoch FROM r.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM r.updated_at) * 1000)::bigint AS updated_at \
    FROM dashboard_reports r JOIN dashboards d ON d.id = r.dashboard_id AND d.org_id = r.org_id LEFT JOIN users u ON u.id = r.author_id \
    WHERE r.org_id = $1";

async fn load(st: &AppState, org: &str, user: &str, id: &str) -> Result<DashboardReport, ApiError> {
    let row: Option<ReportRow> = sqlx::query_as(&format!("{SELECT} AND r.id::text = $3")).bind(org).bind(user).bind(id).fetch_optional(&st.postgres).await?;
    row.map(ReportRow::into_report).ok_or_else(|| ApiError::NotFound(format!("Report {id} not found")))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardReportInput {
    pub name: String,
    pub dashboard_id: String,
    #[serde(default)]
    pub template_values: BTreeMap<String, String>,
    pub time_frame: String,
    pub schedule: ReportSchedule,
    #[serde(default)]
    pub recipients: Vec<String>,
    #[serde(default)]
    pub message: String,
}

/// Checks a report and normalizes its recipients (lowercase, no `@` prefix on handles, unique).
pub(crate) fn validate(input: &DashboardReportInput) -> Result<Vec<String>, String> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err("The report name must be 1–200 characters".into());
    }
    if time_frame(&input.time_frame).is_none() {
        return Err(format!("Unknown time frame {} (use one of {})", input.time_frame, TIME_FRAMES.iter().map(|t| t.0).collect::<Vec<_>>().join(", ")));
    }
    input.schedule.next_after(0)?;
    if input.message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!("The message is at most {MAX_MESSAGE_CHARS} characters"));
    }
    let mut out: Vec<String> = Vec::new();
    for raw in &input.recipients {
        let r = raw.trim();
        let r = r.strip_prefix('@').unwrap_or(r).to_lowercase();
        if r.is_empty() {
            continue;
        }
        let ok = if let Some((local, domain)) = r.split_once('@') {
            !local.is_empty() && domain.contains('.') && !domain.contains('@') && r.len() <= 254 && !r.contains(char::is_whitespace)
        } else {
            (r.starts_with("slack-") || r.starts_with("webhook-")) && r.len() <= 80 && r.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        };
        if !ok {
            return Err(format!("{raw} isn't an email address or a channel handle (@slack-…, @webhook-…)"));
        }
        if !out.contains(&r) {
            out.push(r);
        }
    }
    if out.len() > MAX_RECIPIENTS {
        return Err(format!("A report has at most {MAX_RECIPIENTS} recipients"));
    }
    Ok(out)
}

async fn check_dashboard(st: &AppState, org: &str, id: &str) -> Result<(), ApiError> {
    let found: Option<(String,)> = sqlx::query_as("SELECT id FROM dashboards WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL").bind(org).bind(id).fetch_optional(&st.postgres).await?;
    found.map(|_| ()).ok_or_else(|| ApiError::BadRequest(format!("Dashboard {id} not found")))
}

// ================================================================ handlers

/// The org's reports (the web app filters "Created by you" and "Subscribed").
#[utoipa::path(get, path = "/api/v1/dashboard-reports", operation_id = "list_dashboard_reports", responses((status = 200, body = Vec<DashboardReport>)), tag = "dashboards")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<DashboardReport>> {
    let rows: Vec<ReportRow> = sqlx::query_as(&format!("{SELECT} ORDER BY lower(r.name), r.created_at")).bind(&ctx.org_id).bind(&ctx.user_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(ReportRow::into_report).collect()))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardReportOptions {
    /// Email is configured on this install (`OC_SMTP_URL`); without it only channels receive reports.
    pub email_enabled: bool,
    /// The org's channel handles (`slack-checkout`).
    pub channels: Vec<String>,
    pub time_frames: Vec<DashboardReportTimeFrame>,
}

#[derive(Serialize, ToSchema)]
pub struct DashboardReportTimeFrame {
    pub key: String,
    pub label: String,
}

#[utoipa::path(get, path = "/api/v1/dashboard-reports/options", operation_id = "dashboard_report_options", responses((status = 200, body = DashboardReportOptions)), tag = "dashboards")]
pub async fn options(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<DashboardReportOptions> {
    let channels: Vec<(String,)> = sqlx::query_as("SELECT handle FROM notification_channels WHERE org_id = $1 ORDER BY handle").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(DashboardReportOptions {
        email_enabled: st.config.smtp_url.is_some(),
        channels: channels.into_iter().map(|(h,)| h).collect(),
        time_frames: TIME_FRAMES.iter().map(|(k, l, _)| DashboardReportTimeFrame { key: k.to_string(), label: l.to_string() }).collect(),
    }))
}

#[utoipa::path(get, path = "/api/v1/dashboard-reports/{id}", operation_id = "get_dashboard_report", params(("id" = String, Path)), responses((status = 200, body = DashboardReport), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<DashboardReport> {
    Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/dashboard-reports", operation_id = "create_dashboard_report", request_body = DashboardReportInput, responses((status = 200, body = DashboardReport), (status = 400, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<DashboardReportInput>) -> ApiResult<DashboardReport> {
    let recipients = validate(&input).map_err(ApiError::BadRequest)?;
    check_dashboard(&st, &ctx.org_id, &input.dashboard_id).await?;
    let next = input.schedule.next_after(super::metrics::now_ms()).map_err(ApiError::BadRequest)?;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO dashboard_reports (org_id, dashboard_id, name, template_values, time_frame, schedule, recipients, message, author_id, next_run_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::uuid, to_timestamp($10::double precision / 1000)) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.dashboard_id)
    .bind(input.name.trim())
    .bind(json!(input.template_values))
    .bind(&input.time_frame)
    .bind(json!(input.schedule))
    .bind(&recipients)
    .bind(input.message.trim())
    .bind(&ctx.user_id)
    .bind(next as f64)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "dashboard_report.created", input.name.trim(), &input.dashboard_id).await;
    Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/dashboard-reports/{id}", operation_id = "update_dashboard_report", params(("id" = String, Path)), request_body = DashboardReportInput, responses((status = 200, body = DashboardReport), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<DashboardReportInput>) -> ApiResult<DashboardReport> {
    let recipients = validate(&input).map_err(ApiError::BadRequest)?;
    check_dashboard(&st, &ctx.org_id, &input.dashboard_id).await?;
    let next = input.schedule.next_after(super::metrics::now_ms()).map_err(ApiError::BadRequest)?;
    let updated = sqlx::query(
        "UPDATE dashboard_reports SET dashboard_id = $3, name = $4, template_values = $5, time_frame = $6, schedule = $7, recipients = $8, message = $9, \
         next_run_at = to_timestamp($10::double precision / 1000), updated_at = now() WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.dashboard_id)
    .bind(input.name.trim())
    .bind(json!(input.template_values))
    .bind(&input.time_frame)
    .bind(json!(input.schedule))
    .bind(&recipients)
    .bind(input.message.trim())
    .bind(next as f64)
    .execute(&st.postgres)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Report {id} not found")));
    }
    ctx.audit(&st, "dashboard_report.updated", input.name.trim(), &id).await;
    Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/dashboard-reports/{id}", operation_id = "delete_dashboard_report", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM dashboard_reports WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound(format!("Report {id} not found"))) };
    ctx.audit(&st, "dashboard_report.deleted", &name, &id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct DashboardReportSubscription {
    pub subscribed: bool,
}

/// Subscribes the current user to the report (by email), or unsubscribes them.
#[utoipa::path(put, path = "/api/v1/dashboard-reports/{id}/subscription", operation_id = "set_dashboard_report_subscription", params(("id" = String, Path)), request_body = DashboardReportSubscription, responses((status = 200, body = DashboardReport), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn subscribe(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(req): Json<DashboardReportSubscription>) -> ApiResult<DashboardReport> {
    load(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    if req.subscribed {
        sqlx::query("INSERT INTO dashboard_report_subscriptions (report_id, org_id, user_id) SELECT r.id, r.org_id, $3::uuid FROM dashboard_reports r WHERE r.org_id = $1 AND r.id::text = $2 ON CONFLICT DO NOTHING")
            .bind(&ctx.org_id)
            .bind(&id)
            .bind(&ctx.user_id)
            .execute(&st.postgres)
            .await?;
    } else {
        sqlx::query("DELETE FROM dashboard_report_subscriptions WHERE org_id = $1 AND report_id::text = $2 AND user_id = $3::uuid")
            .bind(&ctx.org_id)
            .bind(&id)
            .bind(&ctx.user_id)
            .execute(&st.postgres)
            .await?;
    }
    Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardReportPreview {
    pub subject: String,
    pub body: String,
    pub widgets: Vec<ReportWidgetSummary>,
    pub range: TimeRange,
}

/// Summarizes a report's dashboard now: the message its recipients would receive.
async fn build(st: &AppState, org: &str, report: &DashboardReport, now: i64) -> Result<DashboardReportPreview, ApiError> {
    let def: Option<(Value,)> = sqlx::query_as("SELECT definition FROM dashboards WHERE org_id = $1 AND id = $2").bind(org).bind(&report.dashboard_id).fetch_optional(&st.postgres).await?;
    let Some((def,)) = def else { return Err(ApiError::NotFound(format!("Dashboard {} not found", report.dashboard_id))) };
    let vars: Vec<TemplateVariable> = serde_json::from_value(def["templateVariables"].clone()).unwrap_or_default();
    let widgets: Vec<Value> = def["widgets"].as_array().cloned().unwrap_or_default();
    let values: HashMap<String, String> = report.template_values.clone().into_iter().collect();
    let (frame_label, frame_ms) = time_frame(&report.time_frame).unwrap_or(("Past 1 Day", 86_400_000));
    let range = TimeRange { from: now - frame_ms, to: now };
    let mut summaries = Vec::new();
    for t in widget_targets(&widgets, &vars, &values) {
        let mut series = Vec::new();
        let mut error = None;
        for q in &t.queries {
            match super::metrics::evaluate_formula(st, org, q, range, 120).await {
                Ok((_, groups)) => {
                    let mut groups = groups;
                    // The busiest groups first.
                    groups.sort_by(|a, b| b.points.iter().map(|p| p.1).sum::<f64>().total_cmp(&a.points.iter().map(|p| p.1).sum::<f64>()));
                    for g in groups.into_iter().take(MAX_SERIES_PER_QUERY) {
                        let label = if g.tags.is_empty() { q.clone() } else { g.tags.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(", ") };
                        series.push(series_stats(label, &g.points));
                    }
                }
                Err(e) => error = Some(e.to_string()),
            }
        }
        if !t.queries.is_empty() && series.is_empty() && error.is_none() {
            series.push(series_stats(t.queries[0].clone(), &[]));
        }
        summaries.push(ReportWidgetSummary { title: t.title, kind: t.kind, series, error });
    }
    let link = dashboard_link(&st.config.public_url, &report.dashboard_id, &report.time_frame, &report.template_values);
    let rendered = render(&ReportContext { report_name: &report.name, dashboard_title: &report.dashboard_title, frame_label, link: &link, message: &report.message }, &summaries);
    Ok(DashboardReportPreview { subject: rendered.subject, body: rendered.body, widgets: summaries, range })
}

/// The dashboard over the report's frame, with its template variable values (`?tpl_var_env=prod`).
pub(crate) fn dashboard_link(base: &str, id: &str, frame: &str, values: &BTreeMap<String, String>) -> String {
    let mut url = format!("{base}/dashboards/{id}?time={frame}");
    for (k, v) in values {
        if v != "*" {
            url.push_str(&format!("&tpl_var_{}={}", url::form_urlencoded::byte_serialize(k.as_bytes()).collect::<String>(), url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()));
        }
    }
    url
}

#[utoipa::path(get, path = "/api/v1/dashboard-reports/{id}/preview", operation_id = "preview_dashboard_report", params(("id" = String, Path)), responses((status = 200, body = DashboardReportPreview), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn preview(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<DashboardReportPreview> {
    let report = load(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    Ok(Json(build(&st, &ctx.org_id, &report, super::metrics::now_ms()).await?))
}

#[derive(Serialize, ToSchema)]
pub struct DashboardReportDelivery {
    pub recipient: String,
    /// sent | failed | not_sent (email not configured, not a member, unknown channel)
    pub status: String,
    pub error: Option<String>,
}

/// The report's recipients plus its subscribers' emails.
async fn all_recipients(st: &AppState, org: &str, report: &DashboardReport) -> Result<Vec<String>, ApiError> {
    let subs: Vec<(String,)> = sqlx::query_as(
        "SELECT lower(u.email) FROM dashboard_report_subscriptions s JOIN users u ON u.id = s.user_id AND u.org_id = s.org_id WHERE s.org_id = $1 AND s.report_id::text = $2 AND u.status = 'active'",
    )
    .bind(org)
    .bind(&report.id)
    .fetch_all(&st.postgres)
    .await?;
    let mut out = report.recipients.clone();
    for (e,) in subs {
        if !out.contains(&e) {
            out.push(e);
        }
    }
    Ok(out)
}

/// Builds and sends the report; records the outcome on the report.
async fn send_report(st: &AppState, org: &str, report: &DashboardReport) -> Result<Vec<DashboardReportDelivery>, ApiError> {
    let built = build(st, org, report, super::metrics::now_ms()).await?;
    let link = dashboard_link(&st.config.public_url, &report.dashboard_id, &report.time_frame, &report.template_values);
    let (frame_label, _) = time_frame(&report.time_frame).unwrap_or(("Past 1 Day", 0));
    let slack = render(&ReportContext { report_name: &report.name, dashboard_title: &report.dashboard_title, frame_label, link: &link, message: &report.message }, &built.widgets).slack;
    let out = Outgoing {
        org: org.to_string(),
        title: built.subject.clone(),
        email_body: built.body.clone(),
        slack_text: slack,
        webhook: json!({
            "event": "dashboard_report",
            "title": built.subject,
            "organization": org,
            "report": { "id": report.id, "name": report.name },
            "dashboard": { "id": report.dashboard_id, "title": report.dashboard_title, "url": link },
            "range": { "from": built.range.from, "to": built.range.to },
            "widgets": built.widgets,
            "message": report.message,
        }),
    };
    let mut deliveries = Vec::new();
    for r in all_recipients(st, org, report).await? {
        let (status, error) = match send(st, &r, &out).await {
            Outcome::Sent => ("sent", None),
            Outcome::Unknown(e) => ("not_sent", Some(e)),
            Outcome::Failed(e) => ("failed", Some(e)),
        };
        deliveries.push(DashboardReportDelivery { recipient: r, status: status.into(), error });
    }
    let (status, error) = outcome(&deliveries);
    sqlx::query("UPDATE dashboard_reports SET last_sent_at = now(), last_status = $3, last_error = $4 WHERE org_id = $1 AND id::text = $2")
        .bind(org)
        .bind(&report.id)
        .bind(status)
        .bind(error)
        .execute(&st.postgres)
        .await?;
    Ok(deliveries)
}

/// The report's status after a send: sent (all), partial (some) or failed (none), and why.
pub(crate) fn outcome(deliveries: &[DashboardReportDelivery]) -> (&'static str, Option<String>) {
    let sent = deliveries.iter().filter(|d| d.status == "sent").count();
    let errors: Vec<String> = deliveries.iter().filter_map(|d| d.error.as_ref().map(|e| format!("{}: {e}", d.recipient))).collect();
    let status = if deliveries.is_empty() {
        "failed"
    } else if sent == deliveries.len() {
        "sent"
    } else if sent > 0 {
        "partial"
    } else {
        "failed"
    };
    let error = if deliveries.is_empty() {
        Some("The report has no recipients".to_string())
    } else if errors.is_empty() {
        None
    } else {
        Some(errors.join("; "))
    };
    (status, error)
}

/// Sends the report now to its recipients (emails only when email is configured).
#[utoipa::path(post, path = "/api/v1/dashboard-reports/{id}/send", operation_id = "send_dashboard_report", params(("id" = String, Path)), responses((status = 200, body = Vec<DashboardReportDelivery>), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn send_now(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<Vec<DashboardReportDelivery>> {
    let report = load(&st, &ctx.org_id, &ctx.user_id, &id).await?;
    let deliveries = send_report(&st, &ctx.org_id, &report).await?;
    ctx.audit(&st, "dashboard_report.sent", &report.name, &format!("{} recipients", deliveries.len())).await;
    Ok(Json(deliveries))
}

// ================================================================ scheduler

/// Claims due reports and moves them to their next send (rows are locked with SKIP LOCKED).
async fn claim(st: &AppState, limit: i64) -> Result<Vec<(String, String)>, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let due: Vec<(String, String, Value)> = sqlx::query_as(
        "SELECT r.org_id, r.id::text, r.schedule FROM dashboard_reports r JOIN orgs o ON o.id = r.org_id \
         WHERE r.next_run_at <= now() AND (r.claimed_until IS NULL OR r.claimed_until < now()) AND o.status = 'active' \
         ORDER BY r.next_run_at LIMIT $1 FOR UPDATE OF r SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let now = super::metrics::now_ms();
    for (org, id, schedule) in &due {
        let next = serde_json::from_value::<ReportSchedule>(schedule.clone()).ok().and_then(|s| s.next_after(now).ok());
        sqlx::query(
            "UPDATE dashboard_reports SET claimed_until = now() + make_interval(secs => $3), \
             next_run_at = CASE WHEN $4::double precision IS NULL THEN NULL ELSE to_timestamp($4::double precision / 1000) END WHERE org_id = $1 AND id::text = $2",
        )
        .bind(org)
        .bind(id)
        .bind(CLAIM_SECONDS)
        .bind(next.map(|n| n as f64))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(due.into_iter().map(|(org, id, _)| (org, id)).collect())
}

/// Sends due reports every minute (several servers can run it).
pub async fn run_scheduler(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let due = match claim(&st, 20).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(error = %e, "claiming dashboard reports failed");
                continue;
            }
        };
        for (org, id) in due {
            let result = match sqlx::query_scalar::<_, Option<String>>("SELECT author_id::text FROM dashboard_reports WHERE org_id = $1 AND id::text = $2")
                .bind(&org)
                .bind(&id)
                .fetch_optional(&st.postgres)
                .await
            {
                Ok(author) => {
                    let user = author.flatten().unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".into());
                    match load(&st, &org, &user, &id).await {
                        Ok(report) => send_report(&st, &org, &report).await.map(|_| ()),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e.into()),
            };
            if let Err(e) = &result {
                tracing::warn!(org = %org, report = %id, error = %e, "dashboard report failed");
                let _ = sqlx::query("UPDATE dashboard_reports SET last_status = 'failed', last_error = $3 WHERE org_id = $1 AND id::text = $2")
                    .bind(&org)
                    .bind(&id)
                    .bind(e.to_string())
                    .execute(&st.postgres)
                    .await;
            }
            let _ = sqlx::query("UPDATE dashboard_reports SET claimed_until = NULL WHERE org_id = $1 AND id::text = $2").bind(&org).bind(&id).execute(&st.postgres).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(frequency: &str, time: &str, weekday: Option<u8>, month_day: Option<u8>, tz: &str) -> ReportSchedule {
        ReportSchedule { frequency: frequency.into(), time: time.into(), weekday, month_day, timezone: tz.into() }
    }

    fn ms(rfc3339: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(rfc3339).unwrap().timestamp_millis()
    }

    fn at(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    #[test]
    fn schedules_fire_at_local_times() {
        // Daily 09:00 UTC, after 2026-10-28 10:00 UTC → the next day.
        assert_eq!(at(sched("daily", "09:00", None, None, "UTC").next_after(ms("2026-10-28T10:00:00Z")).unwrap()), "2026-10-29T09:00:00+00:00");
        // Weekly on Monday 08:30 in Paris (UTC+1 in November).
        assert_eq!(at(sched("weekly", "08:30", Some(1), None, "Europe/Paris").next_after(ms("2026-10-28T10:00:00Z")).unwrap()), "2026-11-02T07:30:00+00:00");
        // Monthly on the 1st at 06:00 UTC.
        assert_eq!(at(sched("monthly", "06:00", None, Some(1), "").next_after(ms("2026-10-28T10:00:00Z")).unwrap()), "2026-11-01T06:00:00+00:00");
    }

    #[test]
    fn schedules_are_checked() {
        assert!(sched("daily", "25:00", None, None, "UTC").next_after(0).is_err());
        assert!(sched("daily", "9:00", None, None, "UTC").next_after(0).is_err());
        assert!(sched("weekly", "09:00", None, None, "UTC").next_after(0).is_err());
        assert!(sched("monthly", "09:00", None, Some(31), "UTC").next_after(0).is_err());
        assert!(sched("hourly", "09:00", None, None, "UTC").next_after(0).is_err());
        assert!(sched("daily", "09:00", None, None, "Mars/Olympus").next_after(0).is_err());
        assert_eq!(sched("weekly", "09:00", Some(1), None, "Europe/Paris").describe(), "Weekly on Monday at 09:00 (Europe/Paris)");
        assert_eq!(sched("daily", "07:15", None, None, "").describe(), "Daily at 07:15 (UTC)");
    }

    fn tv(name: &str, prefix: &str, default: &str) -> TemplateVariable {
        TemplateVariable { name: name.into(), prefix: prefix.into(), default: default.into() }
    }

    #[test]
    fn template_variables_resolve_like_the_web_app() {
        let vars = [tv("env", "env", "prod"), tv("service", "service", "*"), tv("scope", "", "*")];
        let mut values = HashMap::new();
        values.insert("service".to_string(), "billing".to_string());
        assert_eq!(resolve_vars("avg:system.cpu.user{$env,$service} by {host}", &vars, &values), "avg:system.cpu.user{env:prod,service:billing} by {host}");
        values.insert("service".to_string(), "*".to_string());
        assert_eq!(resolve_vars("avg:system.cpu.user{$service}", &vars, &values), "avg:system.cpu.user{*}");
        assert_eq!(resolve_vars("sum:a{$env, region:eu} / sum:b{$service}", &vars, &values), "sum:a{env:prod,region:eu} / sum:b{*}");
        values.insert("scope".to_string(), "team:checkout".to_string());
        assert_eq!(resolve_vars("avg:m{$scope}", &vars, &values), "avg:m{team:checkout}");
        assert_eq!(resolve_vars("avg:m{host:$env.value-1}", &vars, &values), "avg:m{host:prod-1}");
        // Unknown variables are dropped from filters.
        assert_eq!(resolve_vars("avg:m{$nope}", &vars, &values), "avg:m{*}");
    }

    #[test]
    fn widgets_become_report_lines() {
        let widgets: Vec<Value> = serde_json::from_value(json!([
            { "id": "a", "definition": { "type": "timeseries", "title": "CPU", "requests": [{ "query": "avg:system.cpu.user{$env} by {host}" }], "display": "line" } },
            { "id": "b", "definition": { "type": "group", "title": "Group", "color": "blue", "widgets": [
                { "id": "c", "definition": { "type": "query_value", "request": { "query": "sum:orders.count{*}" }, "aggregator": "sum", "precision": 0, "formats": [] } },
                { "id": "d", "definition": { "type": "note", "content": "hi" } }
            ] } },
            { "id": "e", "definition": { "type": "integration", "original": { "type": "timeseries", "title": "Requests",
                "requests": [{ "queries": [{ "name": "query1", "data_source": "metrics", "query": "sum:nginx.requests{*}.as_rate()" }, { "name": "query2", "data_source": "metrics", "query": "sum:nginx.errors{*}" }],
                               "formulas": [{ "formula": "query2 / query1 * 100" }] }] } } },
            { "id": "f", "definition": { "type": "integration", "original": { "type": "query_value", "requests": [{ "q": "avg:redis.mem.used{*}.fill(null)" }] } } },
            { "id": "g", "definition": { "type": "integration", "original": { "type": "check_status", "title": "Up" } } }
        ]))
        .unwrap();
        let t = widget_targets(&widgets, &[tv("env", "env", "prod")], &HashMap::new());
        assert_eq!(
            t,
            vec![
                WidgetTarget { title: "CPU".into(), kind: "timeseries".into(), queries: vec!["avg:system.cpu.user{env:prod} by {host}".into()] },
                WidgetTarget { title: "Query value".into(), kind: "query_value".into(), queries: vec!["sum:orders.count{*}".into()] },
                WidgetTarget { title: "Note".into(), kind: "note".into(), queries: vec![] },
                WidgetTarget { title: "Requests".into(), kind: "timeseries".into(), queries: vec!["sum:nginx.errors{*} / sum:nginx.requests{*} * 100".into()] },
                WidgetTarget { title: "Query value".into(), kind: "query_value".into(), queries: vec!["avg:redis.mem.used{*}".into()] },
                WidgetTarget { title: "Up".into(), kind: "check_status".into(), queries: vec![] },
            ]
        );
    }

    #[test]
    fn stats_and_rendering() {
        let s = series_stats("host:i-0a1b2c3d4e5f60001".into(), &[(1, 3.0), (2, f64::NAN), (3, 1.5), (4, 2.25)]);
        assert_eq!(s, ReportSeriesSummary { label: "host:i-0a1b2c3d4e5f60001".into(), current: Some(2.25), min: Some(1.5), max: Some(3.0) });
        assert_eq!(series_stats("x".into(), &[]).current, None);
        assert_eq!((number(Some(0.0006)), number(Some(1234.5678)), number(Some(2.5e7)), number(None)), ("0.0006".into(), "1234.57".into(), "2.500e7".into(), "–".into()));
        let widgets = vec![
            ReportWidgetSummary { title: "CPU".into(), kind: "timeseries".into(), series: vec![s], error: None },
            ReportWidgetSummary { title: "Notes".into(), kind: "note".into(), series: vec![], error: None },
            ReportWidgetSummary { title: "Errors".into(), kind: "timeseries".into(), series: vec![], error: Some("Unknown metric: x".into()) },
        ];
        let r = render(
            &ReportContext {
                report_name: "Weekly checkout",
                dashboard_title: "Checkout overview",
                frame_label: "Past 1 Week",
                link: "https://oc.example.com/dashboards/d1?time=1w",
                message: "Hello team",
            },
            &widgets,
        );
        assert_eq!(r.subject, "[Report] Checkout overview · Past 1 Week");
        assert_eq!(
            r.body,
            "Weekly checkout\n\nDashboard: Checkout overview\nTime frame: Past 1 Week\n\nHello team\n\n\
             • CPU\n    host:i-0a1b2c3d4e5f60001: current 2.25 · min 1.5 · max 3\n\
             • Notes (note): see the dashboard\n\
             • Errors (timeseries): couldn't be summarized: Unknown metric: x\n\n\
             Open the dashboard: https://oc.example.com/dashboards/d1?time=1w\n"
        );
        assert!(r.slack.starts_with("*[Report] Checkout overview · Past 1 Week*\nHello team\n\n• CPU"));
        assert!(r.slack.ends_with("<https://oc.example.com/dashboards/d1?time=1w|Open the dashboard>"));
    }

    #[test]
    fn links_carry_the_frame_and_variables() {
        let mut v = BTreeMap::new();
        v.insert("env".to_string(), "prod".to_string());
        v.insert("service".to_string(), "*".to_string());
        v.insert("team".to_string(), "a b".to_string());
        assert_eq!(dashboard_link("https://oc.example.com", "d1", "1w", &v), "https://oc.example.com/dashboards/d1?time=1w&tpl_var_env=prod&tpl_var_team=a+b");
    }

    fn input(recipients: &[&str]) -> DashboardReportInput {
        DashboardReportInput {
            name: "Weekly".into(),
            dashboard_id: "d1".into(),
            template_values: BTreeMap::new(),
            time_frame: "1w".into(),
            schedule: sched("weekly", "09:00", Some(1), None, "UTC"),
            recipients: recipients.iter().map(|s| s.to_string()).collect(),
            message: String::new(),
        }
    }

    #[test]
    fn inputs_are_validated_and_recipients_normalized() {
        assert_eq!(validate(&input(&["Ops@Example.com", "@slack-checkout", "ops@example.com", " "])).unwrap(), vec!["ops@example.com", "slack-checkout"]);
        assert!(validate(&input(&["not an address"])).is_err());
        assert!(validate(&input(&["@oncall-checkout"])).is_err());
        assert!(validate(&input(&["a@b"])).is_err());
        let mut bad = input(&[]);
        bad.time_frame = "3y".into();
        assert!(validate(&bad).is_err());
        bad = input(&[]);
        bad.name = "  ".into();
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn send_outcomes() {
        let d = |r: &str, s: &str, e: Option<&str>| DashboardReportDelivery { recipient: r.into(), status: s.into(), error: e.map(str::to_string) };
        assert_eq!(outcome(&[d("slack-ops", "sent", None)]), ("sent", None));
        assert_eq!(outcome(&[d("slack-ops", "sent", None), d("ops@example.com", "not_sent", Some("Email isn't configured"))]), ("partial", Some("ops@example.com: Email isn't configured".into())));
        assert_eq!(outcome(&[d("webhook-x", "failed", Some("Unreachable"))]).0, "failed");
        assert_eq!(outcome(&[]), ("failed", Some("The report has no recipients".into())));
    }
}
