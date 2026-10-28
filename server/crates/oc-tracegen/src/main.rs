//! Synthetic distributed traces for local development, sent as OTLP/HTTP JSON to the OTel Collector
//! exactly like an instrumented application would. The topology mirrors the services in the log
//! load generator so logs and traces correlate by `service`.
//!
//! Every 10s it also reports host and process metrics for the hosts those services run on (see
//! `hosts`), so traces, logs and infrastructure correlate by `host.name`.
//!
//! About every 90s it also sends a CI/CD pipeline run (see `ci`); on first start it backfills a week
//! of pipeline history when ClickHouse has none.
//!
//! Every minute it also uploads continuous profiles like a Datadog profiler (see `profiles`), and
//! every 30 s network connections like the Agent's system-probe (see `network`).
//!
//! Every 10 s it also sends the invocations of the shop's AWS Lambda functions (see `serverless`),
//! and Kafka broker and consumer metrics like the Datadog Agent's Kafka checks (see `kafka`).
//! Submitted orders flow on through Kafka topics (`orders.created` → billing and inventory-service
//! → `payments.authorized` / `inventory.reserved` → …), for Data Streams Monitoring.
//!
//! Every other second it also sends an LLM conversation of `support-assistant` with OpenTelemetry
//! GenAI attributes (see `llm`), for LLM Observability.
//!
//! Also writes OTLP logs inside the traces (one per request handled, one per error) for log ↔ trace links.
//! Env: OTLP_ENDPOINT (default http://localhost:4318, the intake), OC_INGEST_KEY (ingest API key), TRACES_PER_SEC (default 20),
//! CLICKHOUSE_HTTP (default http://localhost:8123), CLICKHOUSE_USER / CLICKHOUSE_PASSWORD (observecraft).

mod appmetrics;
mod ci;
mod hosts;
mod kafka;
mod kubernetes;
mod llm;
mod network;
mod profiles;
mod serverless;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SERVER: i32 = 2;
const CLIENT: i32 = 3;
const PRODUCER: i32 = 4;
const CONSUMER: i32 = 5;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo).max(1)
    }
    /// Log-normal-ish latency around `median_ms` with a long tail.
    fn latency_ns(&mut self, median_ms: f64) -> u64 {
        let z: f64 = (0..4).map(|_| self.f()).sum::<f64>() - 2.0; // ~N(0, 0.58)
        let ms = median_ms * (z * 0.9).exp() * if self.chance(0.01) { 8.0 } else { 1.0 };
        (ms * 1e6) as u64
    }
    fn hex(&mut self, bytes: usize) -> String {
        (0..bytes)
            .map(|_| format!("{:02x}", self.next() as u8))
            .collect()
    }
}

struct Svc {
    name: &'static str,
    lang: &'static str,
    host: &'static str,
}

const GATEWAY: Svc = Svc {
    name: "api-gateway",
    lang: "go",
    host: "i-0a1b2c3d4e5f60001",
};
const WEB: Svc = Svc {
    name: "storefront-api",
    lang: "java",
    host: "i-0a1b2c3d4e5f60002",
};
const ORDERS: Svc = Svc {
    name: "order-service",
    lang: "go",
    host: "i-0a1b2c3d4e5f60001",
};
const BILLING: Svc = Svc {
    name: "billing",
    lang: "go",
    host: "i-0a1b2c3d4e5f60002",
};
const ACTIONS: Svc = Svc {
    name: "inventory-service",
    lang: "nodejs",
    host: "i-0a1b2c3d4e5f60003",
};
const CATALOG: Svc = Svc {
    name: "catalog-service",
    lang: "dotnet",
    host: "i-0a1b2c3d4e5f60004",
};
const BPM: Svc = Svc {
    name: "workflow-engine",
    lang: "java",
    host: "i-0a1b2c3d4e5f60004",
};
const NOTIFY: Svc = Svc {
    name: "notification-worker",
    lang: "python",
    host: "i-0a1b2c3d4e5f60005",
};

struct Span {
    svc: &'static Svc,
    trace_id: String,
    span_id: String,
    parent: Option<String>,
    name: String,
    kind: i32,
    start: u64,
    end: u64,
    attrs: Vec<(String, Value)>,
    error: Option<(String, String)>,
}

struct Trace<'a> {
    rng: &'a mut Rng,
    id: String,
    env: &'static str,
    spans: Vec<Span>,
}

