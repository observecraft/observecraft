//! Assist's tools: the org's telemetry and configuration, read (and changed, with approval) through
//! the same handlers as the UI, as the signed-in user: their org, their access level and their log
//! restrictions apply. Requests are built from JSON so the handlers' own defaults and validation hold.
//! Tools are grouped into skills, which a user can turn off.
use axum::extract::{Path, State};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::agent::Tools;
use super::llm::ToolSpec;
use crate::auth::{AuthContext, Writer};
use crate::error::ApiError;
use crate::routes::{apm, error_tracking, incidents, kubernetes, logs, metrics, monitors, security_signals};
use crate::state::{AppState, CurrentUser, OrgId};

/// A group of tools a user can turn on or off (Assist → Settings → Skills).
pub struct Skill {
    pub name: &'static str,
    pub description: &'static str,
}

pub const SKILLS: &[Skill] = &[
    Skill { name: "logs", description: "Search log events and find error patterns in them." },
    Skill { name: "metrics", description: "Query metrics with the metric query syntax (avg:system.cpu.user{env:prod} by {host})." },
    Skill { name: "apm", description: "Service health (requests, errors, latency) and traces of instrumented services." },
    Skill { name: "monitors", description: "Find monitors and their state, and create new ones (with your approval)." },
    Skill { name: "incidents", description: "Find incidents and declare new ones (with your approval)." },
    Skill { name: "errors", description: "Error Tracking issues: the errors services raise, grouped." },
    Skill { name: "kubernetes", description: "Kubernetes clusters, nodes, workloads and pods." },
    Skill { name: "security", description: "Cloud SIEM security signals: what fired, on which rule, entities and matching queries." },
    Skill { name: "code", description: "Read the code of the repositories added to Assist Code: list files, search and read them." },
];

struct Def {
    name: &'static str,
    skill: &'static str,
    writes: bool,
    description: &'static str,
    schema: fn() -> Value,
}

const RANGE_HELP: &str = "Relative to now (15m, 1h, 4h, 1d, 7d) or epoch milliseconds.";

fn range_props() -> Value {
    json!({
        "from": {"type": "string", "description": format!("Start. {RANGE_HELP} Default 1h.")},
        "to": {"type": "string", "description": format!("End. {RANGE_HELP} Default now.")},
    })
}

