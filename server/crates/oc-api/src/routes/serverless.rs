//! Serverless (AWS Lambda): the organization's functions, from the telemetry they already send:
//! - the Datadog Lambda extension's (or Forwarder's) enhanced metrics `aws.lambda.enhanced.*`:
//!   distributions (exponential histograms after the intake) or counts, tagged `functionname`,
//!   `region`, `account_id`, `memorysize`, `runtime`, `cold_start`, `executedversion`, …; durations
//!   in seconds, memory in MB, cost in USD;
//! - the AWS integration's metrics `aws.lambda.*` (invocations, errors, throttles, duration in ms);
//! - OpenTelemetry FaaS metrics (`faas.invocations`, `faas.errors`, `faas.coldstarts`, `faas.timeouts`,
//!   `faas.invoke_duration`, `faas.mem_usage`);
//! - Lambda spans: the Datadog tracer's `aws.lambda` span (`functionname`, `function_arn`, `cold_start`,
//!   …) and OpenTelemetry's FaaS spans (resource `faas.name`, `cloud.region`, `faas.max_memory`; span
//!   `faas.coldstart`, `faas.trigger`).
//!
//! Per value the most precise source wins: enhanced metrics, then integration metrics, then
//! OpenTelemetry metrics, then spans (which may be sampled). Like the GPU fleet, the functions are
//! few enough to merge in memory.
//!
//! Tenancy: every query filters on `OrgId = {oc_org:String}` (`otel.*`) or goes through `logs::scope`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use axum::extract::State;
use axum::Json;
use oc_query::{matches, parse_query, Fields, Param, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, scope, TimeRange};
use super::metric_inventory::{chart_step, series_values, SeriesValue};
use super::metrics::{exp_histogram_subquery, name_list, with_window};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::{AppState, OrgId};

const MIB: f64 = 1024.0 * 1024.0;
/// Most rows one query reads.
const MAX_ROWS: u64 = 200_000;
/// Functions charted in the summary graphs.
const MAX_CHARTED: usize = 10;

// ---------------------------------------------------------------- what the sources measure

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Field {
    Invocations,
    Errors,
    ColdStarts,
    Timeouts,
    OutOfMemory,
    Throttles,
    /// Average duration (ms).
    DurationAvg,
    /// p95 duration (ms).
    DurationP95,
    /// Peak memory used (MB).
    MemoryUsed,
    /// USD.
    Cost,
}

impl Field {
    /// Counts and cost add up across series; durations average (weighted); memory takes the peak.
    fn combine(self) -> Combine {
        match self {
            Field::DurationAvg | Field::DurationP95 => Combine::Mean,
            Field::MemoryUsed => Combine::Max,
            _ => Combine::Sum,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Combine {
    Sum,
    Mean,
    Max,
}

/// Where a function's numbers come from, in order of precedence.
#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LambdaSource {
    /// `aws.lambda.enhanced.*` from the Datadog Lambda extension or Forwarder.
    EnhancedMetrics,
    /// `aws.lambda.*` from the AWS integration.
    IntegrationMetrics,
    /// OpenTelemetry `faas.*` metrics.
    OtelMetrics,
    /// Lambda spans (Datadog tracer or OpenTelemetry).
    Traces,
}

fn source_of(metric: &str) -> LambdaSource {
    if metric.starts_with("aws.lambda.enhanced.") {
        LambdaSource::EnhancedMetrics
    } else if metric.starts_with("aws.lambda.") {
        LambdaSource::IntegrationMetrics
    } else {
        LambdaSource::OtelMetrics
    }
}

/// A gauge or sum metric → field: `Total` turns the per-second rate back into a count over the window;
/// `Scaled` keeps the value (× the factor to ms / MB).
#[derive(Clone, Copy)]
enum Conv {
    Total,
    Scaled(f64),
}

const SERIES_METRICS: &[(&str, Field, Conv)] = &[
    ("aws.lambda.enhanced.invocations", Field::Invocations, Conv::Total),
    ("aws.lambda.enhanced.errors", Field::Errors, Conv::Total),
    ("aws.lambda.enhanced.timeouts", Field::Timeouts, Conv::Total),
    ("aws.lambda.enhanced.out_of_memory", Field::OutOfMemory, Conv::Total),
    ("aws.lambda.enhanced.estimated_cost", Field::Cost, Conv::Total),
    ("aws.lambda.enhanced.duration", Field::DurationAvg, Conv::Scaled(1000.0)),
    ("aws.lambda.enhanced.max_memory_used", Field::MemoryUsed, Conv::Scaled(1.0)),
    ("aws.lambda.invocations", Field::Invocations, Conv::Total),
    ("aws.lambda.errors", Field::Errors, Conv::Total),
    ("aws.lambda.throttles", Field::Throttles, Conv::Total),
    ("aws.lambda.duration", Field::DurationAvg, Conv::Scaled(1.0)),
    ("aws.lambda.duration.p95", Field::DurationP95, Conv::Scaled(1.0)),
    ("faas.invocations", Field::Invocations, Conv::Total),
    ("faas.errors", Field::Errors, Conv::Total),
    ("faas.coldstarts", Field::ColdStarts, Conv::Total),
    ("faas.timeouts", Field::Timeouts, Conv::Total),
    ("faas.mem_usage", Field::MemoryUsed, Conv::Scaled(1.0 / MIB)),
];

/// What a distribution (exponential histogram) measures.
#[derive(Clone, Copy, PartialEq, Debug)]
enum DistConv {
    /// The sum of the values is the field (1 per invocation, error, …; USD per invocation).
    Sum(Field),
    /// A duration in seconds: average and p95.
    Duration,
    /// One value per cold start (init duration): the count is the cold starts.
    ColdStartCount,
    /// One value per invocation (duration in seconds): the count is the invocations, plus the durations.
    InvocationDuration,
    /// Peak memory in MB (`scale` 1) or bytes.
    Memory(f64),
}

const DIST_METRICS: &[(&str, DistConv)] = &[
    ("aws.lambda.enhanced.invocations", DistConv::Sum(Field::Invocations)),
    ("aws.lambda.enhanced.errors", DistConv::Sum(Field::Errors)),
    ("aws.lambda.enhanced.timeouts", DistConv::Sum(Field::Timeouts)),
    ("aws.lambda.enhanced.out_of_memory", DistConv::Sum(Field::OutOfMemory)),
    ("aws.lambda.enhanced.estimated_cost", DistConv::Sum(Field::Cost)),
    ("aws.lambda.enhanced.duration", DistConv::Duration),
    ("aws.lambda.enhanced.init_duration", DistConv::ColdStartCount),
    ("aws.lambda.enhanced.max_memory_used", DistConv::Memory(1.0)),
    ("faas.invoke_duration", DistConv::InvocationDuration),
    ("faas.init_duration", DistConv::ColdStartCount),
    ("faas.mem_usage", DistConv::Memory(1.0 / MIB)),
];

fn series_names() -> Vec<&'static str> {
    SERIES_METRICS.iter().map(|(n, _, _)| *n).collect()
}

fn dist_names() -> Vec<&'static str> {
    DIST_METRICS.iter().map(|(n, _)| *n).collect()
}

/// Canonical tag → the keys senders use (Datadog tags first, then OpenTelemetry attributes).
const TAGS: &[(&str, &[&str])] = &[
    ("functionname", &["functionname", "function_name", "faas.name"]),
    ("region", &["region", "cloud.region", "aws.region"]),
    ("aws_account", &["account_id", "aws_account", "cloud.account.id"]),
    ("runtime", &["runtime", "faas.runtime", "aws.lambda.runtime"]),
    ("memorysize", &["memorysize", "faas.max_memory"]),
    ("env", &["env", "deployment.environment", "deployment.environment.name"]),
    ("service", &["service", "service.name"]),
    ("stage", &["stage"]),
    ("team", &["team"]),
    ("version", &["executedversion", "function_version", "faas.version"]),
    ("architecture", &["architecture", "host.arch"]),
    ("function_arn", &["function_arn", "faas.id", "cloud.resource_id"]),
    ("datadog_lambda", &["datadog_lambda"]),
    ("dd_extension_version", &["dd_extension_version"]),
];

