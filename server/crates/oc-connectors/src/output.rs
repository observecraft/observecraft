//! A stream's records as logs, metric points, events and costs ([`Mapper`]), and the Kafka bodies the server
//! publishes them as: JSON logs (`oc.logs.json`), OTLP JSON metrics (`oc.otlp-json.metrics`) and OTLP JSON logs
//! carrying events (`oc.otlp-json.logs`, `oc.signal = dd.events`).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::expr::{self, text, to_ms};
use crate::manifest::{Manifest, MetricAggregate, MetricKind, Output, Stream};
use crate::template::{Template, Tree};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Point {
    pub name: String,
    pub kind: MetricKind,
    pub host: Option<String>,
    /// Sorted.
    pub tags: Vec<(String, String)>,
    pub time_ms: i64,
    pub value: f64,
    /// The metric's unit (Datadog's names), sent as OTLP's `unit`; not in golden files.
    #[serde(skip)]
    pub unit: Option<String>,
    /// Combined with the run's other points of the same series and time ([`aggregate`]).
    #[serde(skip)]
    pub aggregate: Option<MetricAggregate>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Event {
    pub title: String,
    pub text: String,
    pub time_ms: i64,
    pub host: String,
    pub tags: Vec<String>,
    pub alert_type: String,
    pub priority: String,
    pub source_type_name: String,
    pub aggregation_key: String,
}

/// A FOCUS row of `otel.cloud_costs`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cost {
    pub provider: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub cost: f64,
    pub currency: String,
    pub service: String,
    pub account: String,
    pub usage_type: String,
    pub usage_amount: f64,
    pub tags: BTreeMap<String, String>,
    /// Stable per record: rerunning a day replaces its rows.
    pub line: u32,
}

/// What a run's records became.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Mapped {
    pub logs: Vec<Value>,
    pub points: Vec<Point>,
    pub events: Vec<Event>,
    pub costs: Vec<Cost>,
}

fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

const BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// A record id's short hash (dedupe state).
pub fn id_hash(id: &str) -> String {
    format!("{:016x}", fnv(BASIS, id.as_bytes()))
}

/// The random bits of a log's identity: the instance, the record's id and the connector (the same record read again
/// gets the same id; ids are per instance).
pub fn identity_seed(instance: &str, id: &str, connector: &str) -> u128 {
    let high = fnv(fnv(BASIS, instance.as_bytes()), id.as_bytes());
    ((high as u128) << 64) | fnv(high, connector.as_bytes()) as u128
}

/// UUIDv7 of `ms` whose other bits come from `seed`, as the log ingester takes `_oc_ingest.id`.
pub fn uuid_v7_seeded(ms: i64, seed: u128) -> String {
    let mut bits = ((ms as u128 & 0xffff_ffff_ffff) << 80) | (seed & 0xffff_ffff_ffff_ffff_ffff);
    bits = (bits & !(0xf << 76)) | (7 << 76);
    bits = (bits & !(3 << 62)) | (2 << 62);
    let hex = format!("{bits:032x}");
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default()
}

fn opt(t: &Option<String>) -> Result<Option<Template>, String> {
    t.as_deref().map(Template::parse).transpose()
}

fn tags(map: &BTreeMap<String, String>) -> Result<Vec<(String, Template)>, String> {
    map.iter().map(|(k, v)| Ok((k.clone(), Template::parse(v)?))).collect()
}

struct LogsMap {
    message: Template,
    status: Option<Template>,
    service: Option<Template>,
    source: Option<Template>,
    host: Option<Template>,
    date: Option<Template>,
    tags: Vec<(String, Template)>,
    attributes: Option<Tree>,
    keep_record_as: Option<String>,
}

struct MetricMap {
    name: Template,
    kind: MetricKind,
    value: Template,
    each: Option<Template>,
    tags: Vec<(String, Template)>,
    tags_from: Option<Template>,
    host: Option<Template>,
    time: Option<Template>,
    filter: Option<Template>,
    unit: Option<Template>,
    aggregate: Option<MetricAggregate>,
}