impl<'a> Trace<'a> {
    fn new_id(&mut self) -> String {
        self.rng.hex(8)
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        svc: &'static Svc,
        id: String,
        parent: Option<&str>,
        name: &str,
        kind: i32,
        start: u64,
        end: u64,
        attrs: Vec<(&str, Value)>,
        error: Option<(String, String)>,
    ) {
        self.spans.push(Span {
            svc,
            trace_id: self.id.clone(),
            span_id: id,
            parent: parent.map(str::to_string),
            name: name.to_string(),
            kind,
            start,
            end,
            attrs: attrs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            error,
        });
    }

    /// A client call to a database / cache / queue (an inferred downstream service).
    fn db(
        &mut self,
        svc: &'static Svc,
        parent: &str,
        start: u64,
        peer: &str,
        system: &str,
        db_name: &str,
        stmt: &str,
        median_ms: f64,
    ) -> u64 {
        let id = self.new_id();
        let end = start + self.rng.latency_ns(median_ms);
        let attrs = vec![
            ("peer.service", json!(peer)),
            ("db.system", json!(system)),
            ("db.name", json!(db_name)),
            ("db.statement", json!(stmt)),
        ];
        self.push(
            svc,
            id,
            Some(parent),
            stmt.split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
                .as_str(),
            CLIENT,
            start,
            end,
            attrs,
            None,
        );
        end
    }
}

fn attrs_json(attrs: &[(String, Value)]) -> Vec<Value> {
    attrs
        .iter()
        .map(|(k, v)| {
            let value = match v {
                Value::String(s) => json!({ "stringValue": s }),
                Value::Number(n) if n.is_i64() => json!({ "intValue": n.to_string() }),
                Value::Number(n) => json!({ "doubleValue": n.as_f64() }),
                Value::Bool(b) => json!({ "boolValue": b }),
                other => json!({ "stringValue": other.to_string() }),
            };
            json!({ "key": k, "value": value })
        })
        .collect()
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

fn orders_version() -> &'static str {
    // Rolls every 20 minutes so the Deployments view has history.
    if (now_ns() / 1_000_000_000 / 1200) % 2 == 0 {
        "3.2.0"
    } else {
        "3.2.1"
    }
}

fn version(svc: &Svc) -> &'static str {
    match svc.name {
        "order-service" => orders_version(),
        "storefront-api" => "2.14.3",
        "billing" => "1.8.0",
        "inventory-service" => "4.1.2",
        "catalog-service" => "7.0.1",
        "workflow-engine" => "3.3.0",
        "notification-worker" => "0.9.4",
        _ => "1.0.0",
    }
}

/// HTTP server span helper: wraps `body` (which receives the server span id and start, returns child end).
fn server<F>(
    t: &mut Trace,
    svc: &'static Svc,
    parent: Option<&str>,
    start: u64,
    method: &str,
    route: &str,
    self_ms: f64,
    error: Option<(String, String)>,
    body: F,
) -> u64
where
    F: FnOnce(&mut Trace, &str, u64) -> u64,
{
    let id = t.new_id();
    let pre = t.rng.latency_ns(self_ms / 2.0);
    let child_end = body(t, &id, start + pre);
    let end = child_end + t.rng.latency_ns(self_ms / 2.0);
    let code = if error.is_some() { 500 } else { 200 };
    t.push(
        svc,
        id,
        parent,
        &format!("{method} {route}"),
        SERVER,
        start,
        end,
        vec![
            ("http.request.method", json!(method)),
            ("http.route", json!(route)),
            ("http.response.status_code", json!(code)),
        ],
        error,
    );
    end
}

/// HTTP client span in `from` calling `to`'s server span.
fn call<F>(
    t: &mut Trace,
    from: &'static Svc,
    to: &'static Svc,
    parent: &str,
    start: u64,
    method: &str,
    route: &str,
    self_ms: f64,
    error: Option<(String, String)>,
    body: F,
) -> u64
where
    F: FnOnce(&mut Trace, &str, u64) -> u64,
{
    let client_id = t.new_id();
    let net = t.rng.latency_ns(0.6);
    let server_end = server(
        t,
        to,
        Some(&client_id),
        start + net,
        method,
        route,
        self_ms,
        error.clone(),
        body,
    );
    let end = server_end + t.rng.latency_ns(0.6);
    let code = if error.is_some() { 500 } else { 200 };
    t.push(
        from,
        client_id,
        Some(parent),
        &format!("{method} {route}"),
        CLIENT,
        start,
        end,
        vec![
            ("peer.service", json!(to.name)),
            ("http.request.method", json!(method)),
            ("url.path", json!(route)),
            ("http.response.status_code", json!(code)),
        ],
        error,
    );
    end
}