/// Every sender key of [`TAGS`] plus the span attributes read for invocations.
fn tag_keys() -> Vec<&'static str> {
    let mut keys: Vec<&str> = TAGS.iter().flat_map(|(_, ks)| ks.iter().copied()).collect();
    keys.extend(["cold_start", "faas.coldstart"]);
    keys
}

fn lookup(attrs: &HashMap<String, String>, res: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| attrs.get(*k).or_else(|| res.get(*k)).filter(|v| !v.is_empty()).cloned())
}

/// `arn:aws:lambda:<region>:<account>:function:<name>[:<qualifier>]` → (region, account, name).
pub(crate) fn parse_arn(arn: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = arn.split(':').collect();
    if parts.len() < 7 || parts[0] != "arn" || parts[2] != "lambda" || parts[5] != "function" || parts[6].is_empty() {
        return None;
    }
    Some((parts[3].to_string(), parts[4].to_string(), parts[6].to_string()))
}

/// Memory size in MB: Datadog's `memorysize` is MB; OpenTelemetry's `faas.max_memory` is bytes in current
/// semantic conventions (MB in older ones).
pub(crate) fn memory_mb(raw: &str) -> Option<f64> {
    let v: f64 = raw.trim().parse().ok()?;
    if v <= 0.0 {
        return None;
    }
    Some(if v >= 1_000_000.0 { (v / MIB).round() } else { v })
}

/// The canonical tags of one series or span group.
fn canonical_tags(attrs: &HashMap<String, String>, res: &HashMap<String, String>) -> BTreeMap<&'static str, String> {
    let mut out: BTreeMap<&'static str, String> = BTreeMap::new();
    for (key, keys) in TAGS {
        if let Some(v) = lookup(attrs, res, keys) {
            out.insert(key, v);
        }
    }
    if let Some((region, account, name)) = out.get("function_arn").and_then(|a| parse_arn(a)) {
        out.entry("region").or_insert(region);
        out.entry("aws_account").or_insert(account);
        out.entry("functionname").or_insert(name);
    }
    if let Some(mb) = out.get("memorysize").and_then(|m| memory_mb(m)) {
        out.insert("memorysize", format!("{mb}"));
    }
    out
}

// ---------------------------------------------------------------- inputs

/// One distribution series in a window (or interval): count, sum, max and its merged buckets.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DistValue {
    pub metric: String,
    pub res: HashMap<String, String>,
    pub attrs: HashMap<String, String>,
    pub t: i64,
    pub count: u64,
    pub sum: f64,
    pub max: f64,
    pub scale: i32,
    /// (bucket index, count): bucket i holds values in (base^i, base^(i+1)], base = 2^(2^-scale).
    pub buckets: Vec<(i32, u64)>,
}

/// Lambda spans of one function (and interval).
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct SpanStats {
    pub res: HashMap<String, String>,
    pub attrs: HashMap<String, String>,
    pub t: i64,
    pub hits: u64,
    pub errors: u64,
    pub cold: u64,
    pub avg_ms: f64,
    pub p95_ms: f64,
}

/// A function version seen in spans: when it was first and last seen (epoch ms).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VersionSeen {
    pub function: String,
    pub version: String,
    pub first: i64,
    pub last: i64,
}

/// Quantile `q` of an exponential histogram (linear inside the bucket).
pub(crate) fn exp_quantile(scale: i32, buckets: &[(i32, u64)], q: f64) -> Option<f64> {
    let total: u64 = buckets.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return None;
    }
    let mut sorted = buckets.to_vec();
    sorted.sort_by_key(|(i, _)| *i);
    let base = 2f64.powf(2f64.powi(-scale));
    let rank = q.clamp(0.0, 1.0) * total as f64;
    let mut seen = 0.0;
    for (i, n) in sorted {
        if n == 0 {
            continue;
        }
        let next = seen + n as f64;
        if next >= rank {
            let (lo, hi) = (base.powi(i), base.powi(i + 1));
            return Some(lo + (hi - lo) * ((rank - seen) / n as f64).clamp(0.0, 1.0));
        }
        seen = next;
    }
    None
}

// ---------------------------------------------------------------- merging

#[derive(Default, Clone, Copy, Debug)]
struct Agg {
    sum: f64,
    weight: f64,
    max: f64,
}

#[derive(Default)]
struct Acc {
    tags: BTreeMap<&'static str, String>,
    obs: HashMap<(Field, LambdaSource), Agg>,
    sources: BTreeSet<LambdaSource>,
}

impl Acc {
    fn add(&mut self, field: Field, source: LambdaSource, value: f64, weight: f64) {
        if !value.is_finite() {
            return;
        }
        let a = self.obs.entry((field, source)).or_default();
        a.sum += value * weight;
        a.weight += weight;
        a.max = a.max.max(value);
        self.sources.insert(source);
    }

    /// The field from the most precise source that has it.
    fn get(&self, field: Field) -> Option<f64> {
        [LambdaSource::EnhancedMetrics, LambdaSource::IntegrationMetrics, LambdaSource::OtelMetrics, LambdaSource::Traces].iter().find_map(|s| {
            let a = self.obs.get(&(field, *s))?;
            match field.combine() {
                Combine::Sum => Some(a.sum),
                Combine::Mean => (a.weight > 0.0).then(|| a.sum / a.weight),
                Combine::Max => Some(a.max),
            }
        })
    }

    fn merge_tags(&mut self, tags: BTreeMap<&'static str, String>) {
        for (k, v) in tags {
            self.tags.entry(k).or_insert(v);
        }
    }
}

fn function_key(tags: &BTreeMap<&'static str, String>) -> Option<String> {
    let name = tags.get("functionname")?.to_lowercase();
    Some(format!("{}/{name}", tags.get("region").map(String::as_str).unwrap_or("")))
}

/// The accumulator of the function the tags name (none without a function name).
fn entry<'a>(accs: &'a mut BTreeMap<String, Acc>, tags: BTreeMap<&'static str, String>) -> Option<&'a mut Acc> {
    let key = function_key(&tags)?;
    let acc = accs.entry(key).or_default();
    acc.merge_tags(tags);
    Some(acc)
}

/// Accumulators per function key, from every source (one window or one interval).
fn accumulate(series: &[SeriesValue], dists: &[DistValue], spans: &[SpanStats], window_ms: i64) -> BTreeMap<String, Acc> {
    let window_s = (window_ms as f64 / 1000.0).max(0.001);
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    for v in series {
        let Some((_, field, conv)) = SERIES_METRICS.iter().find(|(n, _, _)| *n == v.metric) else { continue };
        let source = source_of(&v.metric);
        let cold = v.tag(&["cold_start"]).is_some_and(|c| c == "true") && *field == Field::Invocations;
        let Some(acc) = entry(&mut accs, canonical_tags(&v.attrs, &v.res)) else { continue };
        match conv {
            Conv::Total => {
                let total = v.value * window_s;
                acc.add(*field, source, total, 1.0);
                if cold {
                    acc.add(Field::ColdStarts, source, total, 1.0);
                }
            }
            Conv::Scaled(f) => acc.add(*field, source, v.value * f, v.count.max(1) as f64),
        }
    }
    for d in dists {
        let Some((_, conv)) = DIST_METRICS.iter().find(|(n, _)| *n == d.metric) else { continue };
        let source = source_of(&d.metric);
        let cold = d.attrs.get("cold_start").or_else(|| d.res.get("cold_start")).is_some_and(|c| c == "true");
        let Some(acc) = entry(&mut accs, canonical_tags(&d.attrs, &d.res)) else { continue };
        let n = d.count as f64;
        let durations = |acc: &mut Acc| {
            if d.count > 0 {
                acc.add(Field::DurationAvg, source, d.sum / n * 1000.0, n);
                if let Some(p) = exp_quantile(d.scale, &d.buckets, 0.95) {
                    acc.add(Field::DurationP95, source, p * 1000.0, n);
                }
            }
        };
        match conv {
            DistConv::Sum(field) => {
                acc.add(*field, source, d.sum, 1.0);
                if cold && *field == Field::Invocations {
                    acc.add(Field::ColdStarts, source, d.sum, 1.0);
                }
            }
            DistConv::Duration => durations(acc),
            DistConv::ColdStartCount => acc.add(Field::ColdStarts, source, n, 1.0),
            DistConv::InvocationDuration => {
                acc.add(Field::Invocations, source, n, 1.0);
                durations(acc);
            }
            DistConv::Memory(f) => {
                if d.count > 0 {
                    acc.add(Field::MemoryUsed, source, d.max * f, 1.0);
                }
            }
        }
    }
    for s in spans {
        let Some(acc) = entry(&mut accs, canonical_tags(&s.attrs, &s.res)) else { continue };
        let src = LambdaSource::Traces;
        let hits = s.hits as f64;
        acc.add(Field::Invocations, src, hits, 1.0);
        acc.add(Field::Errors, src, s.errors as f64, 1.0);
        acc.add(Field::ColdStarts, src, s.cold as f64, 1.0);
        if s.hits > 0 {
            acc.add(Field::DurationAvg, src, s.avg_ms, hits);
            acc.add(Field::DurationP95, src, s.p95_ms, hits);
        }
    }
    // A function seen without a region joins the one region it is seen in elsewhere.
    let regionless: Vec<String> = accs.keys().filter(|k| k.starts_with('/')).cloned().collect();
    for key in regionless {
        let suffix = key.clone();
        let homes: Vec<String> = accs.keys().filter(|k| !k.starts_with('/') && k.ends_with(&suffix)).cloned().collect();
        if homes.len() == 1 {
            if let Some(from) = accs.remove(&key) {
                let home = accs.get_mut(&homes[0]).expect("listed above");
                for ((field, source), a) in from.obs {
                    let to = home.obs.entry((field, source)).or_default();
                    to.sum += a.sum;
                    to.weight += a.weight;
                    to.max = to.max.max(a.max);
                }
                home.sources.extend(from.sources);
                home.merge_tags(from.tags);
            }
        }
    }
    accs
}