struct EventsMap {
    title: Template,
    text: Template,
    alert_type: Option<Template>,
    priority: Option<Template>,
    host: Option<Template>,
    tags: Vec<(String, Template)>,
    aggregation_key: Option<Template>,
    date: Option<Template>,
}

struct CostsMap {
    provider: String,
    date: Template,
    cost: Template,
    currency: Option<Template>,
    service: Template,
    account: Option<Template>,
    usage_type: Option<Template>,
    usage_amount: Option<Template>,
    tags: Vec<(String, Template)>,
}

/// A stream's output, compiled.
pub struct Mapper {
    connector: String,
    source: String,
    logs: Option<LogsMap>,
    metrics: Vec<MetricMap>,
    events: Option<EventsMap>,
    costs: Option<CostsMap>,
}

/// One record to map: its scope (`record`, `vars`, …), time and id.
pub struct Item<'a> {
    pub scope: &'a Map<String, Value>,
    pub time_ms: Option<i64>,
    pub id: &'a str,
    pub instance: &'a str,
    pub now_ms: i64,
}

fn rendered_tags(list: &[(String, Template)], scope: &Map<String, Value>) -> Vec<(String, String)> {
    list.iter().map(|(k, t)| (k.clone(), t.text(scope))).filter(|(_, v)| !v.is_empty()).collect()
}

impl Mapper {
    pub fn compile(m: &Manifest, stream: &Stream) -> Result<Mapper, String> {
        Mapper::compile_output(m, &stream.output)
    }

    /// The mapping of an output (a stream's, or a webhook's logs and events).
    pub fn compile_output(m: &Manifest, o: &Output) -> Result<Mapper, String> {
        Mapper::compile_for(&m.id, m.source.as_deref().unwrap_or(&m.id), o)
    }

    /// The mapping of an output for a connector and its logs' source.
    pub fn compile_for(connector: &str, source: &str, o: &Output) -> Result<Mapper, String> {
        Ok(Mapper {
            connector: connector.to_string(),
            source: source.to_string(),
            logs: match &o.logs {
                None => None,
                Some(l) => Some(LogsMap {
                    message: Template::parse(&l.message)?,
                    status: opt(&l.status)?,
                    service: opt(&l.service)?,
                    source: opt(&l.source)?,
                    host: opt(&l.host)?,
                    date: opt(&l.date)?,
                    tags: tags(&l.tags)?,
                    attributes: l.attributes.as_ref().map(Tree::parse).transpose()?,
                    keep_record_as: l.keep_record_as.clone(),
                }),
            },
            metrics: o
                .metrics
                .iter()
                .map(|mo| {
                    Ok(MetricMap {
                        name: Template::parse(&mo.name)?,
                        kind: mo.kind,
                        value: Template::parse(&mo.value)?,
                        each: opt(&mo.each)?,
                        tags: tags(&mo.tags)?,
                        tags_from: opt(&mo.tags_from)?,
                        host: opt(&mo.host)?,
                        time: opt(&mo.time)?,
                        filter: opt(&mo.filter)?,
                        unit: opt(&mo.unit)?,
                        aggregate: mo.aggregate,
                    })
                })
                .collect::<Result<_, String>>()?,
            events: match &o.events {
                None => None,
                Some(e) => Some(EventsMap {
                    title: Template::parse(&e.title)?,
                    text: Template::parse(&e.text)?,
                    alert_type: opt(&e.alert_type)?,
                    priority: opt(&e.priority)?,
                    host: opt(&e.host)?,
                    tags: tags(&e.tags)?,
                    aggregation_key: opt(&e.aggregation_key)?,
                    date: opt(&e.date)?,
                }),
            },
            costs: match &o.costs {
                None => None,
                Some(c) => Some(CostsMap {
                    provider: c.provider.clone(),
                    date: Template::parse(&c.date)?,
                    cost: Template::parse(&c.cost)?,
                    currency: opt(&c.currency)?,
                    service: Template::parse(&c.service)?,
                    account: opt(&c.account)?,
                    usage_type: opt(&c.usage_type)?,
                    usage_amount: opt(&c.usage_amount)?,
                    tags: tags(&c.tags)?,
                }),
            },
        })
    }