fn object(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn with_range(mut props: Value) -> Value {
    if let (Some(p), Value::Object(r)) = (props.as_object_mut(), range_props()) {
        p.extend(r);
    }
    props
}

const DEFS: &[Def] = &[
    Def {
        name: "search_logs",
        skill: "logs",
        writes: false,
        description: "Search log events, newest first. Query syntax: `service:checkout status:error \"timed out\" @http.status_code:>=500 -env:staging`.",
        schema: || object(with_range(json!({"query": {"type": "string"}, "limit": {"type": "integer", "description": "1-50, default 20"}})), &["query"]),
    },
    Def {
        name: "query_metrics",
        skill: "metrics",
        writes: false,
        description: "Query up to 5 metric series, e.g. `avg:kubernetes.cpu.usage.total{kube_namespace:billing} by {kube_deployment}`. Returns per series its last, min, max and average.",
        schema: || object(with_range(json!({"queries": {"type": "array", "items": {"type": "string"}, "maxItems": 5}})), &["queries"]),
    },
    Def {
        name: "list_services",
        skill: "apm",
        writes: false,
        description: "Instrumented services with their requests, error rate and latency in the range.",
        schema: || object(with_range(json!({"env": {"type": "string"}, "query": {"type": "string", "description": "Filter, e.g. service:checkout"}})), &[]),
    },
    Def {
        name: "search_traces",
        skill: "apm",
        writes: false,
        description: "Search spans of traces. Query syntax: `service:checkout status:error resource_name:\"POST /orders\" @duration:>1s`.",
        schema: || object(with_range(json!({"query": {"type": "string"}, "env": {"type": "string"}, "limit": {"type": "integer", "description": "1-50, default 20"}})), &["query"]),
    },
    Def {
        name: "search_monitors",
        skill: "monitors",
        writes: false,
        description: "Find monitors by name, tag or query text, with their state (OK, Warn, Alert, No Data).",
        schema: || object(json!({"query": {"type": "string"}, "statuses": {"type": "array", "items": {"type": "string", "enum": ["alert", "warn", "no_data", "ok"]}}}), &[]),
    },
    Def {
        name: "get_monitor",
        skill: "monitors",
        writes: false,
        description: "One monitor: its query, thresholds, message and the state of each group.",
        schema: || object(json!({"id": {"type": "integer"}}), &["id"]),
    },
    Def {
        name: "create_monitor",
        skill: "monitors",
        writes: true,
        description: "Create a monitor (the user approves it first). type metric: a metric query; apm: a trace.* metric query; log: `logs(\"service:checkout status:error\") by {service}`.",
        schema: || {
            object(
                json!({
                    "name": {"type": "string"},
                    "type": {"type": "string", "enum": ["metric", "apm", "log"]},
                    "query": {"type": "string"},
                    "comparator": {"type": "string", "enum": ["above", "above_or_equal", "below", "below_or_equal"]},
                    "critical": {"type": "number"},
                    "warning": {"type": "number"},
                    "window_minutes": {"type": "integer", "description": "Evaluation window, default 5"},
                    "time_aggregation": {"type": "string", "enum": ["avg", "sum", "min", "max", "last"]},
                    "message": {"type": "string", "description": "Notification text; @handles notify people or channels"},
                    "priority": {"type": "integer", "minimum": 1, "maximum": 5},
                    "tags": {"type": "array", "items": {"type": "string"}},
                }),
                &["name", "type", "query", "comparator", "critical"],
            )
        },
    },
    Def {
        name: "search_incidents",
        skill: "incidents",
        writes: false,
        description: "Find incidents declared in the range (default 30d), newest first.",
        schema: || object(with_range(json!({"query": {"type": "string"}, "statuses": {"type": "array", "items": {"type": "string", "enum": ["active", "stable", "resolved"]}}})), &[]),
    },
    Def {
        name: "get_incident",
        skill: "incidents",
        writes: false,
        description: "One incident with its timeline, by its number (IR-42 is 42).",
        schema: || object(json!({"id": {"type": "integer"}}), &["id"]),
    },
    Def {
        name: "declare_incident",
        skill: "incidents",
        writes: true,
        description: "Declare an incident (the user approves it first).",
        schema: || {
            object(
                json!({
                    "title": {"type": "string"},
                    "severity": {"type": "string", "enum": ["SEV-1", "SEV-2", "SEV-3", "SEV-4", "SEV-5", "UNKNOWN"]},
                    "summary": {"type": "string"},
                    "services": {"type": "array", "items": {"type": "string"}},
                }),
                &["title", "severity"],
            )
        },
    },
    Def {
        name: "search_error_issues",
        skill: "errors",
        writes: false,
        description: "Error Tracking issues (errors grouped by service, type and message), most frequent first.",
        schema: || object(with_range(json!({"query": {"type": "string", "description": "e.g. service:checkout"}, "limit": {"type": "integer", "description": "1-30, default 15"}})), &[]),
    },
    Def {
        name: "search_kubernetes",
        skill: "kubernetes",
        writes: false,
        description: "Kubernetes resources of a kind, with their status. Query by tags, e.g. `kube_namespace:billing`.",
        schema: || {
            object(
                json!({
                    "kind": {"type": "string", "enum": ["clusters", "nodes", "namespaces", "pods", "deployments", "daemonsets", "statefulsets", "jobs", "cronjobs", "services", "ingresses"]},
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "description": "1-50, default 25"},
                }),
                &["kind"],
            )
        },
    },
    Def {
        name: "search_security_signals",
        skill: "security",
        writes: false,
        description: "Cloud SIEM signals in a time range, newest first: rule, severity, state, entities. Query by text or tags.",
        schema: || object(json!({"query": {"type": "string"}, "state": {"type": "string", "enum": ["open", "under_review", "archived"]}, "from": {"type": "string", "description": RANGE_HELP}, "to": {"type": "string"}}), &[]),
    },
    Def {
        name: "get_security_signal",
        skill: "security",
        writes: false,
        description: "One signal in full: its rule, the log queries that matched, group values, entities and how often the rule fires.",
        schema: || object(json!({"id": {"type": "string"}}), &["id"]),
    },
    Def {
        name: "list_repo_files",
        skill: "code",
        writes: false,
        description: "Files of a repository added to Assist Code (owner/name), optionally under a directory, on its default branch unless a ref is given.",
        schema: || object(json!({"repo": {"type": "string"}, "path": {"type": "string", "description": "Directory, e.g. app/models"}, "ref": {"type": "string"}}), &["repo"]),
    },
    Def {
        name: "search_repo_code",
        skill: "code",
        writes: false,
        description: "Searches a repository's code for text (a function, an error message, a route); returns matching files.",
        schema: || object(json!({"repo": {"type": "string"}, "query": {"type": "string"}}), &["repo", "query"]),
    },
    Def {
        name: "read_repo_file",
        skill: "code",
        writes: false,
        description: "A file's content from a repository added to Assist Code.",
        schema: || object(json!({"repo": {"type": "string"}, "path": {"type": "string"}, "ref": {"type": "string"}}), &["repo", "path"]),
    },
];