// ---------------------------------------------------------------- derived values

/// AWS Lambda list prices (USD): per request and per GB-second, by architecture.
const PRICE_PER_REQUEST: f64 = 0.20 / 1_000_000.0;
const PRICE_GB_S_X86: f64 = 0.000_016_666_7;
const PRICE_GB_S_ARM: f64 = 0.000_013_333_4;

/// Estimated cost of `invocations` of `avg_ms` (billed per started ms) with `memory_mb` allocated.
pub(crate) fn estimate_cost(invocations: f64, avg_ms: f64, memory_mb: f64, architecture: Option<&str>) -> f64 {
    let price = if architecture.is_some_and(|a| a.starts_with("arm") || a.starts_with("aarch")) { PRICE_GB_S_ARM } else { PRICE_GB_S_X86 };
    let gb_s = invocations * (avg_ms.max(1.0).ceil() / 1000.0) * (memory_mb / 1024.0);
    invocations * PRICE_PER_REQUEST + gb_s * price
}

/// The newest runtime of each family AWS Lambda offers (update as AWS adds runtimes).
const LATEST_RUNTIMES: &[(&str, &str)] =
    &[("nodejs", "nodejs24.x"), ("python", "python3.14"), ("java", "java25"), ("dotnet", "dotnet10"), ("ruby", "ruby3.4"), ("provided", "provided.al2023"), ("go", "provided.al2023")];

/// `v4.12.0`, `3.9`, `18.x` → numeric parts.
pub(crate) fn version_parts(v: &str) -> Vec<u32> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect()
}

fn runtime_version(runtime: &str, family: &str) -> Vec<u32> {
    let rest = runtime.trim_start_matches(family);
    // `java8.al2` / `provided.al2023`: the Amazon Linux suffix is the version of `provided` only.
    let rest = if family == "provided" { rest } else { rest.split(".al").next().unwrap_or(rest) };
    version_parts(rest)
}

/// The runtime to move to when `runtime` is not its family's newest.
pub(crate) fn runtime_upgrade(runtime: &str) -> Option<&'static str> {
    let runtime = runtime.trim().to_lowercase();
    let (family, latest) = LATEST_RUNTIMES.iter().find(|(f, _)| runtime.starts_with(f))?;
    if *family == "go" {
        return Some(latest);
    }
    let latest_family = if *family == "provided" { "provided" } else { family };
    (runtime_version(&runtime, family) < runtime_version(latest, latest_family)).then_some(latest)
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LambdaInsightKind {
    /// 5 % or more of the invocations failed.
    HighErrors,
    /// 10 % or more of the invocations were cold starts.
    ColdStarts,
    Timeouts,
    OutOfMemory,
    Throttles,
    /// Peak memory at 90 % or more of the allocation.
    HighMemory,
    /// Peak memory at 20 % or less of an allocation of 512 MB or more.
    OverProvisioned,
}

#[derive(Serialize, ToSchema, Clone, PartialEq, Debug)]
pub struct LambdaInsight {
    pub kind: LambdaInsightKind,
    /// The measure behind it: a percentage for rates and memory, a count otherwise.
    pub value: f64,
}

const HIGH_ERROR_PCT: f64 = 5.0;
const COLD_START_PCT: f64 = 10.0;
/// Fewer invocations than this say nothing about the cold start rate.
const COLD_START_MIN_INVOCATIONS: f64 = 10.0;
const HIGH_MEMORY_PCT: f64 = 90.0;
const LOW_MEMORY_PCT: f64 = 20.0;
const OVERPROVISIONED_MIN_MB: f64 = 512.0;

