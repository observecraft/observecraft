//! Prowler's results: its `json-ocsf` output, an array of OCSF Detection Findings (class 2004), one per check and
//! resource. Read leniently: unknown fields are ignored, a record without a check id or resource is skipped.
//!
//! - check: `metadata.event_code`; Prowler's version `metadata.product.version`;
//! - result: `status_code` (`PASS`, `FAIL`; `MANUAL` checks need a person and `MUTED` ones Prowler's own mutelist,
//!   both skipped), `status_detail`, `severity`;
//! - resource: `resources[0]` (`uid`, `name`, `type`, `region`, `group.name` the service, `labels`), account
//!   `cloud.account.uid`;
//! - check description: `finding_info.title` / `.desc`, `risk_details`, `remediation.desc` / `.references`,
//!   compliance mappings `unmapped.compliance` (framework → requirement ids).

use std::collections::BTreeMap;

use serde_json::Value;

/// A check as the results describe it.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub check_id: String,
    pub title: String,
    pub severity: String,
    pub service: String,
    pub resource_type: String,
    pub description: String,
    pub risk: String,
    pub remediation: String,
    pub reference_urls: Vec<String>,
    pub compliance: BTreeMap<String, Vec<String>>,
}

/// One check's evaluation of one resource.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    pub check_id: String,
    pub pass: bool,
    pub severity: String,
    pub detail: String,
    pub resource_uid: String,
    pub resource_name: String,
    pub resource_type: String,
    pub region: String,
    pub account_id: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Results {
    pub checks: BTreeMap<String, Check>,
    pub evaluations: Vec<Evaluation>,
    pub tool_version: String,
    /// Records skipped (manual, muted by Prowler, unreadable).
    pub skipped: usize,
}

/// Datadog's severities from Prowler's (`Critical`, `High`, `Medium`, `Low`, `Informational`).
pub fn severity(s: &str) -> &'static str {
    match s.to_ascii_lowercase().as_str() {
        "critical" => "critical",
        "high" => "high",
        "medium" => "medium",
        "low" => "low",
        _ => "info",
    }
}