/// Every tool's (name, skill, writes) for Settings and tests.
pub fn catalog() -> Vec<(&'static str, &'static str, bool)> {
    DEFS.iter().map(|d| (d.name, d.skill, d.writes)).collect()
}

/// A range (epoch ms) from a tool's `from` / `to`.
pub fn parse_range(input: &Value, now: i64, default_from: &str) -> Result<logs::TimeRange, String> {
    let at = |v: Option<&str>, default: &str| -> Result<i64, String> {
        let s = v.unwrap_or(default).trim();
        if s.is_empty() || s == "now" {
            return Ok(now);
        }
        if let Ok(ms) = s.parse::<i64>() {
            return Ok(ms);
        }
        let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).ok_or(format!("Bad time {s}: {RANGE_HELP}"))?);
        let n: i64 = n.parse().map_err(|_| format!("Bad time {s}: {RANGE_HELP}"))?;
        let ms = match unit {
            "m" | "min" => 60_000,
            "h" => 3_600_000,
            "d" => 86_400_000,
            "w" => 7 * 86_400_000,
            _ => return Err(format!("Bad time {s}: {RANGE_HELP}")),
        };
        Ok(now - n * ms)
    };
    let range = logs::TimeRange { from: at(input["from"].as_str(), default_from)?, to: at(input["to"].as_str(), "now")? };
    if range.from >= range.to {
        return Err("`from` must be before `to`".into());
    }
    Ok(range)
}

fn req<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("Invalid input: {e}"))
}

fn limit(input: &Value, default: u64, max: u64) -> u64 {
    input["limit"].as_u64().unwrap_or(default).clamp(1, max)
}

fn err(e: ApiError) -> String {
    match e {
        ApiError::BadRequest(m) => m,
        other => other.to_string(),
    }
}

/// A one-line log for the model: time, service, status and message.
fn log_line(l: &logs::LogEvent) -> String {
    let at = chrono::DateTime::from_timestamp_millis(l.timestamp).map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default();
    let message: String = l.message.chars().take(400).collect();
    format!("{at} {} {} {}", l.service, l.status, message.replace('\n', " ⏎ "))
}

/// Whether a user may use the tools that change something (the handlers' `Writer` extractor isn't
/// involved when Assist calls them).
pub fn may_write(access: crate::auth::Access) -> bool {
    access >= crate::auth::Access::Standard
}

/// The org's tools for one user, with the skills they turned off left out.
pub struct OrgTools {
    pub st: AppState,
    pub ctx: AuthContext,
    pub disabled: Vec<String>,
}