fn pct(part: Option<f64>, whole: Option<f64>) -> Option<f64> {
    match (part, whole) {
        (Some(p), Some(w)) if w > 0.0 => Some(round1(p / w * 100.0)),
        _ => None,
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub(crate) fn detect_insights(f: &LambdaFunction) -> Vec<LambdaInsight> {
    let mut out = Vec::new();
    let mut push = |kind, value: f64| out.push(LambdaInsight { kind, value });
    if let (Some(rate), Some(errors)) = (f.error_rate_pct, f.errors) {
        if errors >= 1.0 && rate >= HIGH_ERROR_PCT {
            push(LambdaInsightKind::HighErrors, rate);
        }
    }
    if let (Some(rate), Some(inv)) = (f.cold_start_pct, f.invocations) {
        if inv >= COLD_START_MIN_INVOCATIONS && rate >= COLD_START_PCT {
            push(LambdaInsightKind::ColdStarts, rate);
        }
    }
    for (kind, v) in [(LambdaInsightKind::Timeouts, f.timeouts), (LambdaInsightKind::OutOfMemory, f.out_of_memory), (LambdaInsightKind::Throttles, f.throttles)] {
        if let Some(n) = v.filter(|n| *n >= 1.0) {
            push(kind, n);
        }
    }
    if let Some(used) = f.memory_used_pct {
        if used >= HIGH_MEMORY_PCT && f.out_of_memory.unwrap_or(0.0) < 1.0 {
            push(LambdaInsightKind::HighMemory, used);
        } else if used <= LOW_MEMORY_PCT && f.memory_mb.is_some_and(|m| m >= OVERPROVISIONED_MIN_MB) {
            push(LambdaInsightKind::OverProvisioned, used);
        }
    }
    out
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LambdaComponent {
    Runtime,
    /// The Datadog Lambda library (layer).
    DatadogLambda,
    /// The Datadog Lambda extension.
    Extension,
}

#[derive(Serialize, ToSchema, Clone, PartialEq, Debug)]
pub struct LambdaUpgrade {
    pub component: LambdaComponent,
    pub current: String,
    /// The newest runtime, or the newest library / extension version used by the organization's functions.
    pub latest: String,
}

/// Outdated runtimes, and Datadog libraries / extensions older than the newest the fleet already runs.
pub(crate) fn detect_upgrades(functions: &mut [LambdaFunction]) {
    let newest = |key: &str, fs: &[LambdaFunction]| fs.iter().filter_map(|f| f.tags.get(key).cloned()).max_by_key(|v| version_parts(v));
    let library = newest("datadog_lambda", functions);
    let extension = newest("dd_extension_version", functions);
    for f in functions.iter_mut() {
        let mut ups = Vec::new();
        if let Some(rt) = &f.runtime {
            if let Some(latest) = runtime_upgrade(rt) {
                ups.push(LambdaUpgrade { component: LambdaComponent::Runtime, current: rt.clone(), latest: latest.into() });
            }
        }
        for (key, component, latest) in [("datadog_lambda", LambdaComponent::DatadogLambda, &library), ("dd_extension_version", LambdaComponent::Extension, &extension)] {
            if let (Some(cur), Some(latest)) = (f.tags.get(key), latest) {
                if version_parts(cur) < version_parts(latest) {
                    ups.push(LambdaUpgrade { component, current: cur.clone(), latest: latest.clone() });
                }
            }
        }
        f.upgrades = ups;
    }
}

// ---------------------------------------------------------------- functions

#[derive(Serialize, ToSchema, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LambdaFunction {
    /// `<region>/<function name>` (lowercase name).
    pub id: String,
    pub name: String,
    pub region: Option<String>,
    pub account: Option<String>,
    pub runtime: Option<String>,
    pub service: Option<String>,
    pub env: Option<String>,
    pub version: Option<String>,
    pub architecture: Option<String>,
    pub arn: Option<String>,
    /// Allocated memory (MB).
    pub memory_mb: Option<f64>,
    pub invocations: Option<f64>,
    pub errors: Option<f64>,
    pub error_rate_pct: Option<f64>,
    pub cold_starts: Option<f64>,
    pub cold_start_pct: Option<f64>,
    pub timeouts: Option<f64>,
    pub out_of_memory: Option<f64>,
    pub throttles: Option<f64>,
    pub duration_avg_ms: Option<f64>,
    pub duration_p95_ms: Option<f64>,
    /// Peak memory used (MB).
    pub memory_used_mb: Option<f64>,
    pub memory_used_pct: Option<f64>,
    /// USD over the range: the extension's estimate, else computed from list prices.
    pub cost_usd: Option<f64>,
    /// When the current version was first seen, if it replaced another one in the range (epoch ms).
    pub last_deploy: Option<i64>,
    pub insights: Vec<LambdaInsight>,
    pub upgrades: Vec<LambdaUpgrade>,
    pub sources: Vec<LambdaSource>,
    pub has_logs: bool,
    /// Canonical tags (functionname, region, aws_account, runtime, memorysize, env, service, stage, team, version, …).
    pub tags: BTreeMap<String, String>,
}

fn round0(v: Option<f64>) -> Option<f64> {
    v.map(f64::round)
}

fn to_function(id: String, a: &Acc) -> LambdaFunction {
    let tag = |k: &str| a.tags.get(k).cloned();
    let memory_mb = tag("memorysize").and_then(|m| memory_mb(&m));
    let invocations = round0(a.get(Field::Invocations));
    let errors = round0(a.get(Field::Errors));
    let cold_starts = round0(a.get(Field::ColdStarts));
    let duration_avg_ms = a.get(Field::DurationAvg).map(round1);
    let memory_used_mb = a.get(Field::MemoryUsed).map(f64::round);
    let architecture = tag("architecture");
    let cost_usd = a.get(Field::Cost).or_else(|| match (invocations, duration_avg_ms, memory_mb) {
        (Some(i), Some(d), Some(m)) => Some(estimate_cost(i, d, m, architecture.as_deref())),
        _ => None,
    });
    let mut f = LambdaFunction {
        id,
        name: tag("functionname").unwrap_or_default(),
        region: tag("region"),
        account: tag("aws_account"),
        runtime: tag("runtime"),
        service: tag("service"),
        env: tag("env"),
        version: tag("version"),
        architecture,
        arn: tag("function_arn"),
        memory_mb,
        invocations,
        errors,
        error_rate_pct: pct(errors, invocations),
        cold_starts,
        cold_start_pct: pct(cold_starts, invocations),
        timeouts: round0(a.get(Field::Timeouts)),
        out_of_memory: round0(a.get(Field::OutOfMemory)),
        throttles: round0(a.get(Field::Throttles)),
        duration_avg_ms,
        duration_p95_ms: a.get(Field::DurationP95).map(round1),
        memory_used_mb,
        memory_used_pct: pct(memory_used_mb, memory_mb),
        cost_usd: cost_usd.map(|c| (c * 1e6).round() / 1e6),
        last_deploy: None,
        insights: vec![],
        upgrades: vec![],
        sources: a.sources.iter().copied().collect(),
        has_logs: false,
        tags: a.tags.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
    };
    f.insights = detect_insights(&f);
    f
}

/// When each function's latest version first appeared, if another version ran before it in the range.
fn last_deploys(versions: &[VersionSeen]) -> HashMap<String, i64> {
    let mut by_fn: HashMap<&str, Vec<&VersionSeen>> = HashMap::new();
    for v in versions {
        by_fn.entry(v.function.as_str()).or_default().push(v);
    }
    by_fn.into_iter().filter(|(_, vs)| vs.len() > 1).filter_map(|(f, vs)| vs.iter().max_by_key(|v| v.last).map(|v| (f.to_lowercase(), v.first))).collect()
}

/// The functions behind one window's inputs, sorted by name and region.
pub(crate) fn aggregate_functions(series: &[SeriesValue], dists: &[DistValue], spans: &[SpanStats], versions: &[VersionSeen], logged: &HashSet<String>, window_ms: i64) -> Vec<LambdaFunction> {
    let deploys = last_deploys(versions);
    let mut out: Vec<LambdaFunction> = accumulate(series, dists, spans, window_ms).into_iter().map(|(id, a)| to_function(id, &a)).collect();
    for f in &mut out {
        let lower = f.name.to_lowercase();
        f.last_deploy = deploys.get(&lower).copied();
        f.has_logs = logged.contains(&lower);
    }
    detect_upgrades(&mut out);
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.region.cmp(&b.region)));
    out
}

// ---------------------------------------------------------------- search

const FACET_KEYS: &[&str] = &["functionname", "insight", "region", "env", "stage", "service", "runtime", "memorysize", "team", "aws_account", "architecture"];

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// `insight:` terms match any of the function's insights; the rest match its tags and, as free text, its name, service and ARN.
pub(crate) fn function_matches(f: &LambdaFunction, terms: &[Term]) -> bool {
    let insights: HashSet<String> = f.insights.iter().map(|i| enum_str(&i.kind)).collect();
    let (insight_terms, rest): (Vec<&Term>, Vec<&Term>) = terms.iter().partition(|t| matches!(t, Term::Tag { key, .. } if key == "insight"));
    for t in insight_terms {
        if let Term::Tag { values, negate, .. } = t {
            let hit = values.iter().any(|v| insights.contains(v));
            if hit == *negate {
                return false;
            }
        }
    }
    let rest: Vec<Term> = rest.into_iter().cloned().collect();
    let tags: HashMap<String, String> = f.tags.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let empty = HashMap::new();
    let text = format!("{} {} {}", f.name, f.service.as_deref().unwrap_or(""), f.arn.as_deref().unwrap_or(""));
    matches(&rest, &Fields { service: f.service.as_deref().unwrap_or(""), status: "", host: "", source: "", message: &text, tags: &tags, attr: &empty, index: None })
}

#[derive(Serialize, ToSchema, Clone, PartialEq, Debug)]
pub struct LambdaFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema, Clone, PartialEq, Debug)]
pub struct LambdaFacet {
    pub key: String,
    pub values: Vec<LambdaFacetValue>,
}

