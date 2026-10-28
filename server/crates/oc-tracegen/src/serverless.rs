//! The demo shop's AWS Lambda functions (account 123456789012), every 10 seconds, in the shapes the
//! Serverless page reads:
//! - functions with the Datadog Lambda extension: `aws.lambda` spans with the Datadog tracer's tags
//!   (`functionname`, `function_arn`, `cold_start`, …) and the enhanced metrics `aws.lambda.enhanced.*`
//!   as delta exponential histograms tagged like the extension tags them (what the intake stores for
//!   the extension's distributions);
//! - `inventory-sync`, instrumented with OpenTelemetry: spans with `faas.*` resource and span
//!   attributes, and `faas.invocations` / `faas.errors` / `faas.coldstarts` / `faas.timeouts` sums.

use serde_json::{json, Value};

use crate::{attrs_json, Rng};

const ACCOUNT: &str = "123456789012";

struct Function {
    name: &'static str,
    region: &'static str,
    service: &'static str,
    team: &'static str,
    runtime: &'static str,
    memory_mb: f64,
    architecture: &'static str,
    /// Invocations per 10 s.
    rate: f64,
    error_p: f64,
    cold_p: f64,
    median_ms: f64,
    /// Peak memory used, as a fraction of the allocation.
    memory_used: f64,
    timeout_p: f64,
    oom_p: f64,
    trigger: &'static str,
    datadog_lambda: &'static str,
    extension: &'static str,
    /// Instrumented with OpenTelemetry instead of the Datadog extension.
    otel: bool,
}

const FUNCTIONS: &[Function] = &[
    Function {
        name: "checkout-webhook",
        region: "us-east-1",
        service: "checkout",
        team: "payments",
        runtime: "nodejs20.x",
        memory_mb: 1024.0,
        architecture: "arm64",
        rate: 6.0,
        error_p: 0.065,
        cold_p: 0.02,
        median_ms: 190.0,
        memory_used: 0.46,
        timeout_p: 0.0,
        oom_p: 0.0,
        trigger: "api-gateway",
        datadog_lambda: "v10.12.1",
        extension: "88",
        otel: false,
    },
    Function {
        name: "order-confirmation-mailer",
        region: "us-east-1",
        service: "notifications",
        team: "customer-comms",
        runtime: "python3.9",
        memory_mb: 256.0,
        architecture: "x86_64",
        rate: 3.0,
        error_p: 0.01,
        cold_p: 0.14,
        median_ms: 340.0,
        memory_used: 0.94,
        timeout_p: 0.0,
        oom_p: 0.0,
        trigger: "sns",
        datadog_lambda: "v6.98.0",
        extension: "81",
        otel: false,
    },
    Function {
        name: "image-resizer",
        region: "us-east-1",
        service: "catalog-service",
        team: "catalog",
        runtime: "nodejs18.x",
        memory_mb: 1024.0,
        architecture: "x86_64",
        rate: 2.0,
        error_p: 0.015,
        cold_p: 0.05,
        median_ms: 820.0,
        memory_used: 0.99,
        timeout_p: 0.0,
        oom_p: 0.01,
        trigger: "s3",
        datadog_lambda: "v10.4.0",
        extension: "84",
        otel: false,
    },
    Function {
        name: "cart-abandonment-notifier",
        region: "us-east-1",
        service: "notifications",
        team: "customer-comms",
        runtime: "python3.13",
        memory_mb: 2048.0,
        architecture: "x86_64",
        rate: 0.5,
        error_p: 0.0,
        cold_p: 0.4,
        median_ms: 90.0,
        memory_used: 0.08,
        timeout_p: 0.0,
        oom_p: 0.0,
        trigger: "eventbridge",
        datadog_lambda: "v6.104.0",
        extension: "88",
        otel: false,
    },
    Function {
        name: "inventory-sync",
        region: "eu-west-1",
        service: "inventory-service",
        team: "inventory",
        runtime: "python3.12",
        memory_mb: 512.0,
        architecture: "arm64",
        rate: 0.4,
        error_p: 0.025,
        cold_p: 0.06,
        median_ms: 9_000.0,
        memory_used: 0.72,
        timeout_p: 0.03,
        oom_p: 0.0,
        trigger: "timer",
        datadog_lambda: "",
        extension: "",
        otel: true,
    },
];

/// Exponential histogram scale of the converted sketches (the intake uses the same).
const SCALE: i32 = 6;

fn arn(f: &Function) -> String {
    format!("arn:aws:lambda:{}:{ACCOUNT}:function:{}", f.region, f.name)
}

/// One invocation: duration (ms), cold start, error, timeout, out of memory.
struct Invocation {
    start: u64,
    ms: f64,
    cold: bool,
    error: bool,
    timeout: bool,
    oom: bool,
    memory_mb: f64,
}