fn build_trace(rng: &mut Rng) -> Vec<Span> {
    let env = if rng.chance(0.9) { "prod" } else { "staging" };
    let id = rng.hex(16);
    let mut t = Trace {
        rng,
        id,
        env,
        spans: Vec::new(),
    };
    let start = now_ns() - 2_000_000_000;
    let pick = t.rng.f();

    if pick < 0.25 {
        server(
            &mut t,
            &GATEWAY,
            None,
            start,
            "GET",
            "/api/images",
            1.0,
            None,
            |t, p, s| {
                call(
                    t,
                    &GATEWAY,
                    &WEB,
                    p,
                    s,
                    "GET",
                    "/api/images",
                    3.0,
                    None,
                    |t, p, s| {
                        t.db(
                            &WEB,
                            p,
                            s,
                            "mongo",
                            "mongodb",
                            "catalog",
                            "find images",
                            4.0,
                        )
                    },
                )
            },
        );
    } else if pick < 0.5 {
        let fail = t.rng.chance(0.04);
        let err = fail.then(|| {
            (
                "UpstreamError".to_string(),
                "GetCustomer bad response".to_string(),
            )
        });
        let n_plus_one = t.rng.chance(0.15);
        server(
            &mut t,
            &GATEWAY,
            None,
            start,
            "POST",
            "/api/orders/{id}/submit",
            1.0,
            err.clone(),
            |t, p, s| {
                call(
                    t,
                    &GATEWAY,
                    &WEB,
                    p,
                    s,
                    "POST",
                    "/api/orders/{id}/submit",
                    5.0,
                    err.clone(),
                    |t, p, s| {
                        call(
                            t,
                            &WEB,
                            &ORDERS,
                            p,
                            s,
                            "POST",
                            "/orders/submit",
                            6.0,
                            err.clone(),
                            |t, p, s| {
                                let mut cur = s;
                                for _ in 0..if n_plus_one { 6 } else { 1 } {
                                    cur = t.db(
                                        &ORDERS,
                                        p,
                                        cur + 200_000,
                                        "mongo",
                                        "mongodb",
                                        "customers",
                                        "find customers",
                                        3.0,
                                    );
                                }
                                let e = t.db(
                                    &ORDERS,
                                    p,
                                    cur + 100_000,
                                    "redis",
                                    "redis",
                                    "0",
                                    "GET customer:cache",
                                    0.4,
                                );
                                order_created(t, p, e)
                            },
                        )
                    },
                )
            },
        );
    } else if pick < 0.65 {
        let err = t.rng.chance(0.005).then(|| {
            (
                "InvoiceNotFound".to_string(),
                "invoice not found".to_string(),
            )
        });
        server(
            &mut t,
            &GATEWAY,
            None,
            start,
            "GET",
            "/api/billing/v1/invoices/{id}",
            1.0,
            err.clone(),
            |t, p, s| {
                call(
                    t,
                    &GATEWAY,
                    &BILLING,
                    p,
                    s,
                    "GET",
                    "/api/billing/v1/invoices/{id}",
                    4.0,
                    err,
                    |t, p, s| {
                        let e = t.db(
                            &BILLING,
                            p,
                            s,
                            "postgres",
                            "postgresql",
                            "billing",
                            "SELECT invoices",
                            6.0,
                        );
                        t.db(
                            &BILLING,
                            p,
                            e + 100_000,
                            "postgres",
                            "postgresql",
                            "billing",
                            "SELECT revisions",
                            3.0,
                        )
                    },
                )
            },
        );
    } else if pick < 0.85 {
        let stock_err = t.rng.chance(0.012).then(|| {
            (
                "Oracle.DataAccess.Client.OracleException".to_string(),
                "ORA-01422: exact fetch returns more than requested number of rows".to_string(),
            )
        });
        let repeated = t.rng.chance(0.3);
        server(
            &mut t,
            &GATEWAY,
            None,
            start,
            "POST",
            "/inventory/reserve",
            1.0,
            stock_err.clone(),
            |t, p, s| {
                call(
                    t,
                    &GATEWAY,
                    &ACTIONS,
                    p,
                    s,
                    "POST",
                    "/inventory/reserve",
                    8.0,
                    stock_err.clone(),
                    |t, p, s| {
                        let e = t.db(&ACTIONS, p, s, "redis", "redis", "0", "GET flow:state", 0.5);
                        let e = t.db(&ACTIONS, p, e, "redis", "redis", "0", "SET flow:state", 0.6);
                        call(
                            t,
                            &ACTIONS,
                            &CATALOG,
                            p,
                            e + 200_000,
                            "POST",
                            "/stock/levels",
                            12.0,
                            stock_err.clone(),
                            |t, p, s| {
                                let mut cur = s;
                                for _ in 0..if repeated { 7 } else { 1 } {
                                    cur = t.db(
                                        &CATALOG,
                                        p,
                                        cur + 150_000,
                                        "oracle",
                                        "oracle",
                                        "INVENTORY",
                                        "EXEC UPDATE_STOCK_PKG",
                                        9.0,
                                    );
                                }
                                cur
                            },
                        )
                    },
                )
            },
        );
    } else {
        server(
            &mut t,
            &GATEWAY,
            None,
            start,
            "POST",
            "/workflow/tasks/{id}/complete",
            1.0,
            None,
            |t, p, s| {
                call(
                    t,
                    &GATEWAY,
                    &BPM,
                    p,
                    s,
                    "POST",
                    "/workflow/tasks/{id}/complete",
                    15.0,
                    None,
                    |t, p, s| {
                        let e = t.db(
                            &BPM,
                            p,
                            s,
                            "postgres",
                            "postgresql",
                            "workflow",
                            "UPDATE tasks",
                            5.0,
                        );
                        // Producer → consumer hop through Kafka.
                        let prod = t.new_id();
                        let pend = e + t.rng.latency_ns(1.5);
                        t.push(
                            &BPM,
                            prod.clone(),
                            Some(p),
                            "order-events publish",
                            PRODUCER,
                            e,
                            pend,
                            vec![
                                ("peer.service", json!("kafka")),
                                ("messaging.system", json!("kafka")),
                                ("messaging.destination.name", json!("order-events")),
                            ],
                            None,
                        );
                        let cons = t.new_id();
                        let cstart = pend + t.rng.latency_ns(4.0);
                        let fail = t.rng.chance(0.02);
                        let mail_start = cstart + t.rng.latency_ns(1.0);
                        let mail_end = mail_start + t.rng.latency_ns(80.0);
                        let mail = t.new_id();
                        let err = fail.then(|| {
                            (
                                "HTTPError".to_string(),
                                "429 Too Many Requests from sendgrid".to_string(),
                            )
                        });
                        t.push(
                            &NOTIFY,
                            mail,
                            Some(&cons),
                            "POST /v3/mail/send",
                            CLIENT,
                            mail_start,
                            mail_end,
                            vec![
                                ("peer.service", json!("sendgrid-api")),
                                ("http.request.method", json!("POST")),
                                (
                                    "http.response.status_code",
                                    json!(if fail { 429 } else { 202 }),
                                ),
                            ],
                            err.clone(),
                        );
                        let cons_end = mail_end + t.rng.latency_ns(0.8);
                        t.push(
                            &NOTIFY,
                            cons,
                            Some(&prod),
                            "order-events process",
                            CONSUMER,
                            cstart,
                            cons_end,
                            vec![
                                ("messaging.system", json!("kafka")),
                                ("messaging.destination.name", json!("order-events")),
                                ("messaging.operation.type", json!("process")),
                                (
                                    "messaging.consumer.group.name",
                                    json!("notification-consumer"),
                                ),
                            ],
                            err,
                        );
                        pend
                    },
                )
            },
        );
    }
    let env = t.env;
    let mut spans = t.spans;
    for s in &mut spans {
        s.attrs.push(("deployment.environment".into(), json!(env)));
    }
    spans
}