pub(crate) fn facets(functions: &[LambdaFunction]) -> Vec<LambdaFacet> {
    FACET_KEYS
        .iter()
        .map(|k| {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for f in functions {
                if *k == "insight" {
                    for kind in f.insights.iter().map(|i| enum_str(&i.kind)).collect::<BTreeSet<_>>() {
                        *counts.entry(kind).or_default() += 1;
                    }
                } else if let Some(v) = f.tags.get(*k) {
                    *counts.entry(if *k == "functionname" { f.name.clone() } else { v.clone() }).or_default() += 1;
                }
            }
            let mut values: Vec<LambdaFacetValue> = counts.into_iter().map(|(value, count)| LambdaFacetValue { value, count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            LambdaFacet { key: k.to_string(), values }
        })
        .filter(|f| !f.values.is_empty())
        .collect()
}

#[derive(Serialize, ToSchema, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessSummary {
    pub functions: usize,
    pub invocations: f64,
    pub errors: f64,
    pub cold_starts: f64,
    pub cost_usd: f64,
    /// Functions with at least one insight.
    pub with_insights: usize,
    /// Functions with an upgrade available.
    pub upgradable: usize,
}

pub(crate) fn summarize(functions: &[LambdaFunction]) -> ServerlessSummary {
    let total = |pick: fn(&LambdaFunction) -> Option<f64>| functions.iter().filter_map(pick).fold(0.0, |a, b| a + b);
    ServerlessSummary {
        functions: functions.len(),
        invocations: total(|f| f.invocations),
        errors: total(|f| f.errors),
        cold_starts: total(|f| f.cold_starts),
        cost_usd: (total(|f| f.cost_usd) * 1e6).round() / 1e6,
        with_insights: functions.iter().filter(|f| !f.insights.is_empty()).count(),
        upgradable: functions.iter().filter(|f| !f.upgrades.is_empty()).count(),
    }
}

// ---------------------------------------------------------------- ClickHouse

/// The function name of a Lambda span (Datadog tracer, else OpenTelemetry).
const SPAN_FUNCTION: &str = "lower(if(SpanAttributes['functionname'] != '', SpanAttributes['functionname'], ResourceAttributes['faas.name']))";
/// The span of one invocation: the Datadog tracer's `aws.lambda` span, or OpenTelemetry's FaaS server span.
const LAMBDA_SPAN: &str = "(SpanName = 'aws.lambda' OR (ResourceAttributes['faas.name'] != '' AND (SpanAttributes['faas.invocation_id'] != '' OR SpanAttributes['faas.execution'] != '' \
                           OR SpanAttributes['faas.trigger'] != '' OR SpanAttributes['faas.coldstart'] != '')))";
const SPAN_COLD: &str = "(SpanAttributes['cold_start'] = 'true' OR SpanAttributes['faas.coldstart'] = 'true')";
const SPAN_REGION: &str = "multiIf(SpanAttributes['region'] != '', SpanAttributes['region'], ResourceAttributes['cloud.region'] != '', ResourceAttributes['cloud.region'], \
                           arrayElement(splitByChar(':', SpanAttributes['function_arn']), 4))";
const SPAN_VERSION: &str = "multiIf(SpanAttributes['function_version'] != '', SpanAttributes['function_version'], ResourceAttributes['faas.version'] != '', ResourceAttributes['faas.version'], \
                            ResourceAttributes['service.version'])";

fn key_list() -> String {
    name_list(&tag_keys())
}

fn interval_expr(column: &str, step_ms: Option<i64>) -> String {
    match step_ms {
        Some(step) => format!("toInt64(toUnixTimestamp(toStartOfInterval({column}, INTERVAL {} SECOND))) * 1000", (step / 1000).max(1)),
        None => "toInt64(0)".into(),
    }
}

/// Lambda spans per function (and interval): counts, cold starts, average and p95 duration, and their tags.
/// Parameters: from, to (epoch ms), the row limit.
pub(crate) fn span_stats_sql(step_ms: Option<i64>) -> String {
    format!(
        "SELECT {t} AS t, {SPAN_FUNCTION} AS function, {SPAN_REGION} AS region, \
         toJSONString(anyLast(mapFilter((k, v) -> has([{keys}], k), ResourceAttributes))) AS res, \
         toJSONString(anyLast(mapFilter((k, v) -> has([{keys}], k), SpanAttributes))) AS attrs, \
         count() AS hits, countIf(StatusCode = 'Error') AS errors, countIf({SPAN_COLD}) AS cold, \
         avg(Duration) / 1e6 AS avg_ms, quantile(0.95)(Duration) / 1e6 AS p95_ms \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
         AND {LAMBDA_SPAN} GROUP BY t, function, region HAVING function != '' ORDER BY t LIMIT ?",
        t = interval_expr("Timestamp", step_ms),
        keys = key_list(),
    )
}

/// Versions per function name, first and last seen. Parameters: from, to, the row limit.
pub(crate) fn versions_sql() -> String {
    format!(
        "SELECT {SPAN_FUNCTION} AS function, {SPAN_VERSION} AS version, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
         AND {LAMBDA_SPAN} GROUP BY function, version HAVING function != '' AND version != '' LIMIT ?"
    )
}

/// Delta distributions of the Lambda metrics per series (and interval), with their buckets merged.
/// Parameters: from, to (epoch ms), the row limit.
pub(crate) fn dist_sql(step_ms: Option<i64>) -> String {
    format!(
        "SELECT metric, res, attrs, t, scale, count, sum, max, tupleElement(b, 1) AS idx, tupleElement(b, 2) AS cnt FROM ( \
           SELECT MetricName AS metric, toJSONString(ResourceAttributes) AS res, toJSONString(Attributes) AS attrs, {t} AS t, Scale AS scale, \
                  sum(Count) AS count, sum(Sum) AS sum, max(Max) AS max, \
                  sumMap(arrayMap(i -> toInt32(PositiveOffset + i - 1), arrayEnumerate(PositiveBucketCounts)), PositiveBucketCounts) AS b \
           FROM {src} WHERE OrgId = {{oc_org:String}} AND MetricName IN ({names}) AND AggregationTemporality = 1 \
             AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) \
           GROUP BY metric, res, attrs, t, scale) ORDER BY t LIMIT ?",
        t = interval_expr("TimeUnix", step_ms),
        names = name_list(&dist_names()),
        // Series + distribution samples (the client sets the window: `with_window`).
        src = exp_histogram_subquery(&dist_names()),
    )
}

#[derive(clickhouse::Row, Deserialize)]
struct SpanRow {
    t: i64,
    function: String,
    region: String,
    res: String,
    attrs: String,
    hits: u64,
    errors: u64,
    cold: u64,
    avg_ms: f64,
    p95_ms: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct VersionRow {
    function: String,
    version: String,
    first: i64,
    last: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct DistRow {
    metric: String,
    res: String,
    attrs: String,
    t: i64,
    scale: i32,
    count: u64,
    sum: f64,
    max: f64,
    idx: Vec<i32>,
    cnt: Vec<u64>,
}

fn json_map(s: &str) -> HashMap<String, String> {
    serde_json::from_str(s).unwrap_or_default()
}

async fn span_stats(st: &AppState, org: &str, range: TimeRange, step_ms: Option<i64>) -> Result<Vec<SpanStats>, ApiError> {
    let rows = st.ch(org).query(&span_stats_sql(step_ms)).bind(range.from).bind(range.to).bind(MAX_ROWS).fetch_all::<SpanRow>().await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            // The grouping keys win over the attributes they were read from.
            let mut attrs = json_map(&r.attrs);
            attrs.insert("functionname".into(), r.function);
            if !r.region.is_empty() {
                attrs.insert("region".into(), r.region);
            }
            SpanStats { res: json_map(&r.res), attrs, t: r.t, hits: r.hits, errors: r.errors, cold: r.cold, avg_ms: r.avg_ms, p95_ms: r.p95_ms }
        })
        .collect())
}

async fn versions(st: &AppState, org: &str, range: TimeRange) -> Result<Vec<VersionSeen>, ApiError> {
    let rows = st.ch(org).query(&versions_sql()).bind(range.from).bind(range.to).bind(MAX_ROWS).fetch_all::<VersionRow>().await?;
    Ok(rows.into_iter().map(|r| VersionSeen { function: r.function, version: r.version, first: r.first, last: r.last }).collect())
}

async fn dist_values(st: &AppState, org: &str, range: TimeRange, step_ms: Option<i64>) -> Result<Vec<DistValue>, ApiError> {
    let rows = with_window(st.ch(org), range.from, range.to).query(&dist_sql(step_ms)).bind(range.from).bind(range.to).bind(MAX_ROWS).fetch_all::<DistRow>().await?;
    Ok(rows
        .into_iter()
        .map(|r| DistValue {
            metric: r.metric,
            res: json_map(&r.res),
            attrs: json_map(&r.attrs),
            t: r.t,
            count: r.count,
            sum: r.sum,
            max: r.max,
            scale: r.scale,
            buckets: r.idx.into_iter().zip(r.cnt).collect(),
        })
        .collect())
}

