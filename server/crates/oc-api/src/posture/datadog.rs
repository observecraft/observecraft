//! Findings as Datadog's security findings API returns them (`/api/v2/security/findings`, `finding_type:
//! misconfiguration`), and its `filter[query]` (the logs search syntax on the findings' facets).
//!
//! - **Ids** are base64 of `<rule id>~<finding>` as Datadog's are; ours carry the finding row's id after the `~`.
//! - **Document**: `attributes.attributes.custom` holds the finding (`cloud_resource`, `compliance.evaluation`,
//!   `rule`, `severity`, `status` `open` / `muted`, `workflow.mute`, …), `attributes.tags` its tags
//!   (`resource_type:`, `cloud_provider:`, `framework:`, the resource's own), `attributes.timestamp` the last
//!   evaluation change.
//! - **Query**: space-separated terms, all of which must match: `@facet:value` (a facet of the document, `.`-joined
//!   path; values may end in `*`), `@facet:(a OR b)`, `tag:value`, a bare word (title, resource name or id),
//!   `-` to negate one, `*` for everything. Unsupported syntax is an error rather than a guess.

use base64::Engine;
use serde_json::{json, Map, Value};

use super::store::FindingRow;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub fn finding_id(row: &FindingRow) -> String {
    B64.encode(format!("{}~{}", row.check_id, row.id))
}

/// The finding row id an API id names (None when it isn't one of ours).
pub fn row_id(api_id: &str) -> Option<String> {
    let decoded = String::from_utf8(B64.decode(api_id.trim()).ok()?).ok()?;
    let (_, id) = decoded.rsplit_once('~')?;
    is_uuid(id).then(|| id.to_ascii_lowercase())
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

fn status(row: &FindingRow) -> &'static str {
    if row.muted {
        "muted"
    } else {
        "open"
    }
}

/// `framework/requirement` controls from the check's compliance mappings.
fn controls(row: &FindingRow) -> Vec<String> {
    let mut out = vec![];
    if let Some(Value::Object(map)) = &row.compliance {
        for (framework, ids) in map {
            for id in ids.as_array().into_iter().flatten().filter_map(Value::as_str) {
                out.push(format!("{framework}/{id}"));
            }
        }
    }
    out
}

