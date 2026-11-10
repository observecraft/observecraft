//! A package's catalog entry: what the catalog, the connector page and the web's mock mode show of it (the server's
//! `GET /api/v1/connectors`, `oc-connector catalog`).

use serde::Serialize;
use utoipa::ToSchema;

use crate::manifest::{duration_ms, FieldKind, Incremental, MetricKind, PushKind, Stream};
use crate::package::{Origin, Package};

/// Seconds between a stream's runs (its `interval`, at least a minute).
pub fn interval_secs(s: &Stream) -> i64 {
    s.interval.as_deref().and_then(|d| duration_ms(d).ok()).map_or(60, |ms| (ms / 1000).max(60))
}

/// How a connector is set up.
#[derive(Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    /// A package's streams, polled by the engine; set up on the generic connector page.
    Package,
    /// Native code with its own setup page (`page`).
    Native,
    /// The vendor sends to ObserveCraft (a log drain, OTLP, a webhook); installed from its tile.
    Push,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorField {
    pub key: String,
    pub label: String,
    /// string | text | url | number | boolean | select | multiselect | list
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    pub secret: bool,
    #[schema(value_type = Value)]
    pub default: Option<serde_json::Value>,
    pub placeholder: Option<String>,
    pub help: Option<String>,
    pub options: Vec<ConnectorOption>,
    pub pattern: Option<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct ConnectorOption {
    pub value: String,
    pub label: String,
}

/// A metric a stream writes.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMetric {
    /// The name (a template for families: `streamnative.{{ record.name }}`).
    pub name: String,
    /// gauge | count
    #[serde(rename = "type")]
    pub kind: String,
    pub unit: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStream {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Seconds between runs.
    pub interval_secs: i64,
    pub default_enabled: bool,
    /// Its records become logs (with the connector's source).
    pub logs: bool,
    pub metrics: Vec<ConnectorMetric>,
    pub events: bool,
    pub costs: bool,
    /// none | new_records | time | token
    pub incremental: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct ConnectorLink {
    pub label: String,
    /// May hold `{{ config.<key> }}` parts.
    pub url: String,
}

/// Where a push-only vendor sends.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorPush {
    /// logs | otlp | webhook
    pub kind: String,
    /// Markdown with `{{ intake }}` and `{{ source }}`.
    pub instructions: String,
    pub webhook: Option<String>,
    /// The URL to paste (`{{ intake }}`, `{{ source }}`, `{{ api_key }}`, `{{ config.x }}` parts).
    pub url: Option<String>,
    /// Headers the vendor must send.
    pub headers: std::collections::BTreeMap<String, String>,
    /// The header carrying the vendor's signature, when the intake checks it.
    pub signature_header: Option<String>,
    /// The secret config field holding the signing secret, saved as a connection of the org.
    pub signature_secret: Option<String>,
    /// Whether the page can create the push on the vendor's side (`POST /api/v1/connectors/{id}/push`).
    pub can_create: bool,
    /// More endpoints (a trace drain beside a log drain).
    pub endpoints: Vec<ConnectorPushEndpoint>,
}

/// Another endpoint of a push package.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorPushEndpoint {
    pub label: String,
    /// logs | otlp | webhook | splunk_hec
    pub kind: String,
    pub url: Option<String>,
    pub headers: std::collections::BTreeMap<String, String>,
    pub instructions: Option<String>,
}

/// A connector: its package's catalog entry, or a native connector's.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorEntry {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    /// Catalog tiles it makes available.
    pub tiles: Vec<String>,
    pub logo: String,
    pub categories: Vec<String>,
    /// Logs' `source`.
    pub source: String,
    pub kind: ConnectorKind,
    /// Shipped with ObserveCraft, or the operator's (`OC_CONNECTORS_DIR`): community, third-party or private.
    pub third_party: bool,
    /// A native connector's own page.
    pub page: Option<String>,
    pub docs_url: Option<String>,
    /// Markdown.
    pub setup: Option<String>,
    pub links: Vec<ConnectorLink>,
    pub config: Vec<ConnectorField>,
    pub streams: Vec<ConnectorStream>,
    pub push: Option<ConnectorPush>,
    /// Titles of the dashboards it brings (listed once connected).
    pub dashboards: Vec<String>,
    /// The log pipeline installed with a connection.
    pub pipeline: Option<String>,
    /// Names of its recommended monitors.
    pub monitors: Vec<String>,
    /// Whether it has a connection test.
    pub testable: bool,
}

fn kind_name(k: FieldKind) -> &'static str {
    match k {
        FieldKind::String => "string",
        FieldKind::Text => "text",
        FieldKind::Url => "url",
        FieldKind::Number => "number",
        FieldKind::Boolean => "boolean",
        FieldKind::Select => "select",
        FieldKind::Multiselect => "multiselect",
        FieldKind::List => "list",
    }
}

/// The names a metric output gives: its name, its `examples` for a templated one, else the values of the table a
/// `{{ tables.<t>[…] }}` name reads (shown raw when none of these apply).
pub fn metric_names(m: &crate::manifest::Manifest, mo: &crate::manifest::MetricOut) -> Vec<String> {
    if !mo.name.contains("{{") {
        return vec![mo.name.clone()];
    }
    if !mo.examples.is_empty() {
        return mo.examples.clone();
    }
    static TABLE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = TABLE.get_or_init(|| regex::Regex::new(r"^\{\{\s*tables\.([a-z_][a-z0-9_]*)\[[^\]]*\]\s*\}\}$").expect("regex"));
    let names: Vec<String> = re
        .captures(mo.name.trim())
        .and_then(|c| m.tables.get(&c[1]))
        .and_then(|t| t.as_object())
        .map(|t| {
            let mut v: Vec<String> = t.values().filter_map(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_string).collect();
            v.sort();
            v.dedup();
            v
        })
        .unwrap_or_default();
    if names.is_empty() {
        vec![mo.name.clone()]
    } else {
        names
    }
}