/// Lowercase names of the functions with logs in the range (that the caller may read).
async fn logged_functions(st: &AppState, org: &str, access: &LogAccess, range: TimeRange) -> Result<HashSet<String>, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Name {
        function: String,
    }
    let s = scope(org, access, range, &[], None);
    let mut params = s.params.clone();
    params.push(Param::Int(10_000));
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT lower(if(tags['functionname'] != '', tags['functionname'], tags['faas.name'])) AS function FROM {} WHERE {} \
             AND (tags['functionname'] != '' OR tags['faas.name'] != '') GROUP BY function LIMIT ?",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all::<Name>()
    .await?;
    Ok(rows.into_iter().map(|r| r.function).collect())
}

/// Every function of the window (before the query).
async fn load_functions(st: &AppState, org: &str, access: &LogAccess, range: TimeRange) -> Result<Vec<LambdaFunction>, ApiError> {
    let names = series_names();
    let (series, dists, spans, versions, logged) = tokio::try_join!(
        series_values(st, org, &names, range.from, range.to, None),
        dist_values(st, org, range, None),
        span_stats(st, org, range, None),
        versions(st, org, range),
        logged_functions(st, org, access, range),
    )?;
    Ok(aggregate_functions(&series, &dists, &spans, &versions, &logged, range.to - range.from))
}

// ---------------------------------------------------------------- handlers

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessSearchRequest {
    pub range: TimeRange,
    /// `functionname:x region:y env:z runtime:r insight:cold_starts …` + free text on the name, service and ARN.
    #[serde(default)]
    pub query: String,
    /// Only functions with an upgrade available (runtime, Datadog library or extension).
    #[serde(default)]
    pub upgrade_available: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ServerlessSearchPage {
    /// False when no Lambda telemetry arrived at all in the range (show the setup guide).
    pub enabled: bool,
    pub summary: ServerlessSummary,
    pub functions: Vec<LambdaFunction>,
    /// Over every function of the range, whatever the query.
    pub facets: Vec<LambdaFacet>,
}

/// The AWS Lambda functions of the range, with their invocations, errors, durations, memory, cost, insights and upgrades.
#[utoipa::path(post, path = "/api/v1/serverless/functions", operation_id = "search_serverless_functions", request_body = ServerlessSearchRequest, responses((status = 200, body = ServerlessSearchPage)), tag = "infrastructure")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<ServerlessSearchRequest>) -> ApiResult<ServerlessSearchPage> {
    req.range.validate()?;
    let all = load_functions(&st, &org, &access, req.range).await?;
    let terms = parse_query(&req.query);
    let functions: Vec<LambdaFunction> = all.iter().filter(|f| function_matches(f, &terms) && (!req.upgrade_available || !f.upgrades.is_empty())).cloned().collect();
    Ok(Json(ServerlessSearchPage { enabled: !all.is_empty(), summary: summarize(&functions), facets: facets(&all), functions }))
}