fn invocations(rng: &mut Rng, f: &Function, now: u64, interval_s: f64) -> Vec<Invocation> {
    let expected = f.rate * interval_s / 10.0;
    let n = (expected * (0.7 + rng.f() * 0.6)).floor() as u64 + u64::from(rng.chance(expected.fract()));
    (0..n)
        .map(|_| {
            let cold = rng.chance(f.cold_p);
            let timeout = rng.chance(f.timeout_p);
            let oom = !timeout && rng.chance(f.oom_p);
            let mut ms = rng.latency_ns(f.median_ms) as f64 / 1e6 + if cold { 350.0 } else { 0.0 };
            if timeout {
                ms = 15_000.0;
            }
            let start = now - (rng.f() * interval_s * 1e9) as u64 - (ms * 1e6) as u64;
            let memory_mb = (f.memory_mb * (f.memory_used * (0.9 + rng.f() * 0.1))).min(f.memory_mb).round();
            Invocation { start, ms, cold, error: timeout || oom || rng.chance(f.error_p), timeout, oom, memory_mb: if oom { f.memory_mb } else { memory_mb } }
        })
        .collect()
}

fn span(rng: &mut Rng, f: &Function, i: &Invocation) -> Value {
    let request_id = format!("{}-{}-{}-{}-{}", rng.hex(4), rng.hex(2), rng.hex(2), rng.hex(2), rng.hex(6));
    let mut attrs: Vec<(String, Value)> = if f.otel {
        vec![("faas.coldstart".into(), json!(i.cold.to_string())), ("faas.trigger".into(), json!(f.trigger)), ("faas.invocation_id".into(), json!(request_id))]
    } else {
        vec![
            ("functionname".into(), json!(f.name)),
            ("function_arn".into(), json!(arn(f))),
            ("function_version".into(), json!("$LATEST")),
            ("cold_start".into(), json!(i.cold.to_string())),
            ("request_id".into(), json!(request_id)),
            ("function_trigger.event_source".into(), json!(f.trigger)),
            ("datadog_lambda".into(), json!(f.datadog_lambda)),
            ("span.type".into(), json!("serverless")),
        ]
    };
    let message = if i.timeout {
        "Task timed out after 15.00 seconds"
    } else if i.oom {
        "Runtime exited with error: signal: killed (out of memory)"
    } else {
        "upstream returned 503"
    };
    if i.error {
        attrs.push(("error.message".into(), json!(message)));
    }
    let mut status = json!({ "code": if i.error { 2 } else { 1 } });
    if i.error {
        status["message"] = json!(message);
    }
    json!({
        "traceId": rng.hex(16),
        "spanId": rng.hex(8),
        "name": if f.otel { f.name } else { "aws.lambda" },
        "kind": 2,
        "startTimeUnixNano": i.start.to_string(),
        "endTimeUnixNano": (i.start + (i.ms * 1e6) as u64).to_string(),
        "attributes": attrs_json(&attrs),
        "status": status,
    })
}

fn resource(f: &Function) -> Value {
    let mut attrs: Vec<(String, Value)> = vec![("service.name".into(), json!(f.service)), ("deployment.environment".into(), json!("prod"))];
    if f.otel {
        attrs.extend([
            ("cloud.provider".into(), json!("aws")),
            ("cloud.platform".into(), json!("aws_lambda")),
            ("cloud.region".into(), json!(f.region)),
            ("cloud.account.id".into(), json!(ACCOUNT)),
            ("faas.name".into(), json!(f.name)),
            ("faas.version".into(), json!("$LATEST")),
            ("faas.max_memory".into(), json!(((f.memory_mb * 1024.0 * 1024.0) as i64).to_string())),
            ("team".into(), json!(f.team)),
            ("telemetry.sdk.language".into(), json!("python")),
        ]);
    }
    json!({ "attributes": attrs_json(&attrs) })
}

/// The extension's tags on its enhanced metrics.
fn extension_tags(f: &Function, cold: Option<bool>) -> Vec<(String, Value)> {
    let mut tags: Vec<(String, Value)> = [
        ("functionname", f.name.to_string()),
        ("region", f.region.into()),
        ("account_id", ACCOUNT.into()),
        ("aws_account", ACCOUNT.into()),
        ("memorysize", format!("{}", f.memory_mb)),
        ("runtime", f.runtime.into()),
        ("architecture", f.architecture.into()),
        ("resource", f.name.into()),
        ("executedversion", "$LATEST".into()),
        ("team", f.team.into()),
        ("stage", "prod".into()),
        ("datadog_lambda", f.datadog_lambda.into()),
        ("dd_extension_version", f.extension.into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), json!(v)))
    .collect();
    if let Some(c) = cold {
        tags.push(("cold_start".into(), json!(c.to_string())));
    }
    tags
}