    /// Maps one record into `out`.
    pub fn map(&self, item: &Item, out: &mut Mapped) {
        let scope = item.scope;
        if let Some(l) = &self.logs {
            let mut log = match l.attributes.as_ref().map(|a| a.render(scope, true)) {
                Some(Value::Object(o)) => o,
                _ => Map::new(),
            };
            if let Some(key) = &l.keep_record_as {
                log.insert(key.clone(), scope.get("record").cloned().unwrap_or(Value::Null));
            }
            let pick = |t: &Option<Template>, default: &str| t.as_ref().map(|t| t.text(scope)).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string());
            let date = l.date.as_ref().map(|t| t.render(scope)).filter(|v| !crate::template::empty(v));
            let time_ms = date.as_ref().and_then(to_ms).or(item.time_ms).unwrap_or(item.now_ms);
            // A record's own `status` stays an attribute (`@status`), apart from the log's: the log carries its status
            // as `level` and the record's in a JSON message, whose own fields the log ingester keeps as attributes.
            let message = l.message.text(scope);
            match log.remove("status").filter(|v| !v.is_null()) {
                None => {
                    log.insert("message".into(), Value::String(message));
                    log.insert("status".into(), Value::String(pick(&l.status, "info")));
                }
                Some(own) => {
                    log.insert("message".into(), Value::String(json!({ "message": message, "status": own }).to_string()));
                    log.insert("level".into(), Value::String(pick(&l.status, "info")));
                }
            }
            log.insert("service".into(), Value::String(pick(&l.service, &self.source)));
            log.insert("ddsource".into(), Value::String(pick(&l.source, &self.source)));
            log.insert("host".into(), Value::String(pick(&l.host, item.instance)));
            log.insert("date".into(), date.unwrap_or_else(|| Value::String(rfc3339(time_ms))));
            let tags = rendered_tags(&l.tags, scope);
            if !tags.is_empty() {
                log.insert("ddtags".into(), Value::String(tags.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(",")));
            }
            log.insert("_oc_ingest".into(), json!({ "id": uuid_v7_seeded(time_ms, identity_seed(item.instance, item.id, &self.connector)) }));
            out.logs.push(Value::Object(log));
        }
        for mm in &self.metrics {
            if mm.filter.as_ref().is_some_and(|f| !expr::truthy(&f.render(scope))) {
                continue;
            }
            let entries: Vec<Value> = match mm.each.as_ref().map(|e| e.render(scope)) {
                None => vec![Value::Null],
                Some(Value::Object(o)) => o.into_iter().map(|(k, v)| json!({ "key": k, "value": v })).collect(),
                Some(Value::Array(a)) => a,
                Some(Value::Null) => vec![],
                Some(other) => vec![other],
            };
            for entry in entries {
                let mut s;
                let scope = if entry.is_null() {
                    scope
                } else {
                    s = scope.clone();
                    s.insert("entry".into(), entry);
                    &s
                };
                let v = mm.value.render(scope);
                let Some(value) = v.as_f64().or_else(|| v.as_str().and_then(|t| t.trim().parse().ok())).or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 })) else { continue };
                let name = mm.name.text(scope);
                if name.is_empty() {
                    continue;
                }
                let mut tags = rendered_tags(&mm.tags, scope);
                if let Some(Value::Object(more)) = mm.tags_from.as_ref().map(|t| t.render(scope)) {
                    tags.extend(more.into_iter().map(|(k, v)| (k, text(&v))).filter(|(_, v)| !v.is_empty()));
                }
                tags.sort();
                tags.dedup_by(|a, b| a.0 == b.0);
                let host = mm.host.as_ref().map(|h| h.text(scope)).filter(|h| !h.is_empty());
                let minute = item.now_ms - item.now_ms.rem_euclid(60_000);
                let time_ms = mm.time.as_ref().and_then(|t| to_ms(&t.render(scope))).unwrap_or(minute);
                let unit = mm.unit.as_ref().map(|u| u.text(scope)).filter(|u| !u.is_empty());
                out.points.push(Point { name, kind: mm.kind, host, tags, time_ms, value, unit, aggregate: mm.aggregate });
            }
        }
        if let Some(e) = &self.events {
            let pick = |t: &Option<Template>, default: &str| t.as_ref().map(|t| t.text(scope)).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string());
            let time_ms = e.date.as_ref().and_then(|t| to_ms(&t.render(scope))).or(item.time_ms).unwrap_or(item.now_ms);
            let mut tags: Vec<String> = rendered_tags(&e.tags, scope).into_iter().map(|(k, v)| format!("{k}:{v}")).collect();
            tags.push(format!("source:{}", self.source));
            out.events.push(Event {
                title: e.title.text(scope),
                text: e.text.text(scope),
                time_ms,
                host: pick(&e.host, ""),
                tags,
                alert_type: pick(&e.alert_type, "info"),
                priority: pick(&e.priority, "normal"),
                source_type_name: self.source.clone(),
                aggregation_key: pick(&e.aggregation_key, ""),
            });
        }
        if let Some(c) = &self.costs {
            let Some(day_ms) = to_ms(&c.date.render(scope)) else { return };
            let cost = c.cost.render(scope);
            let Some(cost) = cost.as_f64().or_else(|| cost.as_str().and_then(|t| t.trim().parse().ok())) else { return };
            let date = chrono::DateTime::from_timestamp_millis(day_ms).map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default();
            let service = c.service.text(scope);
            let pick = |t: &Option<Template>| t.as_ref().map(|t| t.text(scope)).unwrap_or_default();
            let amount = c.usage_amount.as_ref().map(|t| t.render(scope)).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|t| t.trim().parse().ok()))).unwrap_or(0.0);
            out.costs.push(Cost {
                provider: c.provider.clone(),
                line: (fnv(fnv(BASIS, item.id.as_bytes()), format!("{service}\0{date}").as_bytes()) & 0xffff_ffff) as u32,
                date,
                cost,
                currency: c.currency.as_ref().map(|t| t.text(scope)).filter(|s| !s.is_empty()).unwrap_or_else(|| "USD".into()),
                service,
                account: pick(&c.account),
                usage_type: pick(&c.usage_type),
                usage_amount: amount,
                tags: rendered_tags(&c.tags, scope).into_iter().collect(),
            });
        }
    }
}