#[derive(Deserialize, ToSchema)]
pub struct ServerlessTimeseriesRequest {
    pub range: TimeRange,
    /// Function ids (`<region>/<name>`) to chart; at most 10 are charted.
    pub functions: Vec<String>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct ServerlessSeries {
    /// invocations, errors, cold_starts (counts per interval) or duration (p95, else average; ms).
    pub metric: String,
    /// The function id.
    pub function: String,
    pub unit: String,
    /// `[timestamp ms, value]` pairs.
    pub points: Vec<(i64, f64)>,
}

const CHART_SERIES: [(&str, &str); 4] = [("invocations", "count"), ("errors", "count"), ("duration", "millisecond"), ("cold_starts", "count")];

/// Per interval, the charted functions' invocations, errors, duration and cold starts.
pub(crate) fn function_series(series: &[SeriesValue], dists: &[DistValue], spans: &[SpanStats], step_ms: i64, ids: &[String]) -> Vec<ServerlessSeries> {
    let wanted: Vec<&String> = ids.iter().take(MAX_CHARTED).collect();
    let mut times: BTreeSet<i64> = BTreeSet::new();
    times.extend(series.iter().map(|v| v.t));
    times.extend(dists.iter().map(|v| v.t));
    times.extend(spans.iter().map(|v| v.t));
    let mut out: BTreeMap<(usize, String), Vec<(i64, f64)>> = BTreeMap::new();
    for t in times {
        let s: Vec<SeriesValue> = series.iter().filter(|v| v.t == t).cloned().collect();
        let d: Vec<DistValue> = dists.iter().filter(|v| v.t == t).cloned().collect();
        let sp: Vec<SpanStats> = spans.iter().filter(|v| v.t == t).cloned().collect();
        for (id, acc) in accumulate(&s, &d, &sp, step_ms) {
            if !wanted.contains(&&id) {
                continue;
            }
            let values = [
                acc.get(Field::Invocations).map(f64::round),
                acc.get(Field::Errors).map(f64::round),
                acc.get(Field::DurationP95).or(acc.get(Field::DurationAvg)).map(round1),
                acc.get(Field::ColdStarts).map(f64::round),
            ];
            for (i, v) in values.into_iter().enumerate() {
                if let Some(v) = v {
                    out.entry((i, id.clone())).or_default().push((t, v));
                }
            }
        }
    }
    out.into_iter().map(|((i, function), points)| ServerlessSeries { metric: CHART_SERIES[i].0.into(), function, unit: CHART_SERIES[i].1.into(), points }).collect()
}

/// Invocations, errors, duration and cold starts of the given functions over the range.
#[utoipa::path(post, path = "/api/v1/serverless/timeseries", operation_id = "serverless_timeseries", request_body = ServerlessTimeseriesRequest, responses((status = 200, body = Vec<ServerlessSeries>)), tag = "infrastructure")]
pub async fn timeseries(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServerlessTimeseriesRequest>) -> ApiResult<Vec<ServerlessSeries>> {
    req.range.validate()?;
    if req.functions.is_empty() {
        return Ok(Json(vec![]));
    }
    let step = chart_step(req.range.to - req.range.from, 60);
    let names = series_names();
    let (series, dists, spans) =
        tokio::try_join!(series_values(&st, &org, &names, req.range.from, req.range.to, Some(step)), dist_values(&st, &org, req.range, Some(step)), span_stats(&st, &org, req.range, Some(step)),)?;
    Ok(Json(function_series(&series, &dists, &spans, step, &req.functions)))
}

#[derive(Deserialize, ToSchema)]
pub struct ServerlessInvocationsRequest {
    pub range: TimeRange,
    /// The function name.
    pub function: String,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LambdaInvocation {
    pub timestamp: i64,
    pub trace_id: String,
    pub span_id: String,
    pub duration_ms: f64,
    pub error: bool,
    pub cold_start: bool,
    pub request_id: String,
    /// What invoked it (`api-gateway`, `sqs`, `http`, …), when the tracer says.
    pub trigger: String,
    pub message: String,
}

#[derive(clickhouse::Row, Deserialize)]
struct InvocationRow {
    ts: i64,
    trace_id: String,
    span_id: String,
    ms: f64,
    error: u8,
    cold: u8,
    request_id: String,
    trigger: String,
    message: String,
}

/// The latest invocations (Lambda spans) of one function.
#[utoipa::path(post, path = "/api/v1/serverless/invocations", operation_id = "serverless_invocations", request_body = ServerlessInvocationsRequest, responses((status = 200, body = Vec<LambdaInvocation>)), tag = "infrastructure")]
pub async fn invocations(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServerlessInvocationsRequest>) -> ApiResult<Vec<LambdaInvocation>> {
    req.range.validate()?;
    let sql = format!(
        "SELECT toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, TraceId AS trace_id, SpanId AS span_id, Duration / 1e6 AS ms, toUInt8(StatusCode = 'Error') AS error, \
         toUInt8({SPAN_COLD}) AS cold, if(SpanAttributes['request_id'] != '', SpanAttributes['request_id'], SpanAttributes['faas.invocation_id']) AS request_id, \
         if(SpanAttributes['function_trigger.event_source'] != '', SpanAttributes['function_trigger.event_source'], SpanAttributes['faas.trigger']) AS trigger, \
         StatusMessage AS message FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
         AND {LAMBDA_SPAN} AND {SPAN_FUNCTION} = ? ORDER BY Timestamp DESC LIMIT 50"
    );
    let rows = st.ch(&org).query(&sql).bind(req.range.from).bind(req.range.to).bind(req.function.to_lowercase()).fetch_all::<InvocationRow>().await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| LambdaInvocation {
                timestamp: r.ts,
                trace_id: r.trace_id,
                span_id: r.span_id,
                duration_ms: round1(r.ms),
                error: r.error == 1,
                cold_start: r.cold == 1,
                request_id: r.request_id,
                trigger: r.trigger,
                message: r.message,
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(kv: &[(&str, &str)]) -> HashMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn sv(metric: &str, value: f64, attrs: &[(&str, &str)]) -> SeriesValue {
        SeriesValue { metric: metric.into(), res: HashMap::new(), attrs: map(attrs), t: 0, value, max: value, count: 1, last: 0 }
    }

    fn dv(metric: &str, count: u64, sum: f64, max: f64, attrs: &[(&str, &str)]) -> DistValue {
        DistValue { metric: metric.into(), res: HashMap::new(), attrs: map(attrs), t: 0, count, sum, max, scale: 6, buckets: vec![] }
    }

    const CHECKOUT: &[(&str, &str)] =
        &[("functionname", "checkout-webhook"), ("region", "us-east-1"), ("account_id", "123456789012"), ("memorysize", "1024"), ("runtime", "nodejs18.x"), ("service", "checkout")];
    const HOUR: i64 = 3_600_000;

    #[test]
    fn arns_and_memory_sizes() {
        assert_eq!(parse_arn("arn:aws:lambda:eu-west-1:123456789012:function:image-resizer:7"), Some(("eu-west-1".into(), "123456789012".into(), "image-resizer".into())));
        assert_eq!(parse_arn("arn:aws:s3:::bucket"), None);
        assert_eq!(memory_mb("512"), Some(512.0));
        assert_eq!(memory_mb("536870912"), Some(512.0), "OpenTelemetry faas.max_memory in bytes");
        assert_eq!(memory_mb("x"), None);
    }

    #[test]
    fn exponential_histogram_quantiles() {
        // Scale 0: bucket i holds (2^i, 2^(i+1)].
        let buckets = [(0, 50), (3, 45), (6, 5)];
        let p50 = exp_quantile(0, &buckets, 0.5).unwrap();
        assert!((1.0..=2.0).contains(&p50), "p50 {p50} in (1, 2]");
        let p95 = exp_quantile(0, &buckets, 0.95).unwrap();
        assert!((8.0..=16.0).contains(&p95), "p95 {p95} in (8, 16]");
        assert_eq!(exp_quantile(0, &[], 0.95), None);
    }

    #[test]
    fn enhanced_metrics_make_a_function() {
        let mut cold = CHECKOUT.to_vec();
        cold.push(("cold_start", "true"));
        let dists = [
            dv("aws.lambda.enhanced.invocations", 90, 90.0, 1.0, CHECKOUT),
            dv("aws.lambda.enhanced.invocations", 10, 10.0, 1.0, &cold),
            dv("aws.lambda.enhanced.errors", 8, 8.0, 1.0, CHECKOUT),
            DistValue { buckets: vec![(-200, 100)], ..dv("aws.lambda.enhanced.duration", 100, 25.0, 0.4, CHECKOUT) },
            dv("aws.lambda.enhanced.max_memory_used", 100, 9000.0, 960.0, CHECKOUT),
            dv("aws.lambda.enhanced.estimated_cost", 100, 0.0042, 0.0001, CHECKOUT),
        ];
        let fs = aggregate_functions(&[], &dists, &[], &[], &HashSet::new(), HOUR);
        assert_eq!(fs.len(), 1);
        let f = &fs[0];
        assert_eq!(f.id, "us-east-1/checkout-webhook");
        assert_eq!(f.account.as_deref(), Some("123456789012"));
        assert_eq!(f.invocations, Some(100.0));
        assert_eq!(f.cold_starts, Some(10.0), "invocations tagged cold_start:true");
        assert_eq!(f.error_rate_pct, Some(8.0));
        assert_eq!(f.duration_avg_ms, Some(250.0), "seconds to ms");
        assert!(f.duration_p95_ms.is_some());
        assert_eq!(f.memory_used_mb, Some(960.0));
        assert_eq!(f.memory_used_pct, Some(93.8));
        assert_eq!(f.cost_usd, Some(0.0042), "the extension's estimate wins");
        assert_eq!(f.sources, vec![LambdaSource::EnhancedMetrics]);
        let kinds: Vec<_> = f.insights.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, vec![LambdaInsightKind::HighErrors, LambdaInsightKind::ColdStarts, LambdaInsightKind::HighMemory]);
        assert_eq!(f.upgrades, vec![LambdaUpgrade { component: LambdaComponent::Runtime, current: "nodejs18.x".into(), latest: "nodejs24.x".into() }]);
    }

    #[test]
    fn integration_metrics_and_spans_fill_in_and_metrics_win() {
        // aws.lambda.invocations as a Datadog count: a per-second rate over the hour.
        let series = [sv("aws.lambda.invocations", 1.0, CHECKOUT), sv("aws.lambda.duration", 120.0, CHECKOUT), sv("aws.lambda.throttles", 2.0 / 3600.0, CHECKOUT)];
        let spans = [SpanStats {
            attrs: map(&[("functionname", "checkout-webhook"), ("region", "us-east-1"), ("cold_start", "true")]),
            hits: 600,
            errors: 6,
            cold: 30,
            avg_ms: 100.0,
            p95_ms: 310.0,
            ..Default::default()
        }];
        let fs = aggregate_functions(&series, &[], &spans, &[], &HashSet::from(["checkout-webhook".to_string()]), HOUR);
        let f = &fs[0];
        assert_eq!(f.invocations, Some(3600.0), "metrics beat sampled spans");
        assert_eq!(f.errors, Some(6.0), "spans fill what metrics lack");
        assert_eq!(f.duration_avg_ms, Some(120.0));
        assert_eq!(f.duration_p95_ms, Some(310.0));
        assert_eq!(f.throttles, Some(2.0));
        assert!(f.has_logs);
        assert_eq!(f.sources, vec![LambdaSource::IntegrationMetrics, LambdaSource::Traces]);
        // No cost metric: estimated from invocations × duration × memory.
        assert_eq!(f.cost_usd, Some(((3600.0 * 0.2 / 1e6 + 3600.0 * 0.12 * 1.0 * PRICE_GB_S_X86) * 1e6_f64).round() / 1e6));
        assert!(f.insights.iter().any(|i| i.kind == LambdaInsightKind::Throttles));
    }

    #[test]
    fn otel_spans_and_metrics_use_faas_attributes() {
        let res = map(&[("faas.name", "inventory-sync"), ("cloud.region", "eu-west-1"), ("faas.max_memory", "268435456"), ("cloud.account.id", "123456789012")]);
        let spans = [SpanStats {
            res: res.clone(),
            attrs: map(&[("functionname", "inventory-sync"), ("region", "eu-west-1")]),
            hits: 40,
            errors: 0,
            cold: 2,
            avg_ms: 80.0,
            p95_ms: 150.0,
            ..Default::default()
        }];
        // OpenTelemetry metrics without a region join the function seen in its region.
        let series = [SeriesValue { res: map(&[("faas.name", "inventory-sync")]), ..sv("faas.timeouts", 3.0 / 3600.0, &[]) }];
        let fs = aggregate_functions(&series, &[], &spans, &[], &HashSet::new(), HOUR);
        assert_eq!(fs.len(), 1);
        let f = &fs[0];
        assert_eq!(f.id, "eu-west-1/inventory-sync");
        assert_eq!(f.memory_mb, Some(256.0));
        assert_eq!(f.timeouts, Some(3.0));
        assert_eq!(f.cold_start_pct, Some(5.0));
        assert_eq!(f.sources, vec![LambdaSource::OtelMetrics, LambdaSource::Traces]);
    }

    #[test]
    fn cost_estimates() {
        // 1M invocations of 100 ms at 1 GB: $0.20 + 100,000 GB-s.
        let x86 = estimate_cost(1e6, 100.0, 1024.0, None);
        assert!((x86 - (0.2 + 100_000.0 * PRICE_GB_S_X86)).abs() < 1e-9);
        let arm = estimate_cost(1e6, 100.0, 1024.0, Some("arm64"));
        assert!(arm < x86);
        // Billed per started millisecond.
        assert_eq!(estimate_cost(1.0, 0.2, 1024.0, None), estimate_cost(1.0, 1.0, 1024.0, None));
    }

    #[test]
    fn runtime_upgrades() {
        assert_eq!(runtime_upgrade("nodejs18.x"), Some("nodejs24.x"));
        assert_eq!(runtime_upgrade("nodejs24.x"), None);
        assert_eq!(runtime_upgrade("python3.9"), Some("python3.14"));
        assert_eq!(runtime_upgrade("python3.14"), None);
        assert_eq!(runtime_upgrade("java8.al2"), Some("java25"));
        assert_eq!(runtime_upgrade("go1.x"), Some("provided.al2023"));
        assert_eq!(runtime_upgrade("provided.al2"), Some("provided.al2023"));
        assert_eq!(runtime_upgrade("provided.al2023"), None);
        assert_eq!(runtime_upgrade("nodejs26.x"), None, "newer than the table");
        assert_eq!(runtime_upgrade("cobol"), None);
    }

    #[test]
    fn library_and_extension_upgrades_follow_the_fleet() {
        let a: Vec<(&str, &str)> = [CHECKOUT, &[("datadog_lambda", "v10.2.0"), ("dd_extension_version", "80")]].concat();
        let b = [("functionname", "image-resizer"), ("region", "us-east-1"), ("runtime", "python3.14"), ("datadog_lambda", "v10.12.1"), ("dd_extension_version", "88")];
        let fs = aggregate_functions(&[sv("aws.lambda.invocations", 1.0, &a), sv("aws.lambda.invocations", 1.0, &b)], &[], &[], &[], &HashSet::new(), HOUR);
        let checkout = fs.iter().find(|f| f.name == "checkout-webhook").unwrap();
        let comps: Vec<_> = checkout.upgrades.iter().map(|u| (u.component, u.latest.as_str())).collect();
        assert_eq!(comps, vec![(LambdaComponent::Runtime, "nodejs24.x"), (LambdaComponent::DatadogLambda, "v10.12.1"), (LambdaComponent::Extension, "88")]);
        assert!(fs.iter().find(|f| f.name == "image-resizer").unwrap().upgrades.is_empty());
    }

    #[test]
    fn insights_thresholds() {
        let base = aggregate_functions(&[sv("aws.lambda.invocations", 1.0, CHECKOUT)], &[], &[], &[], &HashSet::new(), HOUR).remove(0);
        let with = |patch: &dyn Fn(&mut LambdaFunction)| {
            let mut f = base.clone();
            patch(&mut f);
            detect_insights(&f).into_iter().map(|i| i.kind).collect::<Vec<_>>()
        };
        assert!(with(&|_| {}).is_empty());
        assert_eq!(with(&|f| f.out_of_memory = Some(1.0)), vec![LambdaInsightKind::OutOfMemory]);
        assert_eq!(with(&|f| f.memory_used_pct = Some(12.0)), vec![LambdaInsightKind::OverProvisioned]);
        assert_eq!(
            with(&|f| {
                f.memory_used_pct = Some(12.0);
                f.memory_mb = Some(128.0);
            }),
            vec![],
            "small allocations are not over-provisioned"
        );
        assert_eq!(
            with(&|f| {
                f.invocations = Some(5.0);
                f.cold_start_pct = Some(60.0);
            }),
            vec![],
            "too few invocations for a cold start rate"
        );
    }

    #[test]
    fn deploys_from_versions_seen() {
        let v = |version: &str, first: i64, last: i64| VersionSeen { function: "checkout-webhook".into(), version: version.into(), first, last };
        let fs = aggregate_functions(&[sv("aws.lambda.invocations", 1.0, CHECKOUT)], &[], &[], &[v("41", 0, 500), v("42", 400, 900)], &HashSet::new(), HOUR);
        assert_eq!(fs[0].last_deploy, Some(400));
        let one = aggregate_functions(&[sv("aws.lambda.invocations", 1.0, CHECKOUT)], &[], &[], &[v("42", 0, 900)], &HashSet::new(), HOUR);
        assert_eq!(one[0].last_deploy, None, "no deploy within the range");
    }

    #[test]
    fn query_facets_and_summary() {
        let b = [("functionname", "image-resizer"), ("region", "eu-west-1"), ("env", "prod"), ("runtime", "python3.14")];
        let fs = aggregate_functions(
            &[sv("aws.lambda.invocations", 1.0, CHECKOUT), sv("aws.lambda.timeouts_unused", 1.0, CHECKOUT), sv("aws.lambda.invocations", 0.5, &b), sv("aws.lambda.throttles", 1.0 / 3600.0, &b)],
            &[],
            &[],
            &[],
            &HashSet::new(),
            HOUR,
        );
        assert_eq!(fs.len(), 2);
        let pick = |q: &str| fs.iter().filter(|f| function_matches(f, &parse_query(q))).map(|f| f.name.clone()).collect::<Vec<_>>();
        assert_eq!(pick("region:eu-west-1"), vec!["image-resizer"]);
        assert_eq!(pick("insight:throttles"), vec!["image-resizer"]);
        assert_eq!(pick("-insight:throttles"), vec!["checkout-webhook"]);
        assert_eq!(pick("checkout"), vec!["checkout-webhook"], "free text on the name and service");
        assert_eq!(pick("runtime:nodejs*"), vec!["checkout-webhook"]);
        let f = facets(&fs);
        let insight = f.iter().find(|f| f.key == "insight").unwrap();
        assert_eq!(insight.values, vec![LambdaFacetValue { value: "throttles".into(), count: 1 }]);
        assert!(f.iter().any(|f| f.key == "memorysize"));
        let s = summarize(&fs);
        assert_eq!((s.functions, s.invocations, s.with_insights, s.upgradable), (2, 5400.0, 1, 1));
    }

    #[test]
    fn series_per_function_and_interval() {
        let at = |t: i64, v: SeriesValue| SeriesValue { t, ..v };
        let series = [at(0, sv("aws.lambda.invocations", 1.0, CHECKOUT)), at(60_000, sv("aws.lambda.invocations", 2.0, CHECKOUT)), at(60_000, sv("aws.lambda.duration", 90.0, CHECKOUT))];
        let spans = [SpanStats { t: 0, attrs: map(&[("functionname", "other"), ("region", "us-east-1")]), hits: 3, ..Default::default() }];
        let out = function_series(&series, &[], &spans, 60_000, &["us-east-1/checkout-webhook".into()]);
        let inv = out.iter().find(|s| s.metric == "invocations").unwrap();
        assert_eq!(inv.function, "us-east-1/checkout-webhook");
        assert_eq!(inv.points, vec![(0, 60.0), (60_000, 120.0)]);
        assert_eq!(out.iter().find(|s| s.metric == "duration").unwrap().points, vec![(60_000, 90.0)]);
        assert!(out.iter().all(|s| s.function != "us-east-1/other"), "only the charted functions");
    }

    #[test]
    fn queries_filter_by_org_and_bind_input() {
        for sql in [span_stats_sql(None), span_stats_sql(Some(60_000)), versions_sql(), dist_sql(None)] {
            assert!(sql.contains("OrgId = {oc_org:String}"), "{sql}");
            assert_eq!(sql.matches('?').count(), 3, "from, to and the row limit are bound: {sql}");
        }
        assert!(dist_sql(None).contains("'aws.lambda.enhanced.duration'"));
        // The metrics storage, not the exporter's tables (dropped).
        assert!(dist_sql(None).contains("otel.metrics_dist_samples_read AS x") && !dist_sql(None).contains("otel_metrics_"));
    }
}