pub fn tags(row: &FindingRow) -> Vec<String> {
    let mut tags = vec![
        format!("cloud_provider:{}", row.provider),
        format!("account_id:{}", row.account_id),
        format!("resource_type:{}", row.resource_type),
        format!("evaluation:{}", row.evaluation),
        format!("severity:{}", row.severity),
        format!("rule_id:{}", row.check_id),
        "source:prowler".to_string(),
    ];
    if !row.region.is_empty() {
        tags.push(format!("region:{}", row.region));
    }
    if let Some(Value::Object(map)) = &row.compliance {
        tags.extend(map.keys().map(|f| format!("framework:{f}")));
    }
    tags.extend(row.resource_tags.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
    tags
}

/// The finding's document (`attributes.attributes.custom`).
pub fn custom(row: &FindingRow) -> Value {
    let title = row.title.clone().unwrap_or_else(|| row.check_id.clone());
    let mut description = row.description.clone().unwrap_or_default();
    if let Some(risk) = row.risk.as_deref().filter(|r| !r.is_empty()) {
        description.push_str(&format!("\n\n## Risk\n\n{risk}"));
    }
    if let Some(fix) = row.remediation.as_deref().filter(|r| !r.is_empty()) {
        description.push_str(&format!("\n\n## Remediation\n\n{fix}"));
    }
    let mut mute = Map::new();
    mute.insert("is_muted".into(), json!(row.muted));
    if row.muted {
        mute.insert("reason".into(), json!(row.mute_reason));
        mute.insert("description".into(), json!(row.mute_description));
        mute.insert("expire_at".into(), json!(row.mute_expire_at));
        mute.insert("muted_by".into(), json!(row.muted_by));
        mute.insert("muted_at".into(), json!(row.muted_at));
    }
    json!({
        "cloud_resource": { "account": row.account_id, "cloud_provider": row.provider, "region": row.region, "category": row.service.clone().unwrap_or_default() },
        "compliance": { "evaluation": row.evaluation, "framework_requirement_controls": controls(row) },
        "description": description,
        "detection_changed_at": row.evaluation_changed_at,
        "finding_id": finding_id(row),
        "finding_type": "misconfiguration",
        "first_seen_at": row.first_seen_at,
        "last_seen_at": row.last_seen_at,
        "metadata": { "schema_version": "2" },
        "message": row.detail,
        "origin": ["prowler"],
        "remediation": { "description": row.remediation, "references": row.reference_urls.clone().unwrap_or(json!([])) },
        "resource_id": row.resource_uid,
        "resource_name": row.resource_name,
        "resource_type": row.resource_type,
        "rule": { "id": row.check_id, "name": title, "type": "cloud configuration" },
        "severity": row.severity,
        "status": status(row),
        "title": title,
        "workflow": { "mute": mute },
    })
}

/// One item of a findings list.
pub fn item(row: &FindingRow) -> Value {
    json!({
        "id": finding_id(row),
        "type": "finding",
        "attributes": { "attributes": { "custom": custom(row) }, "tags": tags(row), "timestamp": row.evaluation_changed_at },
    })
}

// ================================================================ filter[query]

#[derive(Debug, Clone, PartialEq)]
enum Term {
    All,
    Facet { path: Vec<String>, values: Vec<String> },
    Tag { key: String, values: Vec<String> },
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query(Vec<(bool, Term)>);

/// Splits on spaces outside parentheses and quotes.
fn tokens(q: &str) -> Result<Vec<String>, String> {
    let (mut out, mut cur, mut depth, mut quoted) = (vec![], String::new(), 0i32, false);
    for c in q.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            '(' if !quoted => {
                depth += 1;
                cur.push(c);
            }
            ')' if !quoted => {
                depth -= 1;
                if depth < 0 {
                    return Err("unbalanced parentheses".into());
                }
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if depth != 0 || quoted {
        return Err("unbalanced parentheses or quotes".into());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').to_string()
}

/// `a`, `"a b"` or `(a OR b OR "c d")`.
fn values(v: &str) -> Result<Vec<String>, String> {
    let v = v.trim();
    if let Some(inner) = v.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<String> = tokens(inner)?;
        let mut out = vec![];
        for (i, p) in parts.iter().enumerate() {
            if i % 2 == 1 {
                if p != "OR" {
                    return Err(format!("only OR is supported inside parentheses, not {p}"));
                }
            } else {
                out.push(unquote(p));
            }
        }
        if out.is_empty() {
            return Err("empty value list".into());
        }
        return Ok(out);
    }
    Ok(vec![unquote(v)])
}

pub fn parse_query(q: &str) -> Result<Query, String> {
    let mut terms = vec![];
    for tok in tokens(q)? {
        if tok == "AND" {
            continue;
        }
        if tok == "OR" || tok == "NOT" {
            return Err(format!("{tok} between terms isn't supported; use @facet:(a OR b) or -term"));
        }
        let (negated, t) = match tok.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => (true, rest.to_string()),
            _ => (false, tok.clone()),
        };
        let term = if t == "*" {
            Term::All
        } else if let Some(rest) = t.strip_prefix('@') {
            let (path, v) = rest.split_once(':').ok_or_else(|| format!("{tok}: expected @facet:value"))?;
            Term::Facet { path: path.split('.').map(str::to_string).collect(), values: values(v)? }
        } else if let Some((key, v)) = t.split_once(':') {
            Term::Tag { key: key.to_string(), values: values(v)? }
        } else {
            Term::Text(unquote(&t).to_lowercase())
        };
        terms.push((negated, term));
    }
    Ok(Query(terms))
}

fn matches_value(have: &str, want: &str) -> bool {
    match want.strip_suffix('*') {
        Some(prefix) => have.to_lowercase().starts_with(&prefix.to_lowercase()),
        None => have.eq_ignore_ascii_case(want),
    }
}

/// The strings at a facet path (arrays are looked through).
fn at(doc: &Value, path: &[String]) -> Vec<String> {
    let Some((head, rest)) = path.split_first() else {
        return match doc {
            Value::String(s) => vec![s.clone()],
            Value::Bool(b) => vec![b.to_string()],
            Value::Number(n) => vec![n.to_string()],
            Value::Array(a) => a.iter().flat_map(|v| at(v, &[])).collect(),
            _ => vec![],
        };
    };
    match doc {
        Value::Object(o) => o.get(head).map(|v| at(v, rest)).unwrap_or_default(),
        Value::Array(a) => a.iter().flat_map(|v| at(v, path)).collect(),
        _ => vec![],
    }
}

impl Query {
    pub fn matches(&self, row: &FindingRow) -> bool {
        if self.0.is_empty() {
            return true;
        }
        let doc = custom(row);
        let tags = tags(row);
        self.0.iter().all(|(negated, term)| {
            let hit = match term {
                Term::All => true,
                Term::Facet { path, values } => {
                    let have = at(&doc, path);
                    values.iter().any(|w| have.iter().any(|h| matches_value(h, w)))
                }
                Term::Tag { key, values } => tags.iter().filter_map(|t| t.split_once(':')).any(|(k, v)| k.eq_ignore_ascii_case(key) && values.iter().any(|w| matches_value(v, w))),
                Term::Text(w) => [row.title.as_deref().unwrap_or_default(), &row.resource_name, &row.resource_uid, &row.check_id].iter().any(|s| s.to_lowercase().contains(w.trim_end_matches('*'))),
            };
            hit != *negated
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn row(check: &str, severity: &str, evaluation: &str, muted: bool) -> FindingRow {
        FindingRow {
            id: "8f0c2a4e-5b6d-4e7f-9a1b-2c3d4e5f6a70".into(),
            provider: "aws".into(),
            account_id: "123456789012".into(),
            check_id: check.into(),
            title: Some("Ensure S3 buckets are not publicly accessible".into()),
            description: Some("Checks whether buckets allow public access.".into()),
            risk: Some("Public buckets can expose data.".into()),
            remediation: Some("Turn on Block Public Access.".into()),
            service: Some("s3".into()),
            compliance: Some(json!({ "CIS-2.0": ["2.1.4"], "SOC2": ["cc_6_1"] })),
            reference_urls: Some(json!([])),
            resource_uid: "arn:aws:s3:::example-shop-assets".into(),
            resource_name: "example-shop-assets".into(),
            resource_type: "AwsS3Bucket".into(),
            region: "us-east-1".into(),
            evaluation: evaluation.into(),
            severity: severity.into(),
            detail: "S3 Bucket example-shop-assets is public.".into(),
            resource_tags: json!(["team:storefront"]),
            first_seen_at: 1_762_300_000_000,
            last_seen_at: 1_762_400_000_000,
            evaluation_changed_at: 1_762_350_000_000,
            muted,
            mute_reason: muted.then(|| "RISK_ACCEPTED".into()),
            mute_description: None,
            mute_expire_at: None,
            muted_by: None,
            muted_at: None,
        }
    }

    #[test]
    fn ids_round_trip() {
        let r = row("s3_bucket_level_public_access_block", "high", "fail", false);
        assert_eq!(row_id(&finding_id(&r)).as_deref(), Some(r.id.as_str()));
        assert_eq!(row_id("ZGVmLTAwMC0wYmd-MDE4NjcyMDJkMzE4MDE5ODY5MGE4ZmQ2MmFlMjg0Y2M="), None, "Datadog's own ids aren't ours");
        assert_eq!(row_id("not base64!"), None);
    }

    #[test]
    fn the_document_has_datadogs_fields() {
        let d = item(&row("s3_bucket_level_public_access_block", "high", "fail", true));
        let c = &d["attributes"]["attributes"]["custom"];
        assert_eq!(c["finding_type"], "misconfiguration");
        assert_eq!(c["status"], "muted");
        assert_eq!(c["workflow"]["mute"]["reason"], "RISK_ACCEPTED");
        assert_eq!(c["compliance"]["evaluation"], "fail");
        assert_eq!(c["rule"]["id"], "s3_bucket_level_public_access_block");
        assert_eq!(c["cloud_resource"]["account"], "123456789012");
        assert!(d["attributes"]["tags"].as_array().unwrap().contains(&json!("framework:CIS-2.0")));
    }

    #[test]
    fn queries() {
        let high = row("s3_bucket_level_public_access_block", "high", "fail", false);
        let low_muted = row("cloudtrail_multi_region_enabled", "low", "fail", true);
        let pass = row("s3_bucket_level_public_access_block", "high", "pass", false);
        let q = |s: &str| parse_query(s).unwrap();
        assert!(q("*").matches(&high) && q("").matches(&low_muted));
        assert!(q("@severity:(critical OR high)").matches(&high));
        assert!(!q("@severity:(critical OR high)").matches(&low_muted));
        assert!(q("@status:open @compliance.evaluation:fail").matches(&high));
        assert!(!q("@status:open").matches(&low_muted));
        assert!(!q("@compliance.evaluation:fail").matches(&pass));
        assert!(q("-@status:muted").matches(&high));
        assert!(q("@rule.id:s3_*").matches(&high));
        assert!(q("framework:CIS-2.0 team:storefront").matches(&high));
        assert!(q("example-shop").matches(&high));
        assert!(q("@cloud_resource.account:123456789012 AND @resource_type:AwsS3Bucket").matches(&high));
        assert!(parse_query("@severity:high OR @status:open").is_err());
        assert!(parse_query("@severity:(high").is_err());
    }
}