/// Combines the points to aggregate (`aggregate` set) of the same name, kind, host, tags and time into one, in the
/// place of the first; other points stay as they are.
pub fn aggregate(points: Vec<Point>) -> Vec<Point> {
    if points.iter().all(|p| p.aggregate.is_none()) {
        return points;
    }
    type Key = (String, bool, Option<String>, Vec<(String, String)>, i64);
    let key = |p: &Point| -> Key { (p.name.clone(), p.kind == MetricKind::Count, p.host.clone(), p.tags.clone(), p.time_ms) };
    let mut groups: BTreeMap<Key, Vec<f64>> = BTreeMap::new();
    for p in points.iter().filter(|p| p.aggregate.is_some()) {
        groups.entry(key(p)).or_default().push(p.value);
    }
    let mut out = Vec::new();
    for p in points {
        let Some(how) = p.aggregate else {
            out.push(p);
            continue;
        };
        let Some(mut values) = groups.remove(&key(&p)) else { continue };
        let n = values.len() as f64;
        let pct = |values: &mut Vec<f64>, q: f64| {
            values.sort_by(|a, b| a.total_cmp(b));
            values[((q * n).ceil() as usize).clamp(1, values.len()) - 1]
        };
        let value = match how {
            MetricAggregate::Sum => values.iter().sum(),
            MetricAggregate::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
            MetricAggregate::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            MetricAggregate::Avg => values.iter().sum::<f64>() / n,
            MetricAggregate::Count => n,
            MetricAggregate::P50 => pct(&mut values, 0.50),
            MetricAggregate::P75 => pct(&mut values, 0.75),
            MetricAggregate::P90 => pct(&mut values, 0.90),
            MetricAggregate::P95 => pct(&mut values, 0.95),
            MetricAggregate::P99 => pct(&mut values, 0.99),
        };
        out.push(Point { value, ..p });
    }
    out
}