impl OrgTools {
    fn enabled(&self) -> impl Iterator<Item = &'static Def> + '_ {
        DEFS.iter().filter(|d| !self.disabled.iter().any(|s| s == d.skill))
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    async fn call(&self, name: &str, input: &Value) -> Result<Value, String> {
        let st = || State(self.st.clone());
        let org = || OrgId(self.ctx.org_id.clone());
        let now = Self::now();
        let to_json = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
        match name {
            "search_logs" => {
                let access = crate::log_access::for_user(&self.st, &self.ctx).await.map_err(err)?;
                let r: logs::LogSearchRequest = req(json!({"range": parse_range(input, now, "1h")?, "query": input["query"], "limit": limit(input, 20, 50)}))?;
                let page = logs::search(st(), org(), access, Json(r)).await.map_err(err)?.0;
                Ok(json!({"total": page.total, "logs": page.logs.iter().map(log_line).collect::<Vec<_>>()}))
            }
            "query_metrics" => {
                let queries: Vec<String> = input["queries"].as_array().into_iter().flatten().filter_map(Value::as_str).take(5).map(str::to_string).collect();
                if queries.is_empty() {
                    return Err("Give at least one query".into());
                }
                let r: metrics::TimeseriesRequest = req(json!({"queries": queries, "range": parse_range(input, now, "1h")?, "points": 30}))?;
                let series = metrics::timeseries(st(), org(), Json(r)).await.map_err(err)?.0;
                Ok(json!(series
                    .iter()
                    .take(40)
                    .map(|s| {
                        let values: Vec<f64> = s.points.iter().map(|p| p.1).filter(|v| v.is_finite()).collect();
                        let avg = if values.is_empty() { None } else { Some(values.iter().sum::<f64>() / values.len() as f64) };
                        json!({"series": s.name, "unit": s.unit, "last": values.last(), "min": values.iter().cloned().reduce(f64::min), "max": values.iter().cloned().reduce(f64::max), "avg": avg, "points": values.len()})
                    })
                    .collect::<Vec<_>>()))
            }
            "list_services" => {
                let r: apm::ApmRequest = req(json!({"range": parse_range(input, now, "1h")?, "env": input.get("env"), "query": input["query"].as_str().unwrap_or("")}))?;
                to_json(serde_json::to_value(apm::services(st(), org(), Json(r)).await.map_err(err)?.0))
            }
            "search_traces" => {
                let r: apm::TraceSearchRequest = req(json!({"range": parse_range(input, now, "1h")?, "query": input["query"], "env": input.get("env"), "limit": limit(input, 20, 50)}))?;
                to_json(serde_json::to_value(apm::search_traces(st(), org(), Json(r)).await.map_err(err)?.0))
            }
            "search_monitors" => {
                let r: monitors::MonitorSearchRequest = req(json!({"query": input["query"].as_str().unwrap_or(""), "statuses": input.get("statuses").cloned().unwrap_or(json!([])), "limit": 50}))?;
                to_json(serde_json::to_value(monitors::search(st(), org(), Json(r)).await.map_err(err)?.0))
            }
            "get_monitor" => {
                let id = input["id"].as_i64().ok_or("Give the monitor's id")?;
                to_json(serde_json::to_value(monitors::get_one(st(), org(), Path(id)).await.map_err(err)?.0))
            }
            "create_monitor" => {
                let r: monitors::MonitorInput = req(json!({
                    "name": input["name"],
                    "type": input["type"],
                    "query": input["query"],
                    "message": input["message"].as_str().unwrap_or(""),
                    "priority": input.get("priority"),
                    "tags": input.get("tags").cloned().unwrap_or(json!([])),
                    "options": {
                        "detection": "threshold",
                        "timeAggregation": input["time_aggregation"].as_str().unwrap_or("avg"),
                        "windowMinutes": input["window_minutes"].as_i64().unwrap_or(5),
                        "comparator": input["comparator"],
                        "thresholds": {"critical": input["critical"], "warning": input.get("warning")},
                    },
                }))?;
                let created = monitors::create(st(), Writer(self.ctx.clone()), Json(r)).await.map_err(err)?.0;
                to_json(serde_json::to_value(created))
            }
            "search_incidents" => {
                let r: incidents::IncidentSearchRequest = req(json!({"query": input["query"].as_str().unwrap_or(""), "statuses": input.get("statuses").cloned().unwrap_or(json!([])), "range": parse_range(input, now, "30d")?, "offset": 0, "limit": 25}))?;
                to_json(serde_json::to_value(incidents::search(st(), org(), CurrentUser(self.ctx.user_id.clone()), Json(r)).await.map_err(err)?.0))
            }
            "get_incident" => {
                let id = input["id"].as_i64().and_then(|i| i32::try_from(i).ok()).ok_or("Give the incident's number")?;
                to_json(serde_json::to_value(incidents::get_one(st(), org(), Path(id)).await.map_err(err)?.0))
            }
            "declare_incident" => {
                let r: incidents::DeclareRequest = req(json!({"title": input["title"], "severity": input["severity"], "summary": input["summary"].as_str().unwrap_or(""), "services": input.get("services").cloned().unwrap_or(json!([])), "detectionMethod": "assist"}))?;
                let declared = incidents::declare(st(), Writer(self.ctx.clone()), org(), CurrentUser(self.ctx.user_id.clone()), Json(r)).await.map_err(err)?.0;
                to_json(serde_json::to_value(declared))
            }
            "search_error_issues" => {
                let access = crate::log_access::for_user(&self.st, &self.ctx).await.map_err(err)?;
                let r: error_tracking::IssueSearchRequest = req(json!({"range": parse_range(input, now, "1d")?, "query": input["query"].as_str().unwrap_or(""), "limit": limit(input, 15, 30)}))?;
                to_json(serde_json::to_value(error_tracking::search_issues(st(), org(), access, Json(r)).await.map_err(err)?.0))
            }
            "search_kubernetes" => {
                let r: kubernetes::KubeSearchRequest = req(json!({"kind": input["kind"], "query": input["query"].as_str().unwrap_or(""), "limit": limit(input, 25, 50)}))?;
                to_json(serde_json::to_value(kubernetes::search(st(), org(), Json(r)).await.map_err(err)?.0))
            }
            "search_security_signals" => {
                let r: security_signals::SignalSearchRequest = req(json!({"range": parse_range(input, now, "1d")?, "query": input["query"].as_str().unwrap_or(""), "state": input["state"].as_str().unwrap_or(""), "limit": 25}))?;
                to_json(serde_json::to_value(security_signals::search(st(), self.ctx.clone(), Json(r)).await.map_err(err)?.0))
            }
            "get_security_signal" => {
                let id = input["id"].as_str().ok_or("Give the signal's id")?;
                to_json(serde_json::to_value(security_signals::get(st(), self.ctx.clone(), Path(id.to_string())).await.map_err(err)?.0))
            }
            "list_repo_files" | "search_repo_code" | "read_repo_file" => {
                let repo = input["repo"].as_str().ok_or("Give the repository (owner/name)")?;
                let f = crate::assist::code::forge_for(&self.st, &self.ctx.org_id, repo).await?;
                let reference = input["ref"].as_str().filter(|r| !r.is_empty());
                match name {
                    "list_repo_files" => {
                        let files = crate::assist::code::list_files(&self.st, &f, repo, input["path"].as_str().unwrap_or(""), reference).await?;
                        Ok(json!({"files": files.iter().take(500).collect::<Vec<_>>(), "total": files.len()}))
                    }
                    "search_repo_code" => Ok(json!({"matches": crate::assist::code::search(&self.st, &f, repo, input["query"].as_str().unwrap_or("")).await?})),
                    _ => Ok(json!({"path": input["path"], "content": crate::assist::code::read_file(&self.st, &f, repo, input["path"].as_str().unwrap_or(""), reference).await?})),
                }
            }
            other => Err(format!("Unknown tool {other}")),
        }
    }
}

impl Tools for OrgTools {
    fn specs(&self) -> Vec<ToolSpec> {
        self.enabled().map(|d| ToolSpec { name: d.name.to_string(), description: d.description.to_string(), schema: (d.schema)() }).collect()
    }