/// A package's catalog entry.
pub fn entry_of(p: &Package) -> ConnectorEntry {
    let m = &p.manifest;
    let push_kind = |k: PushKind| match k {
        PushKind::Logs => "logs",
        PushKind::Otlp => "otlp",
        PushKind::Webhook => "webhook",
        PushKind::SplunkHec => "splunk_hec",
    };
    ConnectorEntry {
        id: m.id.clone(),
        name: m.name.clone(),
        publisher: m.publisher.clone(),
        vendor: m.vendor.clone(),
        version: m.version.clone(),
        description: m.description.clone(),
        tiles: m.tiles.clone(),
        logo: p.logo().to_string(),
        categories: m.categories.clone(),
        source: p.source().to_string(),
        kind: if m.streams.is_empty() && m.push.is_some() { ConnectorKind::Push } else { ConnectorKind::Package },
        third_party: matches!(p.origin, Origin::ThirdParty { .. }),
        page: None,
        docs_url: m.docs_url.clone(),
        setup: m.setup.clone(),
        links: m.links.iter().map(|l| ConnectorLink { label: l.label.clone(), url: l.url.clone() }).collect(),
        config: m
            .config
            .iter()
            .map(|f| ConnectorField {
                key: f.key.clone(),
                label: f.label.clone(),
                kind: kind_name(f.kind).into(),
                required: f.required,
                secret: f.secret,
                default: f.default.clone(),
                placeholder: f.placeholder.clone(),
                help: f.help.clone(),
                options: f.options.iter().map(|o| ConnectorOption { value: o.value.clone(), label: o.label.clone() }).collect(),
                pattern: f.pattern.clone(),
            })
            .collect(),
        streams: m
            .streams
            .iter()
            .map(|s| ConnectorStream {
                id: s.id.clone(),
                name: s.name.clone(),
                description: s.description.clone(),
                interval_secs: interval_secs(s),
                default_enabled: s.default_enabled,
                logs: s.output.logs.is_some(),
                metrics: s
                    .output
                    .metrics
                    .iter()
                    .flat_map(|mo| {
                        let unit = mo.unit.clone().filter(|u| !u.contains("{{"));
                        metric_names(m, mo).into_iter().map(move |name| ConnectorMetric {
                            name,
                            kind: if mo.kind == MetricKind::Count { "count" } else { "gauge" }.into(),
                            unit: unit.clone(),
                            description: mo.description.clone(),
                            tags: mo.tags.keys().cloned().collect(),
                        })
                    })
                    .collect(),
                events: s.output.events.is_some(),
                costs: s.output.costs.is_some(),
                incremental: match &s.incremental {
                    Incremental::None => "none",
                    Incremental::NewRecords { .. } => "new_records",
                    Incremental::Time(_) => "time",
                    Incremental::Token { .. } => "token",
                }
                .into(),
            })
            .collect(),
        push: m.push.as_ref().map(|p| ConnectorPush {
            kind: push_kind(p.kind).into(),
            instructions: p.instructions.clone(),
            webhook: m.webhook.as_ref().map(|w| w.name.clone().unwrap_or_else(|| m.id.clone())),
            url: p.url.clone(),
            headers: p.headers.clone(),
            signature_header: p.signature.as_ref().map(|s| s.header.clone()),
            signature_secret: p.signature.as_ref().map(|s| s.secret.clone()),
            can_create: p.create.is_some(),
            endpoints: p.endpoints.iter().map(|e| ConnectorPushEndpoint { label: e.label.clone(), kind: push_kind(e.kind).into(), url: e.url.clone(), headers: e.headers.clone(), instructions: e.instructions.clone() }).collect(),
        }),
        dashboards: p.dashboards().iter().filter_map(|d| d["title"].as_str().map(str::to_string)).collect(),
        pipeline: p.pipeline().and_then(|v| v["name"].as_str().map(str::to_string)),
        monitors: p.monitors().iter().filter_map(|v| v["name"].as_str().map(str::to_string)).collect(),
        testable: m.test.is_some(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templated_metric_names_are_listed_by_their_tables_or_examples() {
        let m: crate::manifest::Manifest = serde_yaml_ng::from_str(
            "schema_version: 1\nid: example\nname: Example\npublisher: Example\nvendor: Example\ndescription: Test.\n\
             tables: { metrics: { nOffered: example.offered, nHandled: example.handled, nGone: '' } }\n",
        )
        .unwrap();
        let mo = |name: &str, examples: &[&str]| crate::manifest::MetricOut { name: name.into(), kind: MetricKind::Gauge, value: "1".into(), each: None, tags: Default::default(), tags_from: None, host: None, time: None, filter: None, unit: None, aggregate: None, examples: examples.iter().map(|s| s.to_string()).collect(), description: None };
        assert_eq!(metric_names(&m, &mo("{{ tables.metrics[entry.key] }}", &[])), ["example.handled", "example.offered"]);
        assert_eq!(metric_names(&m, &mo("example.{{ entry.key }}", &["example.a", "example.b"])), ["example.a", "example.b"]);
        assert_eq!(metric_names(&m, &mo("example.{{ entry.key }}", &[])), ["example.{{ entry.key }}"]);
        assert_eq!(metric_names(&m, &mo("example.fixed", &["x"])), ["example.fixed"]);
    }
}