/// A Kafka publish span of `svc` under `parent`: (span id, end).
fn publish(
    t: &mut Trace,
    svc: &'static Svc,
    parent: &str,
    start: u64,
    topic: &str,
) -> (String, u64) {
    let id = t.new_id();
    let end = start + t.rng.latency_ns(1.2);
    t.push(
        svc,
        id.clone(),
        Some(parent),
        &format!("{topic} publish"),
        PRODUCER,
        start,
        end,
        vec![
            ("peer.service", json!("kafka")),
            ("messaging.system", json!("kafka")),
            ("messaging.destination.name", json!(topic)),
            ("messaging.operation.type", json!("publish")),
        ],
        None,
    );
    (id, end)
}

/// A Kafka consume span of `svc` for the message `producer` published, `queued_ms` after
/// `published`; `body` gets its id and start and returns where its work ends.
#[allow(clippy::too_many_arguments)]
fn consume<F>(
    t: &mut Trace,
    svc: &'static Svc,
    producer: &str,
    published: u64,
    queued_ms: f64,
    topic: &str,
    group: &str,
    body: F,
) -> u64
where
    F: FnOnce(&mut Trace, &str, u64) -> u64,
{
    let id = t.new_id();
    let start = published + t.rng.latency_ns(queued_ms);
    let work_end = body(t, &id, start);
    let end = work_end + t.rng.latency_ns(0.3);
    t.push(
        svc,
        id,
        Some(producer),
        &format!("{topic} process"),
        CONSUMER,
        start,
        end,
        vec![
            ("messaging.system", json!("kafka")),
            ("messaging.destination.name", json!(topic)),
            ("messaging.operation.type", json!("process")),
            ("messaging.consumer.group.name", json!(group)),
        ],
        None,
    );
    end
}