/// A delta exponential histogram of `values` (as the intake writes the extension's distributions).
fn exp_histogram(name: &str, values: &[f64], tags: &[(String, Value)], start: u64, now: u64) -> Option<Value> {
    if values.is_empty() {
        return None;
    }
    let idx: Vec<i32> = values.iter().map(|v| (v.max(1e-9).log2() * f64::from(1 << SCALE)).ceil() as i32 - 1).collect();
    let (lo, hi) = (*idx.iter().min()?, *idx.iter().max()?);
    let mut counts = vec![0u64; (hi - lo + 1) as usize];
    for i in &idx {
        counts[(i - lo) as usize] += 1;
    }
    Some(json!({
        "name": name,
        "exponentialHistogram": { "aggregationTemporality": 1, "dataPoints": [{
            "startTimeUnixNano": start.to_string(), "timeUnixNano": now.to_string(),
            "count": values.len().to_string(), "sum": values.iter().sum::<f64>(),
            "min": values.iter().copied().fold(f64::INFINITY, f64::min), "max": values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            "scale": SCALE, "zeroCount": "0",
            "positive": { "offset": lo, "bucketCounts": counts.iter().map(|c| c.to_string()).collect::<Vec<_>>() },
            "attributes": attrs_json(tags),
        }]},
    }))
}

fn delta_sum(name: &str, value: f64, start: u64, now: u64) -> Value {
    json!({ "name": name, "sum": { "aggregationTemporality": 1, "isMonotonic": true, "dataPoints": [{
        "startTimeUnixNano": start.to_string(), "timeUnixNano": now.to_string(), "asDouble": value,
    }]}})
}

fn cost(f: &Function, ms: f64) -> f64 {
    let price = if f.architecture == "arm64" { 0.000_013_333_4 } else { 0.000_016_666_7 };
    0.2 / 1e6 + ms.ceil() / 1000.0 * f.memory_mb / 1024.0 * price
}

fn metrics(f: &Function, calls: &[Invocation], start: u64, now: u64) -> Vec<Value> {
    if f.otel {
        let count = |p: fn(&Invocation) -> bool| calls.iter().filter(|i| p(i)).count() as f64;
        return vec![
            delta_sum("faas.invocations", calls.len() as f64, start, now),
            delta_sum("faas.errors", count(|i| i.error), start, now),
            delta_sum("faas.coldstarts", count(|i| i.cold), start, now),
            delta_sum("faas.timeouts", count(|i| i.timeout), start, now),
        ];
    }
    let tags = extension_tags(f, None);
    let ones = |p: fn(&Invocation) -> bool| calls.iter().filter(|i| p(i)).map(|_| 1.0).collect::<Vec<f64>>();
    let mut out: Vec<Option<Value>> = vec![
        exp_histogram("aws.lambda.enhanced.invocations", &ones(|i| i.cold), &extension_tags(f, Some(true)), start, now),
        exp_histogram("aws.lambda.enhanced.invocations", &ones(|i| !i.cold), &extension_tags(f, Some(false)), start, now),
        exp_histogram("aws.lambda.enhanced.errors", &ones(|i| i.error), &tags, start, now),
        exp_histogram("aws.lambda.enhanced.timeouts", &ones(|i| i.timeout), &tags, start, now),
        exp_histogram("aws.lambda.enhanced.out_of_memory", &ones(|i| i.oom), &tags, start, now),
    ];
    let durations: Vec<f64> = calls.iter().map(|i| i.ms / 1000.0).collect();
    out.push(exp_histogram("aws.lambda.enhanced.duration", &durations, &tags, start, now));
    out.push(exp_histogram("aws.lambda.enhanced.billed_duration", &calls.iter().map(|i| i.ms.ceil() / 1000.0).collect::<Vec<_>>(), &tags, start, now));
    out.push(exp_histogram("aws.lambda.enhanced.init_duration", &calls.iter().filter(|i| i.cold).map(|_| 0.35).collect::<Vec<_>>(), &tags, start, now));
    out.push(exp_histogram("aws.lambda.enhanced.max_memory_used", &calls.iter().map(|i| i.memory_mb).collect::<Vec<_>>(), &tags, start, now));
    out.push(exp_histogram("aws.lambda.enhanced.estimated_cost", &calls.iter().map(|i| cost(f, i.ms)).collect::<Vec<_>>(), &tags, start, now));
    out.into_iter().flatten().collect()
}

