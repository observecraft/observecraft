//! What the cloud crawlers' metric names and points have in common: Datadog-style names, glob
//! patterns, and the OTLP JSON they publish.

use serde_json::{json, Value};

/// Lowercase, words joined by `_`, nothing but `[a-z0-9_]`.
pub fn underscored(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

/// `CPUUtilization` → `cpuutilization`, `NetworkIn` → `network_in`: a word starts at an uppercase
/// letter that follows a lowercase letter (acronyms and `5XX` stay whole, as Datadog names them).
pub fn snake(metric: &str) -> String {
    let mut out = String::with_capacity(metric.len() + 4);
    let mut prev: Option<char> = None;
    for c in metric.chars() {
        if c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase()) {
            out.push('_');
        }
        out.push(c);
        prev = Some(c);
    }
    underscored(&out)
}

/// `*` matches any run of characters.
pub fn glob(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == s;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !s.starts_with(first) || !s[first.len()..].ends_with(last) || s.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &s[first.len()..s.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    true
}

/// One value of one series at one time.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub name: String,
    /// The VM the series is about (the `host`).
    pub host: Option<String>,
    /// `key:value` tags, sorted.
    pub tags: Vec<(String, String)>,
    pub time_ms: i64,
    pub value: f64,
}

fn kv(k: &str, v: &str) -> Value {
    json!({ "key": k, "value": { "stringValue": v } })
}

/// An OTLP JSON `ExportMetricsServiceRequest`: one resource per host (EC2 instances and Azure VMs are
/// hosts, as Datadog has them) with `cloud.provider`, gauges in the `observecraft.<provider>` scope,
/// data point attributes sorted (the same tags in another order would be another series to ClickHouse).
pub fn otlp_json(provider: &str, points: &[Point]) -> Vec<u8> {
    use std::collections::BTreeMap;
    let mut by_host: BTreeMap<Option<&str>, BTreeMap<&str, Vec<&Point>>> = BTreeMap::new();
    for p in points {
        by_host.entry(p.host.as_deref()).or_default().entry(&p.name).or_default().push(p);
    }
    let resource_metrics: Vec<Value> = by_host
        .into_iter()
        .map(|(host, metrics)| {
            let attributes: Vec<Value> = host.map(|h| kv("host.name", h)).into_iter().chain([kv("cloud.provider", provider)]).collect();
            let metrics: Vec<Value> = metrics
                .into_iter()
                .map(|(name, points)| {
                    let data_points: Vec<Value> = points
                        .iter()
                        .map(|p| {
                            let mut tags = p.tags.clone();
                            tags.sort();
                            json!({
                                "timeUnixNano": (p.time_ms as i128 * 1_000_000).to_string(),
                                "asDouble": p.value,
                                "attributes": tags.iter().map(|(k, v)| kv(k, v)).collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    json!({ "name": name, "gauge": { "dataPoints": data_points } })
                })
                .collect();
            json!({ "resource": { "attributes": attributes }, "scopeMetrics": [{ "scope": { "name": format!("observecraft.{provider}") }, "metrics": metrics }] })
        })
        .collect();
    serde_json::to_vec(&json!({ "resourceMetrics": resource_metrics })).expect("JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_underscored_and_camel_case_is_split_after_lowercase_letters() {
        assert_eq!(snake("CPUUtilization"), "cpuutilization");
        assert_eq!(snake("NetworkIn"), "network_in");
        assert_eq!(snake("Percentage CPU"), "percentage_cpu");
        assert_eq!(snake("Http5xx"), "http5xx");
        assert_eq!(underscored("Microsoft.Compute/virtualMachines"), "microsoft_compute_virtualmachines");
        assert!(glob("aws.ec2.network_*", "aws.ec2.network_in"));
        assert!(!glob("aws.ec2.network_*", "aws.ec2.cpuutilization"));
    }

    #[test]
    fn points_become_otlp_gauges_grouped_by_host_with_sorted_attributes() {
        let points = vec![
            Point { name: "aws.ec2.cpuutilization".into(), host: Some("i-0a1b2c3d4e5f60001".into()), tags: vec![("aws_account".into(), "123456789012".into()), ("region".into(), "us-east-1".into())], time_ms: 1_700_000_000_000, value: 12.5 },
            Point { name: "aws.ec2.cpuutilization".into(), host: Some("i-0a1b2c3d4e5f60001".into()), tags: vec![("aws_account".into(), "123456789012".into()), ("region".into(), "us-east-1".into())], time_ms: 1_700_000_060_000, value: 14.0 },
            Point { name: "aws.sqs.number_of_messages_sent.sum".into(), host: None, tags: vec![("queuename".into(), "orders".into())], time_ms: 1_700_000_000_000, value: 3.0 },
        ];
        let doc: Value = serde_json::from_slice(&otlp_json("aws", &points)).unwrap();
        let rms = doc["resourceMetrics"].as_array().unwrap();
        assert_eq!(rms.len(), 2);
        let ec2 = rms.iter().find(|r| r["resource"]["attributes"][0]["key"] == "host.name").unwrap();
        assert_eq!(ec2["resource"]["attributes"][0]["value"]["stringValue"], "i-0a1b2c3d4e5f60001");
        let metric = &ec2["scopeMetrics"][0]["metrics"][0];
        assert_eq!(metric["name"], "aws.ec2.cpuutilization");
        let dps = metric["gauge"]["dataPoints"].as_array().unwrap();
        assert_eq!(dps.len(), 2);
        assert_eq!(dps[0]["timeUnixNano"], "1700000000000000000");
        assert_eq!(dps[0]["asDouble"], 12.5);
        assert_eq!(dps[0]["attributes"][0]["key"], "aws_account");
        assert_eq!(dps[0]["attributes"][1]["key"], "region");
        assert_eq!(ec2["scopeMetrics"][0]["scope"]["name"], "observecraft.aws");
    }
}