/// A submitted order announced on `orders.created`: billing authorizes the payment
/// (`payments.authorized` → notification-worker) and inventory-service reserves stock
/// (`inventory.reserved` → catalog-service). Returns the publish's end.
fn order_created(t: &mut Trace, parent: &str, start: u64) -> u64 {
    let (created, published) = publish(t, &ORDERS, parent, start, "orders.created");
    let topic = "orders.created";
    consume(t, &BILLING, &created, published, 8.0, topic, "billing-consumer", |t, p, s| {
        let e = t.db(&BILLING, p, s, "postgres", "postgresql", "billing", "INSERT payments", 4.0);
        let (auth, end) = publish(t, &BILLING, p, e, "payments.authorized");
        let group = "notification-consumer";
        consume(t, &NOTIFY, &auth, end, 5.0, "payments.authorized", group, |t, _, s| {
            s + t.rng.latency_ns(3.0)
        });
        end
    });
    let lagging = kafka::inventory_lagging(now_ns() / 1_000_000_000);
    let queued = if lagging { 2_500.0 } else { 25.0 };
    consume(t, &ACTIONS, &created, published, queued, topic, "inventory-consumer", |t, p, s| {
        let e = t.db(&ACTIONS, p, s, "redis", "redis", "0", "DECRBY stock", 0.6);
        let (reserved, end) = publish(t, &ACTIONS, p, e, "inventory.reserved");
        let group = "catalog-consumer";
        consume(t, &CATALOG, &reserved, end, 10.0, "inventory.reserved", group, |t, _, s| {
            s + t.rng.latency_ns(2.0)
        });
        end
    });
    published
}

/// Application logs written inside the traces (OTLP logs with trace and span ids), so logs and APM
/// link both ways: one per request handled (server / consumer spans) and one per error.
fn otlp_logs(spans: &[Span]) -> Value {
    use std::collections::BTreeMap;
    let mut by_service: BTreeMap<&str, (&Svc, Vec<Value>)> = BTreeMap::new();
    for s in spans {
        let entry = s.kind == 2 || s.kind == 5;
        if !entry && s.error.is_none() {
            continue;
        }
        let ms = (s.end - s.start) as f64 / 1e6;
        let (severity, number, body) = match &s.error {
            Some((ty, msg)) => ("ERROR", 17, format!("{} failed: {ty}: {msg}", s.name)),
            None => ("INFO", 9, format!("{} handled in {ms:.1} ms", s.name)),
        };
        let record = json!({
            "timeUnixNano": s.end.to_string(),
            "observedTimeUnixNano": s.end.to_string(),
            "severityText": severity,
            "severityNumber": number,
            "body": { "stringValue": body },
            "traceId": s.trace_id,
            "spanId": s.span_id,
            "attributes": attrs_json(&[("code.function".into(), json!(format!("{}.handler", s.svc.name))), ("duration_ms".into(), json!(ms))]),
        });
        by_service
            .entry(s.svc.name)
            .or_insert((s.svc, Vec::new()))
            .1
            .push(record);
    }
    let resource_logs: Vec<Value> = by_service
        .into_values()
        .map(|(svc, records)| {
            json!({
                "resource": { "attributes": attrs_json(&[
                    ("service.name".into(), json!(svc.name)),
                    ("service.version".into(), json!(version(svc))),
                    ("host.name".into(), json!(svc.host)),
                ]) },
                "scopeLogs": [{ "scope": { "name": "oc-tracegen" }, "logRecords": records }],
            })
        })
        .collect();
    json!({ "resourceLogs": resource_logs })
}