/// (OTLP traces, OTLP metrics) of the functions' invocations in the last `interval_s` seconds.
pub fn payloads(rng: &mut Rng, now: u64, interval_s: f64) -> (Value, Value) {
    let start = now - (interval_s * 1e9) as u64;
    let mut resource_spans = Vec::new();
    let mut resource_metrics = Vec::new();
    for f in FUNCTIONS {
        let calls = invocations(rng, f, now, interval_s);
        let spans: Vec<Value> = calls.iter().map(|i| span(rng, f, i)).collect();
        if !spans.is_empty() {
            resource_spans
                .push(json!({ "resource": resource(f), "scopeSpans": [{ "scope": { "name": if f.otel { "opentelemetry.instrumentation.aws_lambda" } else { "datadog-lambda" } }, "spans": spans }] }));
        }
        let metrics = metrics(f, &calls, start, now);
        if !metrics.is_empty() {
            resource_metrics.push(json!({ "resource": resource(f), "scopeMetrics": [{ "scope": { "name": "datadog-lambda-extension" }, "metrics": metrics }] }));
        }
    }
    (json!({ "resourceSpans": resource_spans }), json!({ "resourceMetrics": resource_metrics }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr<'a>(attrs: &'a Value, key: &str) -> Option<&'a str> {
        attrs.as_array()?.iter().find(|a| a["key"] == key)?["value"]["stringValue"].as_str()
    }

    #[test]
    fn spans_and_metrics_of_every_function() {
        let mut rng = Rng(42);
        let now = 1_760_000_000_000_000_000;
        let (traces, metrics) = payloads(&mut rng, now, 600.0);
        let rs = traces["resourceSpans"].as_array().unwrap();
        assert_eq!(rs.len(), FUNCTIONS.len(), "ten minutes invoke every function");
        let dd = rs.iter().find(|r| r["scopeSpans"][0]["spans"][0]["name"] == "aws.lambda").unwrap();
        let s = &dd["scopeSpans"][0]["spans"][0];
        assert!(attr(&s["attributes"], "function_arn").unwrap().starts_with("arn:aws:lambda:us-east-1:123456789012:function:"));
        assert!(matches!(attr(&s["attributes"], "cold_start"), Some("true" | "false")));
        let otel = rs.iter().find(|r| attr(&r["resource"]["attributes"], "faas.name") == Some("inventory-sync")).unwrap();
        assert_eq!(attr(&otel["resource"]["attributes"], "faas.max_memory"), Some("536870912"));

        let rm = metrics["resourceMetrics"].as_array().unwrap();
        let names: Vec<&str> = rm.iter().flat_map(|r| r["scopeMetrics"][0]["metrics"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap())).collect();
        for n in ["aws.lambda.enhanced.invocations", "aws.lambda.enhanced.duration", "aws.lambda.enhanced.max_memory_used", "aws.lambda.enhanced.estimated_cost", "faas.invocations", "faas.coldstarts"]
        {
            assert!(names.contains(&n), "{n}");
        }
        // Invocation histograms count what the spans show.
        let checkout_spans =
            rs.iter().find(|r| attr(&r["scopeSpans"][0]["spans"][0]["attributes"], "functionname") == Some("checkout-webhook")).unwrap()["scopeSpans"][0]["spans"].as_array().unwrap().len();
        let checkout = rm
            .iter()
            .find(|r| r["scopeMetrics"][0]["metrics"].as_array().unwrap().iter().any(|m| attr(&m["exponentialHistogram"]["dataPoints"][0]["attributes"], "functionname") == Some("checkout-webhook")))
            .unwrap();
        let counted: u64 = checkout["scopeMetrics"][0]["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["name"] == "aws.lambda.enhanced.invocations")
            .map(|m| m["exponentialHistogram"]["dataPoints"][0]["count"].as_str().unwrap().parse::<u64>().unwrap())
            .sum();
        assert_eq!(counted as usize, checkout_spans);
    }

    #[test]
    fn histogram_buckets_hold_their_values() {
        let h = exp_histogram("x", &[0.2, 0.25, 3.0], &[], 0, 1).unwrap();
        let p = &h["exponentialHistogram"]["dataPoints"][0];
        let offset = p["positive"]["offset"].as_i64().unwrap() as i32;
        let counts: Vec<u64> = p["positive"]["bucketCounts"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().parse().unwrap()).collect();
        assert_eq!(counts.iter().sum::<u64>(), 3);
        let base = 2f64.powf(2f64.powi(-SCALE));
        // Bucket i holds (base^i, base^(i+1)].
        assert!(base.powi(offset) < 0.2 && 0.2 <= base.powi(offset + 1));
        assert!(exp_histogram("x", &[], &[], 0, 1).is_none());
    }
}