fn text(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        match cur.get(p) {
            Some(next) => cur = next,
            None => return String::new(),
        }
    }
    match cur {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// `key:value` tags from Prowler's labels (a list of strings, or of `{key, value}`) or a tag map.
fn tags(resource: &Value) -> Vec<String> {
    let mut out = vec![];
    match resource.get("labels") {
        Some(Value::Array(items)) => {
            for item in items {
                match item {
                    Value::String(s) if !s.is_empty() => out.push(s.clone()),
                    Value::Object(o) => {
                        if let (Some(k), Some(v)) = (o.get("key").and_then(Value::as_str), o.get("value").and_then(Value::as_str)) {
                            out.push(format!("{k}:{v}"));
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(Value::Object(o)) => out.extend(o.iter().map(|(k, v)| format!("{k}:{}", v.as_str().unwrap_or_default()))),
        _ => {}
    }
    out.sort();
    out.dedup();
    out
}

fn compliance(record: &Value) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(map)) = record.pointer("/unmapped/compliance") {
        for (framework, ids) in map {
            let ids: Vec<String> = match ids {
                Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                Value::String(s) => vec![s.clone()],
                _ => vec![],
            };
            if !ids.is_empty() {
                out.insert(framework.clone(), ids);
            }
        }
    }
    out
}

/// Reads a `json-ocsf` file (a JSON array; Prowler also wrote one object per line in older versions).
pub fn parse(body: &[u8]) -> Result<Results, String> {
    let records: Vec<Value> = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Array(a)) => a,
        Ok(Value::Object(o)) => vec![Value::Object(o)],
        Ok(_) => return Err("expected a JSON array of OCSF findings".into()),
        Err(_) => body
            .split(|b| *b == b'\n')
            .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
            .map(serde_json::from_slice::<Value>)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("not OCSF JSON: {e}"))?,
    };
    let mut out = Results::default();
    for r in &records {
        let check_id = text(r, &["metadata", "event_code"]);
        let resource = r.pointer("/resources/0").cloned().unwrap_or(Value::Null);
        let resource_uid = text(&resource, &["uid"]);
        let status = text(r, &["status_code"]).to_ascii_uppercase();
        if out.tool_version.is_empty() {
            out.tool_version = text(r, &["metadata", "product", "version"]);
        }
        if check_id.is_empty() || resource_uid.is_empty() || !(status == "PASS" || status == "FAIL") {
            out.skipped += 1;
            continue;
        }
        let sev = severity(&text(r, &["severity"])).to_string();
        let resource_type = text(&resource, &["type"]);
        let region = Some(text(&resource, &["region"])).filter(|s| !s.is_empty()).unwrap_or_else(|| text(r, &["cloud", "region"]));
        out.checks.entry(check_id.clone()).or_insert_with(|| Check {
            check_id: check_id.clone(),
            title: Some(text(r, &["finding_info", "title"])).filter(|s| !s.is_empty()).unwrap_or_else(|| check_id.clone()),
            severity: sev.clone(),
            service: text(&resource, &["group", "name"]),
            resource_type: resource_type.clone(),
            description: text(r, &["finding_info", "desc"]),
            risk: text(r, &["risk_details"]),
            remediation: text(r, &["remediation", "desc"]),
            reference_urls: r.pointer("/remediation/references").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).filter(|u| u.starts_with("https://")).collect()).unwrap_or_default(),
            compliance: compliance(r),
        });
        out.evaluations.push(Evaluation {
            check_id,
            pass: status == "PASS",
            severity: sev,
            detail: text(r, &["status_detail"]),
            resource_name: Some(text(&resource, &["name"])).filter(|s| !s.is_empty()).unwrap_or_else(|| resource_uid.clone()),
            resource_uid,
            resource_type,
            region,
            account_id: text(r, &["cloud", "account", "uid"]),
            tags: tags(&resource),
        });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// Shaped like Prowler 5.x's `json-ocsf` records; the account, bucket and values are the demo's.
    pub fn sample() -> Value {
        let record = |code: &str, status: &str, uid: &str, name: &str| {
            json!({
                "message": format!("S3 Bucket {name} checked"),
                "metadata": { "event_code": code, "product": { "name": "Prowler", "vendor_name": "Prowler", "version": "5.47.0" }, "version": "1.5.0" },
                "severity_id": 4, "severity": "High",
                "status": "New", "status_code": status, "status_detail": format!("S3 Bucket {name} {}.", if status == "PASS" { "is not public" } else { "is public" }),
                "status_id": 1,
                "finding_info": { "title": "Ensure S3 buckets are not publicly accessible", "desc": "Checks whether buckets allow public access.", "uid": format!("prowler-aws-{code}-123456789012-us-east-1-{name}") },
                "resources": [{ "uid": uid, "name": name, "type": "AwsS3Bucket", "region": "us-east-1", "group": { "name": "s3" }, "labels": ["team:storefront", "env:prod"] }],
                "cloud": { "account": { "uid": "123456789012", "type": "AWS Account" }, "provider": "aws", "region": "us-east-1" },
                "remediation": { "desc": "Turn on Block Public Access.", "references": ["https://docs.aws.amazon.com/AmazonS3/latest/userguide/access-control-block-public-access.html", "aws s3api put-public-access-block"] },
                "risk_details": "Public buckets can expose data.",
                "unmapped": { "compliance": { "CIS-2.0": ["2.1.4"], "SOC2": ["cc_6_1"] }, "categories": ["internet-exposed"] },
                "class_uid": 2004
            })
        };
        json!([
            record("s3_bucket_level_public_access_block", "FAIL", "arn:aws:s3:::example-shop-assets", "example-shop-assets"),
            record("s3_bucket_level_public_access_block", "PASS", "arn:aws:s3:::example-shop-logs", "example-shop-logs"),
            record("iam_root_hardware_mfa_enabled", "MANUAL", "arn:aws:iam::123456789012:root", "<root_account>"),
        ])
    }

    #[test]
    fn reads_prowler_findings() {
        let r = parse(sample().to_string().as_bytes()).unwrap();
        assert_eq!(r.tool_version, "5.47.0");
        assert_eq!(r.skipped, 1, "the manual check is left out");
        assert_eq!(r.evaluations.len(), 2);
        let fail = &r.evaluations[0];
        assert!(!fail.pass);
        assert_eq!((fail.resource_uid.as_str(), fail.resource_name.as_str(), fail.region.as_str(), fail.account_id.as_str()), ("arn:aws:s3:::example-shop-assets", "example-shop-assets", "us-east-1", "123456789012"));
        assert_eq!(fail.severity, "high");
        assert_eq!(fail.tags, ["env:prod", "team:storefront"]);
        let check = &r.checks["s3_bucket_level_public_access_block"];
        assert_eq!(check.service, "s3");
        assert_eq!(check.compliance["CIS-2.0"], ["2.1.4"]);
        assert_eq!(check.reference_urls.len(), 1, "only links are kept as references");
    }

    #[test]
    fn reads_one_record_per_line_and_rejects_other_json() {
        let lines: String = sample().as_array().unwrap().iter().map(|v| v.to_string() + "\n").collect();
        assert_eq!(parse(lines.as_bytes()).unwrap().evaluations.len(), 2);
        assert!(parse(b"42").is_err());
        assert_eq!(severity("Informational"), "info");
    }
}