fn otlp_payload(spans: Vec<Span>) -> Value {
    use std::collections::BTreeMap;
    let mut by_service: BTreeMap<&str, (&Svc, Vec<Value>, &str)> = BTreeMap::new();
    for s in &spans {
        let env = s
            .attrs
            .iter()
            .find(|(k, _)| k == "deployment.environment")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("prod");
        let mut span = json!({
            "traceId": s.trace_id,
            "spanId": s.span_id,
            "name": s.name,
            "kind": s.kind,
            "startTimeUnixNano": s.start.to_string(),
            "endTimeUnixNano": s.end.to_string(),
            "attributes": attrs_json(&s.attrs.iter().filter(|(k, _)| k != "deployment.environment").cloned().collect::<Vec<_>>()),
            "status": { "code": if s.error.is_some() { 2 } else { 1 } },
        });
        if let Some(p) = &s.parent {
            span["parentSpanId"] = json!(p);
        }
        if let Some((ty, msg)) = &s.error {
            span["status"]["message"] = json!(msg);
            span["events"] = json!([{
                "name": "exception",
                "timeUnixNano": s.end.to_string(),
                "attributes": attrs_json(&[
                    ("exception.type".into(), json!(ty)),
                    ("exception.message".into(), json!(msg)),
                    ("exception.stacktrace".into(), json!(format!("{ty}: {msg}\n    at {}.handler", s.svc.name))),
                ]),
            }]);
        }
        let key = s.svc.name;
        let entry = by_service.entry(key).or_insert((s.svc, Vec::new(), env));
        entry.1.push(span);
    }
    let resource_spans: Vec<Value> = by_service
        .into_values()
        .map(|(svc, spans, env)| {
            json!({
                "resource": { "attributes": attrs_json(&[
                    ("service.name".into(), json!(svc.name)),
                    ("service.version".into(), json!(version(svc))),
                    ("deployment.environment".into(), json!(env)),
                    ("telemetry.sdk.language".into(), json!(svc.lang)),
                    ("host.name".into(), json!(svc.host)),
                    ("k8s.namespace.name".into(), json!(format!("ns-billing-{env}"))),
                ]) },
                "scopeSpans": [{ "scope": { "name": "oc-tracegen" }, "spans": spans }],
            })
        })
        .collect();
    json!({ "resourceSpans": resource_spans })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    let endpoint =
        std::env::var("OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4318".into());
    let per_sec: u64 = std::env::var("TRACES_PER_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let url = format!("{endpoint}/v1/traces");
    let metrics_url = format!("{endpoint}/v1/metrics");
    let logs_url = format!("{endpoint}/v1/logs");
    let profile_url = format!("{endpoint}/api/v2/profile");
    let events_url = format!("{endpoint}/api/v1/events");
    let series_url = format!("{endpoint}/api/v2/series");
    let mut headers = reqwest::header::HeaderMap::new();
    if let Ok(key) = std::env::var("OC_INGEST_KEY") {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {key}").parse()?,
        );
    } else {
        tracing::warn!("OC_INGEST_KEY is not set: the intake will reject the data");
    }
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .default_headers(headers)
        .build()?;
    let mut rng = Rng(now_ns() | 1);
    tracing::info!(%url, per_sec, "oc-tracegen started");

    if let Err(e) = backfill_ci(&http, &url, &mut rng).await {
        tracing::warn!(error = %e, "CI history backfill skipped");
    }
    if let Err(e) = backfill_profiles(&http, &profile_url, &mut rng).await {
        tracing::warn!(error = %e, "profile backfill skipped");
    }
    if let Err(e) = backfill_network(&http, &logs_url, &mut rng).await {
        tracing::warn!(error = %e, "network backfill skipped");
    }

    kubernetes::backfill(&http, &metrics_url, now_ns()).await;
    // Half an hour of Kafka metrics, a point a minute, so the console's charts have history.
    let now_s = now_ns() / 1_000_000_000;
    for minutes in (1..=30u64).rev() {
        send_kafka(&http, &series_url, now_s - minutes * 60).await;
    }

    let mut host_state = hosts::initial_state(&mut rng, now_ns());
    let mut app_metrics = appmetrics::AppMetrics::new(now_ns());
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut seconds = 0u64;
    loop {
        tick.tick().await;
        if seconds % 10 == 0 {
            let payload = hosts::metrics_payload(&mut rng, &mut host_state, now_ns(), 10.0);
            if let Err(e) = http
                .post(&metrics_url)
                .json(&payload)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                tracing::warn!(error = %e, "metrics export failed");
            }
            let payload = kubernetes::metrics_payload(now_ns());
            if let Err(e) = http
                .post(&metrics_url)
                .json(&payload)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                tracing::warn!(error = %e, "Kubernetes metrics export failed");
            }
            let (lambda_traces, lambda_metrics) = serverless::payloads(&mut rng, now_ns(), 10.0);
            for (target, payload, what) in [
                (&url, lambda_traces, "Lambda spans"),
                (&metrics_url, lambda_metrics, "Lambda metrics"),
            ] {
                if let Err(e) = http
                    .post(target)
                    .json(&payload)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                {
                    tracing::warn!(error = %e, "{what} export failed");
                }
            }
            let payload = app_metrics.payload(&mut rng, now_ns());
            if let Err(e) = http
                .post(&metrics_url)
                .json(&payload)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                tracing::warn!(error = %e, "application metrics export failed");
            }
        }
        if seconds % 10 == 5 {
            send_kafka(&http, &series_url, now_ns() / 1_000_000_000).await;
        }
        if seconds.is_multiple_of(60) {
            kubernetes::send_events(&http, &events_url, now_ns()).await;
        }
        if seconds % 90 == 45 {
            send_ci(&http, &url, ci_push_ending_now(&mut rng)).await;
        }
        if seconds % 60 == 5 {
            profiles::send(&http, &profile_url, &mut rng, now_ns()).await;
        }
        if seconds % 30 == 15 {
            send_network(&http, &logs_url, &mut rng, now_ns(), true).await;
        }
        seconds += 1;
        let n = rng.range(per_sec * 7 / 10, per_sec * 13 / 10 + 1);
        let mut spans: Vec<Span> = (0..n).flat_map(|_| build_trace(&mut rng)).collect();
        // About one support-assistant conversation every two seconds (LLM Observability).
        if seconds.is_multiple_of(2) {
            spans.extend(llm::support_trace(&mut rng, now_ns() - 4_000_000_000));
        }
        let count = spans.len();
        let logs = otlp_logs(&spans);
        for s in &spans {
            let secs = (s.end - s.start) as f64 / 1e9;
            match s.kind {
                2 => app_metrics.observe_http(s.svc.name, s.svc.host, &s.name, secs),
                5 => app_metrics.observe_messaging(s.svc.name, s.svc.host, secs),
                _ => {}
            }
        }
        if let Err(e) = http
            .post(&logs_url)
            .json(&logs)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            tracing::warn!(error = %e, "logs export failed");
        }
        match http.post(&url).json(&otlp_payload(spans)).send().await {
            Ok(r) if r.status().is_success() => tracing::debug!(traces = n, spans = count, "sent"),
            Ok(r) => tracing::warn!(status = %r.status(), "collector rejected batch"),
            Err(e) => tracing::warn!(error = %e, "collector unreachable"),
        }
    }
}