    fn writes(&self, name: &str) -> bool {
        DEFS.iter().any(|d| d.name == name && d.writes)
    }

    async fn run(&self, name: &str, input: &Value) -> Result<String, String> {
        if !self.enabled().any(|d| d.name == name) {
            return Err(format!("The {name} tool is turned off in Assist settings"));
        }
        if self.writes(name) && !may_write(self.ctx.access) {
            return Err(format!("{} has read-only access and can't use {name}", self.ctx.name));
        }
        self.call(name, input).await.map(|v| v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn ranges_are_relative_to_now_or_epoch_milliseconds() {
        let r = parse_range(&json!({}), NOW, "1h").unwrap();
        assert_eq!((r.from, r.to), (NOW - 3_600_000, NOW));
        let r = parse_range(&json!({"from": "7d", "to": "1d"}), NOW, "1h").unwrap();
        assert_eq!((r.from, r.to), (NOW - 7 * 86_400_000, NOW - 86_400_000));
        let r = parse_range(&json!({"from": "15m"}), NOW, "1h").unwrap();
        assert_eq!(r.from, NOW - 900_000);
        let r = parse_range(&json!({"from": "1799990000000", "to": "now"}), NOW, "1h").unwrap();
        assert_eq!((r.from, r.to), (1_799_990_000_000, NOW));
        assert!(parse_range(&json!({"from": "yesterday"}), NOW, "1h").unwrap_err().starts_with("Bad time yesterday"));
        assert!(parse_range(&json!({"from": "1h", "to": "2h"}), NOW, "1h").is_err());
    }

    #[test]
    fn read_only_users_cant_change_anything() {
        use crate::auth::Access;
        assert!(!may_write(Access::ReadOnly));
        assert!(may_write(Access::Standard) && may_write(Access::Admin));
    }

    #[test]
    fn every_tool_belongs_to_a_skill_and_has_an_object_schema() {
        let mut names = std::collections::HashSet::new();
        for d in DEFS {
            assert!(names.insert(d.name), "{} twice", d.name);
            assert!(SKILLS.iter().any(|s| s.name == d.skill), "{}: unknown skill {}", d.name, d.skill);
            let schema = (d.schema)();
            assert_eq!(schema["type"], "object", "{}", d.name);
            for r in schema["required"].as_array().unwrap() {
                assert!(schema["properties"].get(r.as_str().unwrap()).is_some(), "{}: required {r} has no property", d.name);
            }
        }
        // Only these change something, so only they wait for approval.
        let writes: Vec<_> = catalog().into_iter().filter(|c| c.2).map(|c| c.0).collect();
        assert_eq!(writes, vec!["create_monitor", "declare_incident"]);
    }
}