fn kv(k: &str, v: &str) -> Value {
    json!({ "key": k, "value": { "stringValue": v } })
}

/// OTLP JSON metrics for `points` (gauges, and counts as delta sums), as the cloud crawlers publish them: resource
/// `cloud.provider = <connector>` (and `host.name`), scope `observecraft.<connector>`.
pub fn metrics_otlp(connector: &str, points: &[Point]) -> Vec<u8> {
    type ByName<'a> = BTreeMap<(&'a str, bool), Vec<&'a Point>>;
    let mut by_host: BTreeMap<Option<&str>, ByName> = BTreeMap::new();
    for p in points {
        by_host.entry(p.host.as_deref()).or_default().entry((&p.name, p.kind == MetricKind::Count)).or_default().push(p);
    }
    let resource_metrics: Vec<Value> = by_host
        .into_iter()
        .map(|(host, metrics)| {
            let attributes: Vec<Value> = host.map(|h| kv("host.name", h)).into_iter().chain([kv("cloud.provider", connector)]).collect();
            let metrics: Vec<Value> = metrics
                .into_iter()
                .map(|((name, count), points)| {
                    let unit = points.iter().find_map(|p| p.unit.as_deref());
                    let data_points: Vec<Value> = points
                        .iter()
                        .map(|p| json!({ "timeUnixNano": (p.time_ms as i128 * 1_000_000).to_string(), "asDouble": p.value, "attributes": p.tags.iter().map(|(k, v)| kv(k, v)).collect::<Vec<_>>() }))
                        .collect();
                    let mut metric = if count {
                        json!({ "name": name, "sum": { "aggregationTemporality": 1, "isMonotonic": true, "dataPoints": data_points } })
                    } else {
                        json!({ "name": name, "gauge": { "dataPoints": data_points } })
                    };
                    if let Some(u) = unit {
                        metric["unit"] = json!(u);
                    }
                    metric
                })
                .collect();
            json!({ "resource": { "attributes": attributes }, "scopeMetrics": [{ "scope": { "name": format!("observecraft.{connector}") }, "metrics": metrics }] })
        })
        .collect();
    serde_json::to_vec(&json!({ "resourceMetrics": resource_metrics })).expect("JSON")
}