/// A CI push whose last run ends now: built once to learn its duration, then rebuilt from the same
/// random state shifted back by that duration.
fn ci_push_ending_now(rng: &mut Rng) -> Vec<Value> {
    let now = now_ns();
    let seed = rng.0;
    let (_, end) = ci::push(&mut Rng(seed), now);
    let (runs, _) = ci::push(rng, now - (end - now));
    runs
}

async fn send_ci(http: &reqwest::Client, url: &str, resource_spans: Vec<Value>) {
    if let Err(e) = http
        .post(url)
        .json(&json!({ "resourceSpans": resource_spans }))
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        tracing::warn!(error = %e, "CI export failed");
    }
}

/// Fills the last 7 days with pipeline runs when ClickHouse has (almost) none yet.
/// The Agent's Kafka checks' series at `now_s` (Datadog `/api/v2/series` JSON).
async fn send_kafka(http: &reqwest::Client, url: &str, now_s: u64) {
    if let Err(e) = http
        .post(url)
        .json(&kafka::series_payload(now_s))
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        tracing::warn!(error = %e, "Kafka metrics export failed");
    }
}

async fn send_network(
    http: &reqwest::Client,
    url: &str,
    rng: &mut Rng,
    now: u64,
    connections: bool,
) {
    if let Err(e) = http
        .post(url)
        .json(&network::payload(rng, now, connections))
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        tracing::warn!(error = %e, "network connections export failed");
    }
}