/// OTLP JSON logs carrying events (`oc.signal = dd.events`), as the intake makes Datadog events.
pub fn events_otlp(events: &[Event]) -> Vec<u8> {
    let mut by_host: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for e in events {
        let t = (e.time_ms.max(0) as i128 * 1_000_000).to_string();
        let severity = match e.alert_type.as_str() {
            "error" => 17,
            "warning" => 13,
            _ => 9,
        };
        let mut attributes = vec![kv("title", &e.title), kv("alert_type", &e.alert_type), kv("priority", &e.priority), kv("source_type_name", &e.source_type_name)];
        if !e.aggregation_key.is_empty() {
            attributes.push(kv("aggregation_key", &e.aggregation_key));
        }
        if !e.tags.is_empty() {
            attributes.push(kv("tags", &e.tags.join(",")));
        }
        by_host.entry(&e.host).or_default().push(json!({
            "timeUnixNano": t, "observedTimeUnixNano": t, "severityNumber": severity, "severityText": e.alert_type, "body": { "stringValue": e.text }, "attributes": attributes,
        }));
    }
    let resource_logs: Vec<Value> = by_host
        .into_iter()
        .map(|(host, records)| {
            let mut attributes = vec![kv("oc.signal", "dd.events")];
            if !host.is_empty() {
                attributes.insert(0, kv("host.name", host));
            }
            json!({ "resource": { "attributes": attributes }, "scopeLogs": [{ "scope": { "name": "observecraft.connectors" }, "logRecords": records }] })
        })
        .collect();
    serde_json::to_vec(&json!({ "resourceLogs": resource_logs })).expect("JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_match_the_native_netbird_logs() {
        // crate::netbird::logs::seed in oc-api before NetBird became a package: the same event, the same row.
        let id = uuid_v7_seeded(1_793_964_000_123, identity_seed("netbird.example.com", "1042", "netbird"));
        assert_eq!(id.replace('-', "")[..12], format!("{:012x}", 1_793_964_000_123i64), "timed at the event");
        // The native seed: FNV-1a of the host then the id (high bits), then of "netbird" (low bits).
        let fnv = |h: u64, b: &[u8]| b.iter().fold(h, |h, x| (h ^ *x as u64).wrapping_mul(0x0100_0000_01b3));
        let high = fnv(fnv(0xcbf2_9ce4_8422_2325, b"netbird.example.com"), b"1042");
        assert_eq!(identity_seed("netbird.example.com", "1042", "netbird"), ((high as u128) << 64) | fnv(high, b"netbird") as u128);
        assert_eq!(id, uuid_v7_seeded(1_793_964_000_123, identity_seed("netbird.example.com", "1042", "netbird")));
        assert_ne!(id, uuid_v7_seeded(1_793_964_000_123, identity_seed("vpn.example.com", "1042", "netbird")));
    }

    fn mapper(yaml: &str) -> Mapper {
        let m: Manifest = serde_yaml_ng::from_str(&format!("schema_version: 1\nid: example\nname: Example\npublisher: Example\nvendor: Example\ndescription: Test.\ntables: {{ units: {{ latency: millisecond }} }}\n{yaml}")).unwrap();
        Mapper::compile(&m, &m.streams[0]).unwrap()
    }

    fn map_one(mapper: &Mapper, record: Value) -> Mapped {
        let mut scope = Map::new();
        scope.insert("record".into(), record);
        scope.insert("tables".into(), json!({ "units": { "latency": "millisecond" } }));
        let mut out = Mapped::default();
        mapper.map(&Item { scope: &scope, time_ms: None, id: "1", instance: "i", now_ms: 60_000 }, &mut out);
        out
    }

    const STREAM: &str = "streams:\n  - id: s\n    name: S\n    description: S.\n    request: { url: 'https://api.example.com' }\n    output:\n";

    #[test]
    fn a_records_own_status_stays_apart_from_the_logs() {
        let m = mapper(&format!("{STREAM}      logs: {{ message: '{{{{ record.msg }}}}', status: \"{{{{ record.ok ? 'info' : 'error' }}}}\", attributes: \"{{{{ record }}}}\" }}\n"));
        let log = &map_one(&m, json!({ "msg": "sync done", "ok": false, "status": "DONE" })).logs[0];
        assert_eq!((&log["level"], log.get("status")), (&json!("error"), None));
        assert_eq!(serde_json::from_str::<Value>(log["message"].as_str().unwrap()).unwrap(), json!({ "message": "sync done", "status": "DONE" }));
        let plain = &map_one(&m, json!({ "msg": "sync done", "ok": true })).logs[0];
        assert_eq!((&plain["status"], &plain["message"], plain.get("level")), (&json!("info"), &json!("sync done"), None));
    }

    #[test]
    fn an_attributes_template_drops_nulls_too() {
        let m = mapper(&format!("{STREAM}      logs: {{ message: x, attributes: \"{{{{ record + {{extra: null}} }}}}\" }}\n"));
        let log = &map_one(&m, json!({ "a": 1, "gone": null, "nested": { "b": null, "c": "" , "d": 2 }, "empty": {} })).logs[0];
        assert_eq!((&log["a"], &log["nested"], log.get("gone"), log.get("extra"), log.get("empty")), (&json!(1), &json!({ "d": 2 }), None, None, None));
    }

    #[test]
    fn units_may_be_templates() {
        let m = mapper(&format!("{STREAM}      metrics:\n        - {{ name: 'example.{{{{ record.k }}}}', value: '{{{{ record.v }}}}', unit: '{{{{ tables.units[record.k] }}}}' }}\n"));
        let out = map_one(&m, json!({ "k": "latency", "v": 3 }));
        assert_eq!((out.points[0].name.as_str(), out.points[0].unit.as_deref()), ("example.latency", Some("millisecond")));
        assert_eq!(map_one(&m, json!({ "k": "count", "v": 3 })).points[0].unit, None, "no unit for unknown keys");
    }

    #[test]
    fn points_are_aggregated_per_series_and_time() {
        let p = |v: f64, gpu: &str, how| Point { name: "example.gpu.util".into(), kind: MetricKind::Gauge, host: None, tags: vec![("cluster".into(), gpu.into())], time_ms: 60_000, value: v, unit: None, aggregate: how };
        let plain = Point { name: "example.other".into(), ..p(7.0, "a", None) };
        let out = aggregate(vec![p(10.0, "a", Some(MetricAggregate::Max)), plain.clone(), p(30.0, "a", Some(MetricAggregate::Max)), p(5.0, "b", Some(MetricAggregate::Max))]);
        assert_eq!(out.iter().map(|p| (p.name.as_str(), p.tags[0].1.as_str(), p.value)).collect::<Vec<_>>(), [("example.gpu.util", "a", 30.0), ("example.other", "a", 7.0), ("example.gpu.util", "b", 5.0)]);
        let values: Vec<Point> = (1..=100).map(|v| p(v as f64, "a", Some(MetricAggregate::P95))).collect();
        assert_eq!(aggregate(values)[0].value, 95.0);
        for (how, want) in [(MetricAggregate::Sum, 6.0), (MetricAggregate::Min, 1.0), (MetricAggregate::Avg, 2.0), (MetricAggregate::Count, 3.0), (MetricAggregate::P50, 2.0)] {
            assert_eq!(aggregate(vec![p(1.0, "a", Some(how)), p(2.0, "a", Some(how)), p(3.0, "a", Some(how))])[0].value, want, "{how:?}");
        }
    }

    #[test]
    fn metrics_are_gauges_or_delta_sums() {
        let p = |name: &str, kind| Point { name: name.into(), kind, host: None, tags: vec![("peer".into(), "laptop".into())], time_ms: 60_000, value: 1.0, unit: None, aggregate: None };
        let timed = Point { unit: Some("second".into()), ..p("a.gauge", MetricKind::Gauge) };
        let doc: Value = serde_json::from_slice(&metrics_otlp("netbird", &[timed, p("a.count", MetricKind::Count)])).unwrap();
        let metrics = &doc["resourceMetrics"][0]["scopeMetrics"][0]["metrics"];
        assert_eq!((&metrics[1]["unit"], &metrics[0]["unit"]), (&json!("second"), &Value::Null), "the unit as OTLP's");
        assert_eq!(metrics[0]["sum"]["aggregationTemporality"], 1);
        assert_eq!(metrics[1]["gauge"]["dataPoints"][0]["timeUnixNano"], "60000000000");
        assert_eq!(doc["resourceMetrics"][0]["resource"]["attributes"][0], kv("cloud.provider", "netbird"));
        let e = Event { title: "t".into(), text: "x".into(), time_ms: 1000, host: "h".into(), tags: vec!["a:b".into()], alert_type: "error".into(), priority: "normal".into(), source_type_name: "s".into(), aggregation_key: String::new() };
        let doc: Value = serde_json::from_slice(&events_otlp(&[e])).unwrap();
        assert_eq!(doc["resourceLogs"][0]["resource"]["attributes"][1], kv("oc.signal", "dd.events"));
        assert_eq!(doc["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["severityNumber"], 17);
    }
}