/// The last hour of network connections and DNS stats, when there are none yet (DNS alone when
/// only the connections exist, from before DNS was generated).
async fn backfill_network(http: &reqwest::Client, url: &str, rng: &mut Rng) -> anyhow::Result<()> {
    let ch = std::env::var("CLICKHOUSE_HTTP").unwrap_or_else(|_| "http://localhost:8123".into());
    let user = std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "observecraft".into());
    let password = std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_else(|_| "observecraft".into());
    let counts = http
        .get(&ch)
        .basic_auth(user, Some(password))
        .query(&[("query", "SELECT countIf(LogAttributes['kind'] != 'dns'), countIf(LogAttributes['kind'] = 'dns') FROM otel.otel_network WHERE Timestamp > now() - INTERVAL 70 MINUTE")])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut it = counts.split_whitespace().map(|n| n.parse::<u64>());
    let (conns, dns) = (it.next().unwrap_or(Ok(0))?, it.next().unwrap_or(Ok(0))?);
    if conns >= 100 && dns >= 100 {
        return Ok(());
    }
    let now = now_ns();
    for slot in (1..=120u64).rev() {
        send_network(http, url, rng, now - slot * 30_000_000_000, conns < 100).await;
    }
    tracing::info!("backfilled an hour of network data");
    Ok(())
}

/// The last hour of profiles, when there are none yet (so the Profiles explorer and its insights
/// have data right away).
async fn backfill_profiles(http: &reqwest::Client, url: &str, rng: &mut Rng) -> anyhow::Result<()> {
    let ch = std::env::var("CLICKHOUSE_HTTP").unwrap_or_else(|_| "http://localhost:8123".into());
    let user = std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "observecraft".into());
    let password = std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_else(|_| "observecraft".into());
    let count: u64 = http
        .get(&ch)
        .basic_auth(user, Some(password))
        .query(&[(
            "query",
            "SELECT count() FROM otel.profiles WHERE Start > now() - INTERVAL 70 MINUTE",
        )])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?
        .trim()
        .parse()?;
    if count >= 20 {
        return Ok(());
    }
    let now = now_ns();
    for minutes_ago in (1..=60u64).rev() {
        profiles::send(http, url, rng, now - minutes_ago * 60_000_000_000).await;
    }
    tracing::info!("backfilled an hour of profiles");
    Ok(())
}

async fn backfill_ci(http: &reqwest::Client, url: &str, rng: &mut Rng) -> anyhow::Result<()> {
    let ch = std::env::var("CLICKHOUSE_HTTP").unwrap_or_else(|_| "http://localhost:8123".into());
    let user = std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "observecraft".into());
    let password = std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_else(|_| "observecraft".into());
    let count: u64 = http
        .get(&ch)
        .basic_auth(user, Some(password))
        .query(&[("query", "SELECT count() FROM otel.otel_ci_traces WHERE ParentSpanId = '' AND Timestamp > now() - INTERVAL 6 DAY")])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?
        .trim()
        .parse()?;
    if count >= 50 {
        return Ok(());
    }
    let now = now_ns();
    let mut t = now - 7 * 86_400 * 1_000_000_000;
    let mut batch = Vec::new();
    let mut runs = 0;
    while t < now - 3600 * 1_000_000_000 {
        let (r, _) = ci::push(rng, t);
        runs += r.len();
        batch.extend(r);
        if batch.len() >= 40 {
            send_ci(http, url, std::mem::take(&mut batch)).await;
        }
        // Busier during working hours.
        let hour = (t / 1_000_000_000 / 3600) % 24;
        let gap = if (7..19).contains(&hour) {
            rng.range(3, 9)
        } else {
            rng.range(15, 45)
        };
        t += gap * 60 * 1_000_000_000;
    }
    send_ci(http, url, batch).await;
    tracing::info!(runs, "backfilled a week of CI pipeline runs");
    Ok(())
}
