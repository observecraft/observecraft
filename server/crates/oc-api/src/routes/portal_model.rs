//! Developer Portal → Software Catalog, the pure part: entity definition documents, entities
//! inferred from spans, and the merge of both into the catalog's entities.
//!
//! - **Entity definitions** follow the published entity definition schema, `apiVersion: v3`
//!   (kinds service, datastore, queue, api, system and frontend; `metadata` with name, displayName,
//!   owner, description, links, contacts and tags; `spec` with lifecycle, tier, type, languages,
//!   dependsOn and componentOf). Documents come as JSON or YAML (several per file); they are
//!   validated and normalized here: names are lower case, owners are team handles without `@`,
//!   references are `kind:name`. `integrations`, `datadog` and `extensions` are kept as sent.
//! - **Inferred entities** come from outbound spans: a span with `db.system` (Datadog `db.type`)
//!   names a datastore, one with `messaging.system` a queue, and a `peer.service` that isn't an
//!   instrumented service an external provider. Datastores are named by the database
//!   (`db.instance`/`db.name`), queues by their destination, falling back to `peer.service`, the
//!   server address and the technology.
//! - **Merge.** Telemetry entities and definitions meet by (kind, name); a service also takes its
//!   team, contacts, repository and docs from the service metadata Scorecards edits.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::scorecard_rules::references_service;
use super::scorecards::ServiceContact;

pub const DEFINITION_KINDS: [&str; 6] = ["service", "datastore", "queue", "api", "system", "frontend"];
const LINK_TYPES: [&str; 5] = ["doc", "repo", "runbook", "dashboard", "other"];
const CONTACT_TYPES: [&str; 4] = ["email", "slack", "microsoft-teams", "link"];
pub const MAX_DOCUMENTS: usize = 200;
pub const MAX_DOCUMENT_BYTES: usize = 1_000_000;
const MAX_NAME_CHARS: usize = 100;
const MAX_TEXT_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 2000;
const MAX_ITEMS: usize = 50;
const MAX_URL_CHARS: usize = 2048;

// ================================================================ entity definitions

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
pub struct EntityOwnerRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
pub struct EntityContact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// email | slack | microsoft-teams | link
    pub r#type: String,
    /// Address, channel or URL.
    pub contact: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
pub struct EntityLink {
    pub name: String,
    /// doc | repo | runbook | dashboard | other
    pub r#type: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntityDefinitionMetadata {
    /// Unique per kind; stored in lower case.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Team handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_owners: Vec<EntityOwnerRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `key:value` tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<EntityContact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<EntityLink>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntityDefinitionSpec {
    /// production | experimental | deprecated, or the org's own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// web, grpc, postgres, kafka, openapi…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// `kind:name` references (a bare name is a service).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Systems this entity belongs to (`system:name`; a bare name is a system).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub component_of: Vec<String>,
    /// Systems: their components (`kind:name`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
    /// APIs: the services implementing them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implemented_by: Vec<String>,
}

/// An entity definition document (apiVersion v3).
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntityDefinition {
    /// Always `v3`.
    pub api_version: String,
    /// service | datastore | queue | api | system | frontend
    pub kind: String,
    pub metadata: EntityDefinitionMetadata,
    #[serde(default)]
    pub spec: EntityDefinitionSpec,
    /// Kept as sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub integrations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub datadog: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub extensions: Option<Value>,
}

fn text(value: Option<String>, what: &str, max: usize) -> Result<Option<String>, String> {
    let Some(v) = value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) else { return Ok(None) };
    if v.chars().count() > max {
        return Err(format!("{what} has at most {max} characters"));
    }
    Ok(Some(v))
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

fn entity_name(name: &str, what: &str) -> Result<String, String> {
    let n = name.trim().to_lowercase();
    if n.is_empty() {
        return Err(format!("{what} is required"));
    }
    if n.chars().count() > MAX_NAME_CHARS {
        return Err(format!("{what} has at most {MAX_NAME_CHARS} characters"));
    }
    if !valid_name(&n) {
        return Err(format!("{what} '{n}' may only contain letters, digits, '-', '_', '.' and '/', starting with a letter or digit"));
    }
    Ok(n)
}

pub(crate) fn http_url(url: &str, what: &str) -> Result<String, String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if url.chars().count() > MAX_URL_CHARS || !(lower.starts_with("https://") || lower.starts_with("http://")) || url.chars().any(char::is_whitespace) {
        return Err(format!("{what} must be an http(s) URL"));
    }
    Ok(url.to_string())
}

/// `kind:name`, with `default_kind` for a bare name.
fn reference(r: &str, default_kind: &str, what: &str) -> Result<String, String> {
    let r = r.trim().to_lowercase();
    let (kind, name) = match r.split_once(':') {
        Some((k, n)) => (k.to_string(), n.to_string()),
        None => (default_kind.to_string(), r.clone()),
    };
    if !DEFINITION_KINDS.contains(&kind.as_str()) {
        return Err(format!("{what} '{r}' has an unknown kind; use one of {}", DEFINITION_KINDS.join(", ")));
    }
    Ok(format!("{kind}:{}", entity_name(&name, what)?))
}

fn references(list: Vec<String>, default_kind: &str, what: &str) -> Result<Vec<String>, String> {
    if list.len() > MAX_ITEMS {
        return Err(format!("{what} has at most {MAX_ITEMS} entries"));
    }
    let mut out: Vec<String> = list.iter().map(|r| reference(r, default_kind, what)).collect::<Result<_, _>>()?;
    out.dedup();
    Ok(out)
}

fn strings(list: Vec<String>, what: &str, max: usize) -> Result<Vec<String>, String> {
    if list.len() > MAX_ITEMS {
        return Err(format!("{what} has at most {MAX_ITEMS} entries"));
    }
    let mut out = Vec::new();
    for s in list {
        if let Some(v) = text(Some(s), what, max)? {
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    Ok(out)
}

/// Checks and normalizes a definition.
pub fn validate_definition(def: EntityDefinition) -> Result<EntityDefinition, String> {
    if def.api_version.trim() != "v3" {
        return Err(format!("apiVersion '{}' isn't supported; use v3", def.api_version.trim()));
    }
    let kind = def.kind.trim().to_lowercase();
    if !DEFINITION_KINDS.contains(&kind.as_str()) {
        return Err(format!("kind '{kind}' isn't supported; use one of {}", DEFINITION_KINDS.join(", ")));
    }
    let m = def.metadata;
    let name = entity_name(&m.name, "metadata.name")?;
    let owner = text(m.owner, "metadata.owner", MAX_TEXT_CHARS)?.map(|o| o.trim_start_matches('@').to_lowercase()).filter(|o| !o.is_empty());
    let tags = strings(m.tags, "metadata.tags", MAX_TEXT_CHARS)?;
    if let Some(t) = tags.iter().find(|t| t.chars().any(char::is_whitespace)) {
        return Err(format!("Tag '{t}' contains whitespace"));
    }
    if m.links.len() > MAX_ITEMS || m.contacts.len() > MAX_ITEMS || m.additional_owners.len() > MAX_ITEMS {
        return Err(format!("metadata lists at most {MAX_ITEMS} links, contacts and owners"));
    }
    let mut links = Vec::new();
    for l in m.links {
        let r#type = l.r#type.trim().to_lowercase();
        if !LINK_TYPES.contains(&r#type.as_str()) {
            return Err(format!("Link type '{type}' isn't supported; use one of {}", LINK_TYPES.join(", ")));
        }
        let url = http_url(&l.url, "A link URL")?;
        let name = text(Some(l.name), "A link name", MAX_TEXT_CHARS)?.unwrap_or_else(|| url.clone());
        links.push(EntityLink { name, r#type, url, provider: text(l.provider, "A link provider", MAX_TEXT_CHARS)? });
    }
    let mut contacts = Vec::new();
    for c in m.contacts {
        let r#type = c.r#type.trim().to_lowercase();
        let contact = c.contact.trim().to_string();
        let ok = match r#type.as_str() {
            "email" => contact.contains('@') && !contact.contains(char::is_whitespace) && contact.len() <= 320,
            "link" | "microsoft-teams" => http_url(&contact, "A contact").is_ok(),
            "slack" => !contact.is_empty() && contact.chars().count() <= MAX_TEXT_CHARS,
            _ => return Err(format!("Contact type '{type}' isn't supported; use one of {}", CONTACT_TYPES.join(", "))),
        };
        if !ok {
            return Err(format!("'{contact}' isn't a valid {type} contact"));
        }
        contacts.push(EntityContact { name: text(c.name, "A contact name", MAX_TEXT_CHARS)?, r#type, contact });
    }
    let additional_owners = m
        .additional_owners
        .into_iter()
        .filter_map(|o| {
            let name = o.name.trim().trim_start_matches('@').to_lowercase();
            (!name.is_empty()).then(|| EntityOwnerRef { name, r#type: o.r#type.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) })
        })
        .collect();
    let s = def.spec;
    let spec = EntityDefinitionSpec {
        lifecycle: text(s.lifecycle, "spec.lifecycle", MAX_TEXT_CHARS)?.map(|l| l.to_lowercase()),
        tier: text(s.tier, "spec.tier", MAX_TEXT_CHARS)?,
        r#type: text(s.r#type, "spec.type", MAX_TEXT_CHARS)?.map(|t| t.to_lowercase()),
        languages: strings(s.languages, "spec.languages", MAX_TEXT_CHARS)?.into_iter().map(|l| l.to_lowercase()).collect(),
        depends_on: references(s.depends_on, "service", "spec.dependsOn")?,
        component_of: references(s.component_of, "system", "spec.componentOf")?,
        components: references(s.components, "service", "spec.components")?,
        implemented_by: references(s.implemented_by, "service", "spec.implementedBy")?,
    };
    Ok(EntityDefinition {
        api_version: "v3".into(),
        kind,
        metadata: EntityDefinitionMetadata {
            name,
            display_name: text(m.display_name, "metadata.displayName", MAX_TEXT_CHARS)?,
            namespace: text(m.namespace, "metadata.namespace", MAX_TEXT_CHARS)?,
            owner,
            additional_owners,
            description: text(m.description, "metadata.description", MAX_DESCRIPTION_CHARS)?,
            tags,
            contacts,
            links,
        },
        spec,
        integrations: def.integrations,
        datadog: def.datadog,
        extensions: def.extensions,
    })
}

/// A document as parsed JSON (YAML documents are converted) → a checked definition.
pub fn definition_from_value(mut v: Value) -> Result<EntityDefinition, String> {
    let Some(obj) = v.as_object_mut() else { return Err("A document must be an object with apiVersion, kind and metadata".into()) };
    // YAML writes `tier: 1`: tiers are strings.
    if let Some(tier) = obj.get_mut("spec").and_then(|s| s.get_mut("tier")) {
        if let Value::Number(n) = tier {
            *tier = Value::String(n.to_string());
        }
    }
    for key in ["apiVersion", "kind", "metadata"] {
        if !obj.contains_key(key) {
            return Err(format!("{key} is required"));
        }
    }
    let def: EntityDefinition = serde_json::from_value(v).map_err(|e| format!("Not an entity definition: {e}"))?;
    validate_definition(def)
}

/// Every definition in a JSON (an object or an array) or YAML (one or more `---` documents) text.
pub fn parse_documents(content: &str) -> Result<Vec<EntityDefinition>, String> {
    if content.len() > MAX_DOCUMENT_BYTES {
        return Err(format!("Import at most {} KB at a time", MAX_DOCUMENT_BYTES / 1000));
    }
    let trimmed = content.trim_start();
    let values: Vec<Value> = if trimmed.starts_with('{') || trimmed.starts_with('[') {
        match serde_json::from_str::<Value>(trimmed).map_err(|e| format!("Invalid JSON: {e}"))? {
            Value::Array(items) => items,
            v => vec![v],
        }
    } else {
        let mut out = Vec::new();
        for doc in serde_yaml_ng::Deserializer::from_str(content) {
            let v = serde_yaml_ng::Value::deserialize(doc).map_err(|e| format!("Invalid YAML: {e}"))?;
            if v.is_null() {
                continue;
            }
            out.push(serde_json::to_value(v).map_err(|e| format!("Invalid YAML: {e}"))?);
            if out.len() > MAX_DOCUMENTS {
                break;
            }
        }
        out
    };
    if values.is_empty() {
        return Err("No entity definition found".into());
    }
    if values.len() > MAX_DOCUMENTS {
        return Err(format!("Import at most {MAX_DOCUMENTS} entities at a time"));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (i, v) in values.into_iter().enumerate() {
        let def = definition_from_value(v).map_err(|e| format!("Document {}: {e}", i + 1))?;
        if !seen.insert((def.kind.clone(), def.metadata.name.clone())) {
            return Err(format!("Document {}: {}:{} is defined twice", i + 1, def.kind, def.metadata.name));
        }
        out.push(def);
    }
    Ok(out)
}

/// The repository and documentation links and contacts of a definition as service metadata.
pub fn metadata_of(def: &EntityDefinition) -> (String, Vec<ServiceContact>, String, String) {
    let link = |t: &str| def.metadata.links.iter().find(|l| l.r#type == t).map(|l| l.url.clone()).unwrap_or_default();
    let contacts = def
        .metadata
        .contacts
        .iter()
        .map(|c| ServiceContact { r#type: if c.r#type == "microsoft-teams" { "link".into() } else { c.r#type.clone() }, name: c.name.clone().unwrap_or_default(), value: c.contact.clone() })
        .collect();
    (def.metadata.owner.clone().unwrap_or_default(), contacts, link("repo"), link("doc"))
}

// ================================================================ inferred entities

/// Outbound (and messaging consumer) spans of one service, grouped by their peer attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeerCall {
    pub service: String,
    pub db_system: String,
    /// `db.instance` (Datadog), else `db.namespace` / `db.name`.
    pub db_name: String,
    pub server_address: String,
    pub peer_service: String,
    pub messaging_system: String,
    pub destination: String,
    pub hits: u64,
    pub errors: u64,
    pub p95_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InferredKind {
    Datastore,
    Queue,
    External,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InferredEntity {
    pub kind: InferredKind,
    pub name: String,
    /// `db.system` / `messaging.system`.
    pub technology: Option<String>,
    pub callers: BTreeSet<String>,
    pub requests: u64,
    pub errors: u64,
    pub p95_ms: f64,
}

fn first_named<'a>(candidates: &[&'a str]) -> Option<&'a str> {
    candidates.iter().map(|c| c.trim()).find(|c| !c.is_empty() && !c.chars().all(|ch| ch.is_ascii_digit()))
}

/// What a call reaches: (kind, name, technology), or None for calls between instrumented services.
pub fn infer_peer(call: &PeerCall, instrumented: &HashSet<String>) -> Option<(InferredKind, String, Option<String>)> {
    let tech = |t: &str| Some(t.trim().to_lowercase());
    if !call.db_system.trim().is_empty() {
        let name = first_named(&[&call.db_name, &call.peer_service, &call.server_address, &call.db_system])?;
        return Some((InferredKind::Datastore, name.to_lowercase(), tech(&call.db_system)));
    }
    if !call.messaging_system.trim().is_empty() {
        let name = first_named(&[&call.destination, &call.peer_service, &call.server_address, &call.messaging_system])?;
        return Some((InferredKind::Queue, name.to_lowercase(), tech(&call.messaging_system)));
    }
    let peer = call.peer_service.trim();
    if !peer.is_empty() && !instrumented.contains(peer) {
        return Some((InferredKind::External, peer.to_lowercase(), None));
    }
    None
}

/// Datastores, queues and external providers with the services calling them.
pub fn infer_entities(calls: &[PeerCall], instrumented: &HashSet<String>) -> Vec<InferredEntity> {
    let mut by_key: BTreeMap<(InferredKind, String), InferredEntity> = BTreeMap::new();
    for c in calls {
        let Some((kind, name, technology)) = infer_peer(c, instrumented) else { continue };
        let e = by_key.entry((kind, name.clone())).or_insert_with(|| InferredEntity { kind, name, technology, callers: BTreeSet::new(), requests: 0, errors: 0, p95_ms: 0.0 });
        if !c.service.is_empty() {
            e.callers.insert(c.service.clone());
        }
        e.requests += c.hits;
        e.errors += c.errors;
        if c.p95_ms.is_finite() {
            e.p95_ms = e.p95_ms.max(c.p95_ms);
        }
    }
    by_key.into_values().collect()
}

// ================================================================ catalog entities

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortalEntityKind {
    #[default]
    Service,
    Datastore,
    Queue,
    Api,
    System,
    Frontend,
    Endpoint,
    ExternalProvider,
    Repository,
}

impl PortalEntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Datastore => "datastore",
            Self::Queue => "queue",
            Self::Api => "api",
            Self::System => "system",
            Self::Frontend => "frontend",
            Self::Endpoint => "endpoint",
            Self::ExternalProvider => "external_provider",
            Self::Repository => "repository",
        }
    }

    pub fn of_definition(kind: &str) -> Option<Self> {
        Some(match kind {
            "service" => Self::Service,
            "datastore" => Self::Datastore,
            "queue" => Self::Queue,
            "api" => Self::Api,
            "system" => Self::System,
            "frontend" => Self::Frontend,
            _ => return None,
        })
    }
}

/// One entity of the catalog, with what telemetry and definitions say about it.
#[derive(Serialize, ToSchema, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalEntity {
    /// `kind:name` (endpoints: `endpoint:service/resource`).
    pub id: String,
    pub kind: PortalEntityKind,
    pub name: String,
    pub display_name: Option<String>,
    /// Where it was seen: apm, spans, logs, metrics, ci, metadata, definition.
    pub sources: Vec<String>,
    /// Known only from other services' spans or from log and metric tags: not instrumented, not defined.
    pub inferred: bool,
    /// Owning team handle; empty when none.
    pub owner: String,
    pub description: String,
    pub lifecycle: Option<String>,
    pub tier: Option<String>,
    /// web, worker, or the definition's spec.type.
    pub entity_type: Option<String>,
    pub languages: Vec<String>,
    /// postgresql, redis, kafka, … for datastores and queues.
    pub technology: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<EntityLink>,
    pub contacts: Vec<EntityContact>,
    pub repo_url: String,
    pub docs_url: String,
    /// Systems it belongs to (names).
    pub systems: Vec<String>,
    /// `kind:name` references.
    pub depends_on: Vec<String>,
    /// Endpoints: their service.
    pub service: Option<String>,
    /// Datastores, queues and external providers: the services calling them; systems: their
    /// components; repositories: the services built from them.
    pub related: Vec<String>,
    pub requests: Option<u64>,
    pub req_per_sec: Option<f64>,
    pub error_rate: Option<f64>,
    pub p95_ms: Option<f64>,
    pub version: Option<String>,
    /// When the current version was first seen (epoch ms).
    pub last_deploy: Option<i64>,
    pub slos: u32,
    pub monitors: u32,
    pub monitors_alerting: u32,
    /// Incidents declared in the range that list the service.
    pub incidents: u32,
    pub open_incidents: u32,
    /// Distinct error types on the service's spans in the range.
    pub error_issues: Option<u64>,
    /// CI pipeline runs of the repository in the past 30 days.
    pub ci_runs: Option<u64>,
    pub ci_success_rate: Option<f64>,
    pub last_ci_run: Option<i64>,
    /// Id of the stored definition, when there is one.
    pub definition_id: Option<String>,
}

/// An instrumented APM service.
#[derive(Clone, Debug, Default)]
pub struct ApmService {
    pub name: String,
    pub web: bool,
    pub language: Option<String>,
    pub version: Option<String>,
    pub requests: u64,
    pub req_per_sec: f64,
    pub error_rate: f64,
    pub p95_ms: f64,
    pub last_deploy: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct EndpointStat {
    pub service: String,
    pub resource: String,
    pub requests: u64,
    pub req_per_sec: f64,
    pub error_rate: f64,
    pub p95_ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ServiceMeta {
    pub team: String,
    pub contacts: Vec<ServiceContact>,
    pub repo_url: String,
    pub docs_url: String,
}

#[derive(Clone, Debug)]
pub struct StoredDefinition {
    pub id: String,
    pub definition: EntityDefinition,
}

/// An SLO or monitor as the reliability columns see it: the texts that may name a service.
#[derive(Clone, Debug, Default)]
pub struct ReliabilityRef {
    pub name: String,
    pub texts: Vec<String>,
    /// SLOs: names of their monitors.
    pub monitors: Vec<String>,
    pub alerting: bool,
}

#[derive(Clone, Debug, Default)]
pub struct IncidentRef {
    pub services: Vec<String>,
    pub open: bool,
    pub in_range: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CiRepo {
    pub name: String,
    pub url: String,
    pub runs: u64,
    pub failures: u64,
    pub last_run: i64,
}

#[derive(Default)]
pub struct CatalogInputs {
    pub services: Vec<ApmService>,
    /// Services named on logs (count) and metrics, instrumented or not.
    pub log_services: BTreeMap<String, u64>,
    pub metric_services: BTreeSet<String>,
    pub peers: Vec<InferredEntity>,
    pub endpoints: Vec<EndpointStat>,
    pub error_types: HashMap<String, u64>,
    pub definitions: Vec<StoredDefinition>,
    pub metadata: BTreeMap<String, ServiceMeta>,
    pub slos: Vec<ReliabilityRef>,
    pub monitors: Vec<ReliabilityRef>,
    pub incidents: Vec<IncidentRef>,
    pub repos: Vec<CiRepo>,
}

/// CI runs, success rate and last run of a repository.
type CiResults = (Option<u64>, Option<f64>, Option<i64>);

/// `https://git.example.com/example-org/order-service.git` → `example-org/order-service`.
pub fn repo_path(url_or_name: &str) -> String {
    let s = url_or_name.trim().trim_end_matches('/');
    let s = s.split_once("://").map(|(_, rest)| rest.split_once('/').map(|(_, path)| path).unwrap_or("")).unwrap_or(s);
    // git@host:owner/repo
    let s = if !url_or_name.contains("://") { s.split_once(':').map(|(_, p)| p).unwrap_or(s) } else { s };
    s.trim_end_matches(".git").trim_matches('/').to_lowercase()
}

fn entity_id(kind: PortalEntityKind, name: &str) -> String {
    format!("{}:{name}", kind.as_str())
}

fn push_source(e: &mut PortalEntity, source: &str) {
    if !e.sources.iter().any(|s| s == source) {
        e.sources.push(source.to_string());
    }
}

fn contacts_of(meta: &ServiceMeta) -> Vec<EntityContact> {
    meta.contacts.iter().map(|c| EntityContact { name: (!c.name.is_empty()).then(|| c.name.clone()), r#type: c.r#type.clone(), contact: c.value.clone() }).collect()
}

/// Overlays a definition on an entity.
fn apply_definition(e: &mut PortalEntity, d: &StoredDefinition) {
    let def = &d.definition;
    push_source(e, "definition");
    e.definition_id = Some(d.id.clone());
    e.display_name = def.metadata.display_name.clone();
    e.owner = def.metadata.owner.clone().unwrap_or_default();
    e.description = def.metadata.description.clone().unwrap_or_default();
    e.lifecycle = def.spec.lifecycle.clone();
    e.tier = def.spec.tier.clone();
    if def.spec.r#type.is_some() {
        e.entity_type = def.spec.r#type.clone();
    }
    if !def.spec.languages.is_empty() {
        e.languages = def.spec.languages.clone();
    }
    e.tags = def.metadata.tags.clone();
    e.links = def.metadata.links.clone();
    e.contacts = def.metadata.contacts.clone();
    let link = |t: &str| def.metadata.links.iter().find(|l| l.r#type == t).map(|l| l.url.clone()).unwrap_or_default();
    e.repo_url = link("repo");
    e.docs_url = link("doc");
    e.systems = def.spec.component_of.iter().filter_map(|r| r.strip_prefix("system:")).map(str::to_string).collect();
    e.depends_on = def.spec.depends_on.clone();
    if def.kind == "system" {
        e.related = def.spec.components.clone();
    }
    if def.kind == "api" && !def.spec.implemented_by.is_empty() {
        e.related = def.spec.implemented_by.clone();
    }
}

fn blank(kind: PortalEntityKind, name: &str) -> PortalEntity {
    PortalEntity { id: entity_id(kind, name), kind, name: name.to_string(), ..Default::default() }
}

/// Every entity of the catalog, merged from telemetry, service metadata and definitions; sorted by kind and name.
pub fn assemble(inputs: &CatalogInputs) -> Vec<PortalEntity> {
    let mut entities: BTreeMap<(PortalEntityKind, String), PortalEntity> = BTreeMap::new();
    let instrumented: HashSet<&str> = inputs.services.iter().map(|s| s.name.as_str()).collect();

    // Services: instrumented, then only named on logs and metrics (inferred).
    for s in &inputs.services {
        let e = entities.entry((PortalEntityKind::Service, s.name.clone())).or_insert_with(|| blank(PortalEntityKind::Service, &s.name));
        push_source(e, "apm");
        e.entity_type = Some(if s.web { "web" } else { "worker" }.into());
        e.languages = s.language.iter().cloned().collect();
        e.version = s.version.clone();
        e.last_deploy = s.last_deploy;
        e.requests = Some(s.requests);
        e.req_per_sec = Some(s.req_per_sec);
        e.error_rate = Some(s.error_rate);
        e.p95_ms = Some(s.p95_ms);
    }
    for (name, source) in inputs.log_services.keys().map(|n| (n, "logs")).chain(inputs.metric_services.iter().map(|n| (n, "metrics"))) {
        if name.is_empty() {
            continue;
        }
        let e = entities.entry((PortalEntityKind::Service, name.clone())).or_insert_with(|| blank(PortalEntityKind::Service, name));
        push_source(e, source);
    }
    for name in inputs.metadata.keys() {
        let e = entities.entry((PortalEntityKind::Service, name.clone())).or_insert_with(|| blank(PortalEntityKind::Service, name));
        push_source(e, "metadata");
    }

    // Datastores, queues and external providers from spans.
    for p in &inputs.peers {
        let kind = match p.kind {
            InferredKind::Datastore => PortalEntityKind::Datastore,
            InferredKind::Queue => PortalEntityKind::Queue,
            InferredKind::External => PortalEntityKind::ExternalProvider,
        };
        let e = entities.entry((kind, p.name.clone())).or_insert_with(|| blank(kind, &p.name));
        push_source(e, "spans");
        e.technology = p.technology.clone();
        e.entity_type = p.technology.clone();
        e.related = p.callers.iter().cloned().collect();
        e.requests = Some(p.requests);
        e.error_rate = Some(if p.requests == 0 { 0.0 } else { p.errors as f64 / p.requests as f64 });
        e.p95_ms = Some(p.p95_ms);
    }

    // Definitions overlay (and add) entities by (kind, name).
    for d in &inputs.definitions {
        let Some(kind) = PortalEntityKind::of_definition(&d.definition.kind) else { continue };
        let e = entities.entry((kind, d.definition.metadata.name.clone())).or_insert_with(|| blank(kind, &d.definition.metadata.name));
        apply_definition(e, d);
    }
    // Systems named by componentOf without a definition of their own.
    let mut systems: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ((kind, name), e) in &entities {
        for s in &e.systems {
            systems.entry(s.clone()).or_default().push(format!("{}:{name}", kind.as_str()));
        }
    }
    for (system, members) in systems {
        let e = entities.entry((PortalEntityKind::System, system.clone())).or_insert_with(|| blank(PortalEntityKind::System, &system));
        if e.definition_id.is_none() {
            push_source(e, "definition");
        }
        for m in members {
            if !e.related.contains(&m) {
                e.related.push(m);
            }
        }
        e.related.sort();
    }

    // Service metadata (shared with Scorecards) is the source of a service's team, contacts, repository and docs.
    for ((kind, name), e) in entities.iter_mut() {
        if *kind != PortalEntityKind::Service {
            continue;
        }
        if let Some(meta) = inputs.metadata.get(name) {
            e.owner = meta.team.clone();
            e.contacts = contacts_of(meta);
            e.repo_url = meta.repo_url.clone();
            e.docs_url = meta.docs_url.clone();
        }
    }

    // Reliability of services.
    for ((kind, name), e) in entities.iter_mut() {
        if *kind != PortalEntityKind::Service {
            continue;
        }
        let mentions = |t: &String| references_service(t, name);
        let monitors: Vec<&ReliabilityRef> = inputs.monitors.iter().filter(|m| m.texts.iter().any(mentions)).collect();
        e.monitors = monitors.len() as u32;
        e.monitors_alerting = monitors.iter().filter(|m| m.alerting).count() as u32;
        e.slos = inputs.slos.iter().filter(|s| s.texts.iter().any(mentions) || s.monitors.iter().any(|m| monitors.iter().any(|x| &x.name == m))).count() as u32;
        let listed = || inputs.incidents.iter().filter(|i| i.services.iter().any(|s| s == name));
        e.incidents = listed().filter(|i| i.in_range).count() as u32;
        e.open_incidents = listed().filter(|i| i.open).count() as u32;
        if instrumented.contains(name.as_str()) {
            e.error_issues = Some(inputs.error_types.get(name).copied().unwrap_or(0));
        }
    }

    // Endpoints, owned by their service's team.
    for ep in &inputs.endpoints {
        let id_name = format!("{}/{}", ep.service, ep.resource);
        let owner = entities.get(&(PortalEntityKind::Service, ep.service.clone())).map(|s| s.owner.clone()).unwrap_or_default();
        let mut e = blank(PortalEntityKind::Endpoint, &ep.resource);
        e.id = entity_id(PortalEntityKind::Endpoint, &id_name);
        e.sources = vec!["apm".into()];
        e.service = Some(ep.service.clone());
        e.owner = owner;
        e.requests = Some(ep.requests);
        e.req_per_sec = Some(ep.req_per_sec);
        e.error_rate = Some(ep.error_rate);
        e.p95_ms = Some(ep.p95_ms);
        // Its monitors: those naming its service and its route (`resource_name:"GET /orders"`, `/orders`).
        let route = match ep.resource.split_once(' ') {
            Some((_, r)) if r.len() > 1 => r.to_lowercase(),
            _ => ep.resource.to_lowercase(),
        };
        let mentions = |t: &String| references_service(t, &ep.service) && t.to_lowercase().contains(&route);
        let monitors: Vec<&ReliabilityRef> = inputs.monitors.iter().filter(|m| m.texts.iter().any(mentions)).collect();
        e.monitors = monitors.len() as u32;
        e.monitors_alerting = monitors.iter().filter(|m| m.alerting).count() as u32;
        entities.insert((PortalEntityKind::Endpoint, id_name), e);
    }

    // Repositories: CI repositories and the repository links of entities.
    let mut repos: BTreeMap<String, PortalEntity> = BTreeMap::new();
    let mut ci: BTreeMap<String, (u64, u64, i64)> = BTreeMap::new();
    for r in &inputs.repos {
        let path = if r.url.is_empty() { repo_path(&r.name) } else { repo_path(&r.url) };
        if path.is_empty() {
            continue;
        }
        let e = repos.entry(path.clone()).or_insert_with(|| blank(PortalEntityKind::Repository, &path));
        push_source(e, "ci");
        if !r.url.is_empty() {
            e.repo_url = r.url.clone();
        }
        let c = ci.entry(path).or_default();
        c.0 += r.runs;
        c.1 += r.failures;
        c.2 = c.2.max(r.last_run);
    }
    for (path, (runs, failures, last)) in &ci {
        if let Some(e) = repos.get_mut(path) {
            e.ci_runs = Some(*runs);
            e.ci_success_rate = (*runs > 0).then(|| runs.saturating_sub(*failures) as f64 / *runs as f64);
            e.last_ci_run = Some(*last);
        }
    }
    for ((kind, name), e) in entities.iter() {
        if e.repo_url.is_empty() || *kind == PortalEntityKind::Endpoint {
            continue;
        }
        let path = repo_path(&e.repo_url);
        if path.is_empty() {
            continue;
        }
        let r = repos.entry(path.clone()).or_insert_with(|| blank(PortalEntityKind::Repository, &path));
        push_source(r, if e.definition_id.is_some() && *kind != PortalEntityKind::Service { "definition" } else { "metadata" });
        if r.repo_url.is_empty() {
            r.repo_url = e.repo_url.clone();
        }
        let member = format!("{}:{name}", kind.as_str());
        if !r.related.contains(&member) {
            r.related.push(member);
        }
        if r.owner.is_empty() {
            r.owner = e.owner.clone();
        }
    }
    // Services take their repository's CI results.
    let ci_by_path: HashMap<String, CiResults> = repos.iter().map(|(p, r)| (p.clone(), (r.ci_runs, r.ci_success_rate, r.last_ci_run))).collect();
    for ((kind, _), e) in entities.iter_mut() {
        if *kind == PortalEntityKind::Service && !e.repo_url.is_empty() {
            if let Some((runs, rate, last)) = ci_by_path.get(&repo_path(&e.repo_url)) {
                e.ci_runs = *runs;
                e.ci_success_rate = *rate;
                e.last_ci_run = *last;
            }
        }
    }
    for (path, r) in repos {
        entities.insert((PortalEntityKind::Repository, path), r);
    }

    for e in entities.values_mut() {
        e.inferred = !e.sources.iter().any(|s| matches!(s.as_str(), "apm" | "definition" | "metadata" | "ci"));
        e.sources.sort();
    }
    entities.into_values().collect()
}

/// Entities owned per team handle.
pub fn entities_per_team(entities: &[PortalEntity]) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    for e in entities.iter().filter(|e| !e.owner.is_empty() && e.kind != PortalEntityKind::Endpoint) {
        *out.entry(e.owner.clone()).or_insert(0) += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(service: &str, attrs: &[(&str, &str)], hits: u64) -> PeerCall {
        let mut c = PeerCall { service: service.into(), hits, errors: hits / 10, p95_ms: hits as f64, ..Default::default() };
        for (k, v) in attrs {
            let v = v.to_string();
            match *k {
                "db.system" => c.db_system = v,
                "db.name" => c.db_name = v,
                "server.address" => c.server_address = v,
                "peer.service" => c.peer_service = v,
                "messaging.system" => c.messaging_system = v,
                "destination" => c.destination = v,
                _ => unreachable!(),
            }
        }
        c
    }

    fn instrumented() -> HashSet<String> {
        ["order-service", "billing", "storefront-api"].into_iter().map(String::from).collect()
    }

    #[test]
    fn datastores_are_named_by_database_then_peer_then_address() {
        let i = instrumented();
        let pg = call("order-service", &[("db.system", "postgresql"), ("db.name", "orders-db"), ("peer.service", "postgres")], 10);
        assert_eq!(infer_peer(&pg, &i), Some((InferredKind::Datastore, "orders-db".into(), Some("postgresql".into()))));
        // Redis database numbers aren't names: the peer service names it.
        let redis = call("storefront-api", &[("db.system", "redis"), ("db.name", "0"), ("peer.service", "cart-cache")], 5);
        assert_eq!(infer_peer(&redis, &i).unwrap().1, "cart-cache");
        let by_address = call("billing", &[("db.system", "mysql"), ("server.address", "billing-db.internal")], 1);
        assert_eq!(infer_peer(&by_address, &i).unwrap().1, "billing-db.internal");
        let bare = call("billing", &[("db.system", "MongoDB")], 1);
        assert_eq!(infer_peer(&bare, &i), Some((InferredKind::Datastore, "mongodb".into(), Some("mongodb".into()))));
    }

    #[test]
    fn queues_are_named_by_destination_and_externals_by_peer() {
        let i = instrumented();
        let kafka = call("order-service", &[("messaging.system", "kafka"), ("destination", "order-events"), ("peer.service", "kafka")], 3);
        assert_eq!(infer_peer(&kafka, &i), Some((InferredKind::Queue, "order-events".into(), Some("kafka".into()))));
        let rabbit = call("billing", &[("messaging.system", "rabbitmq"), ("peer.service", "invoices-queue")], 1);
        assert_eq!(infer_peer(&rabbit, &i).unwrap().1, "invoices-queue");
        let payments = call("billing", &[("peer.service", "payments-provider")], 2);
        assert_eq!(infer_peer(&payments, &i), Some((InferredKind::External, "payments-provider".into(), None)));
        // A call to an instrumented service is an APM dependency, not an entity of its own.
        assert_eq!(infer_peer(&call("storefront-api", &[("peer.service", "order-service")], 2), &i), None);
        assert_eq!(infer_peer(&call("storefront-api", &[], 2), &i), None);
    }

    #[test]
    fn inferred_entities_collect_their_callers() {
        let calls = vec![
            call("order-service", &[("db.system", "postgresql"), ("db.name", "orders-db")], 10),
            call("billing", &[("db.system", "postgresql"), ("db.name", "orders-db")], 30),
            call("order-service", &[("messaging.system", "kafka"), ("destination", "order-events")], 4),
            call("notification-worker", &[("messaging.system", "kafka"), ("destination", "order-events")], 4),
        ];
        let out = infer_entities(&calls, &instrumented());
        assert_eq!(out.len(), 2);
        let db = &out[0];
        assert_eq!((db.kind, db.name.as_str(), db.requests, db.errors, db.p95_ms), (InferredKind::Datastore, "orders-db", 40, 4, 30.0));
        assert_eq!(db.callers.iter().cloned().collect::<Vec<_>>(), vec!["billing", "order-service"]);
        assert_eq!(out[1].callers.len(), 2);
    }

    fn yaml_doc() -> &'static str {
        "apiVersion: v3\nkind: service\nmetadata:\n  name: Order-Service\n  displayName: Order Service\n  owner: '@checkout'\n  description: Takes orders\n  tags: [tier:1, 'env:prod']\n  links:\n    - name: Source\n      type: repo\n      url: https://git.example.com/example-org/order-service\n    - name: Runbook\n      type: runbook\n      url: https://docs.example.com/runbooks/order-service\n  contacts:\n    - name: On-call\n      type: email\n      contact: checkout-oncall@example.com\nspec:\n  lifecycle: Production\n  tier: 1\n  type: web\n  languages: [go]\n  dependsOn: [service:billing, datastore:orders-db]\n  componentOf: [checkout]\nintegrations:\n  pagerduty:\n    serviceURL: https://example.pagerduty.com/service-directory/PABC123\n---\napiVersion: v3\nkind: datastore\nmetadata:\n  name: orders-db\n  owner: checkout\nspec:\n  type: postgres\n"
    }

    #[test]
    fn yaml_documents_are_parsed_and_normalized() {
        let defs = parse_documents(yaml_doc()).unwrap();
        assert_eq!(defs.len(), 2);
        let svc = &defs[0];
        assert_eq!((svc.kind.as_str(), svc.metadata.name.as_str(), svc.metadata.owner.as_deref()), ("service", "order-service", Some("checkout")));
        assert_eq!(svc.spec.tier.as_deref(), Some("1"));
        assert_eq!(svc.spec.lifecycle.as_deref(), Some("production"));
        assert_eq!(svc.spec.depends_on, vec!["service:billing", "datastore:orders-db"]);
        assert_eq!(svc.spec.component_of, vec!["system:checkout"]);
        assert_eq!(svc.metadata.tags, vec!["tier:1", "env:prod"]);
        assert!(svc.integrations.is_some(), "integrations are kept");
        assert_eq!(defs[1].spec.r#type.as_deref(), Some("postgres"));
        let (team, contacts, repo, docs) = metadata_of(svc);
        assert_eq!((team.as_str(), contacts.len(), repo.as_str(), docs.as_str()), ("checkout", 1, "https://git.example.com/example-org/order-service", ""));
    }

    #[test]
    fn json_documents_and_arrays_are_accepted() {
        let one = r#"{"apiVersion":"v3","kind":"api","metadata":{"name":"orders-api"},"spec":{"implementedBy":["order-service"]}}"#;
        assert_eq!(parse_documents(one).unwrap()[0].spec.implemented_by, vec!["service:order-service"]);
        let many = format!("[{one}, {}]", one.replace("orders-api", "billing-api"));
        assert_eq!(parse_documents(&many).unwrap().len(), 2);
    }

    #[test]
    fn invalid_documents_are_rejected_with_their_position() {
        let err = |s: &str| parse_documents(s).unwrap_err();
        assert!(err("apiVersion: v2.2\nkind: service\nmetadata:\n  name: x\n").contains("apiVersion 'v2.2'"));
        assert!(err("apiVersion: v3\nkind: lambda\nmetadata:\n  name: x\n").contains("kind 'lambda'"));
        assert!(err("apiVersion: v3\nkind: service\n").contains("metadata is required"));
        assert!(err("apiVersion: v3\nkind: service\nmetadata:\n  name: 'has space'\n").contains("may only contain"));
        let bad_link = "apiVersion: v3\nkind: service\nmetadata:\n  name: x\n  links:\n    - {name: a, type: repo, url: 'javascript:alert(1)'}\n";
        assert!(err(bad_link).contains("http(s) URL"));
        let bad_contact = "apiVersion: v3\nkind: service\nmetadata:\n  name: x\n  contacts:\n    - {type: email, contact: nobody}\n";
        assert!(err(bad_contact).contains("valid email"));
        let bad_ref = "apiVersion: v3\nkind: service\nmetadata:\n  name: x\nspec:\n  dependsOn: ['host:i-0a1b2c3d4e5f60001']\n";
        assert!(err(bad_ref).contains("unknown kind"));
        let twice = "apiVersion: v3\nkind: queue\nmetadata: {name: q}\n---\napiVersion: v3\nkind: queue\nmetadata: {name: Q}\n";
        assert_eq!(err(twice), "Document 2: queue:q is defined twice");
        assert!(err("[]").contains("No entity definition"));
        assert!(err("apiVersion: v3\nkind: queue\nmetadata: {name: q}\n---\n- just\n- a list\n").starts_with("Document 2:"));
    }

    fn inputs() -> CatalogInputs {
        let svc = |name: &str, rps: f64| ApmService {
            name: name.into(),
            web: true,
            language: Some("go".into()),
            version: Some("1.4.0".into()),
            requests: 100,
            req_per_sec: rps,
            error_rate: 0.01,
            p95_ms: 120.0,
            last_deploy: Some(5),
        };
        let def = |id: &str, doc: &str| StoredDefinition { id: id.into(), definition: parse_documents(doc).unwrap().remove(0) };
        CatalogInputs {
            services: vec![svc("order-service", 2.0), svc("billing", 1.0)],
            log_services: BTreeMap::from([("order-service".into(), 10), ("legacy-cron".into(), 3)]),
            metric_services: BTreeSet::from(["cart-sync".to_string()]),
            peers: infer_entities(
                &[call("order-service", &[("db.system", "postgresql"), ("db.name", "orders-db")], 10), call("billing", &[("peer.service", "payments-provider")], 2)],
                &instrumented(),
            ),
            endpoints: vec![EndpointStat { service: "order-service".into(), resource: "POST /orders".into(), requests: 50, req_per_sec: 1.0, error_rate: 0.02, p95_ms: 80.0 }],
            error_types: HashMap::from([("order-service".to_string(), 3)]),
            definitions: vec![
                def("d1", "apiVersion: v3\nkind: datastore\nmetadata:\n  name: orders-db\n  owner: checkout\n  description: Orders\nspec:\n  componentOf: [checkout]\n"),
                def("d2", "apiVersion: v3\nkind: service\nmetadata:\n  name: order-service\n  owner: someone-else\n  description: Takes orders\nspec:\n  tier: '1'\n  componentOf: [checkout]\n"),
                def("d3", "apiVersion: v3\nkind: frontend\nmetadata:\n  name: storefront-web\n  owner: storefront\n  links: [{name: Code, type: repo, url: 'https://git.example.com/example-org/storefront-web'}]\n"),
            ],
            metadata: BTreeMap::from([(
                "order-service".to_string(),
                ServiceMeta {
                    team: "checkout".into(),
                    contacts: vec![ServiceContact { r#type: "slack".into(), name: String::new(), value: "#checkout".into() }],
                    repo_url: "https://git.example.com/example-org/order-service.git".into(),
                    docs_url: String::new(),
                },
            )]),
            slos: vec![ReliabilityRef { name: "Checkout availability".into(), monitors: vec!["Order errors".into()], ..Default::default() }],
            monitors: vec![
                ReliabilityRef { name: "Order errors".into(), texts: vec!["avg:trace.error_rate{service:order-service} > 5".into()], alerting: true, ..Default::default() },
                ReliabilityRef { name: "Billing latency".into(), texts: vec!["service:billing".into()], ..Default::default() },
                ReliabilityRef { name: "Slow orders".into(), texts: vec!["avg:trace.http.request{service:order-service,resource_name:post_/orders} > 1".into()], ..Default::default() },
            ],
            incidents: vec![
                IncidentRef { services: vec!["order-service".into()], open: true, in_range: true },
                IncidentRef { services: vec!["order-service".into(), "billing".into()], open: false, in_range: false },
            ],
            repos: vec![CiRepo { name: "example-org/order-service".into(), url: "https://git.example.com/example-org/order-service".into(), runs: 20, failures: 5, last_run: 9 }],
        }
    }

    fn find<'a>(all: &'a [PortalEntity], id: &str) -> &'a PortalEntity {
        all.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("no entity {id}"))
    }

    #[test]
    fn telemetry_definitions_and_metadata_merge_by_kind_and_name() {
        let all = assemble(&inputs());
        let order = find(&all, "service:order-service");
        assert_eq!(order.sources, vec!["apm", "definition", "logs", "metadata"]);
        assert!(!order.inferred);
        // The service metadata (shared with Scorecards) owns team, contacts and repository.
        assert_eq!(order.owner, "checkout");
        assert_eq!(order.contacts[0].contact, "#checkout");
        assert_eq!((order.description.as_str(), order.tier.as_deref()), ("Takes orders", Some("1")));
        assert_eq!((order.req_per_sec, order.version.as_deref()), (Some(2.0), Some("1.4.0")));
        assert_eq!(order.systems, vec!["checkout"]);

        let legacy = find(&all, "service:legacy-cron");
        assert!(legacy.inferred && legacy.sources == vec!["logs"]);
        assert!(find(&all, "service:cart-sync").inferred);

        let db = find(&all, "datastore:orders-db");
        assert_eq!((db.owner.as_str(), db.technology.as_deref(), db.related.clone()), ("checkout", Some("postgresql"), vec!["order-service".to_string()]));
        assert!(!db.inferred && db.definition_id.as_deref() == Some("d1"));
        let ext = find(&all, "external_provider:payments-provider");
        assert!(ext.inferred);

        // componentOf names a system that has no definition of its own.
        let system = find(&all, "system:checkout");
        assert_eq!(system.related, vec!["datastore:orders-db", "service:order-service"]);
    }

    #[test]
    fn services_get_reliability_and_endpoints_their_owner() {
        let all = assemble(&inputs());
        let order = find(&all, "service:order-service");
        assert_eq!((order.monitors, order.monitors_alerting, order.slos, order.incidents, order.open_incidents, order.error_issues), (2, 1, 1, 1, 1, Some(3)));
        let billing = find(&all, "service:billing");
        assert_eq!((billing.monitors, billing.slos, billing.incidents, billing.error_issues), (1, 0, 0, Some(0)));
        assert_eq!(find(&all, "service:legacy-cron").error_issues, None);
        let ep = find(&all, "endpoint:order-service/POST /orders");
        assert_eq!((ep.name.as_str(), ep.service.as_deref(), ep.owner.as_str()), ("POST /orders", Some("order-service"), "checkout"));
        // Only the monitor naming the route counts for the endpoint.
        assert_eq!((ep.monitors, ep.monitors_alerting), (1, 0));
    }

    #[test]
    fn repositories_come_from_ci_and_links_with_their_success_rate() {
        let all = assemble(&inputs());
        let repo = find(&all, "repository:example-org/order-service");
        assert_eq!(repo.sources, vec!["ci", "metadata"]);
        assert_eq!((repo.ci_runs, repo.ci_success_rate), (Some(20), Some(0.75)));
        assert_eq!(repo.related, vec!["service:order-service"]);
        let order = find(&all, "service:order-service");
        assert_eq!((order.ci_runs, order.ci_success_rate, order.last_ci_run), (Some(20), Some(0.75), Some(9)));
        let web = find(&all, "repository:example-org/storefront-web");
        assert_eq!((web.sources.clone(), web.related.clone(), web.owner.as_str()), (vec!["definition".to_string()], vec!["frontend:storefront-web".to_string()], "storefront"));
        let per_team = entities_per_team(&all);
        assert_eq!(per_team["checkout"], 3, "service, datastore and repository: {per_team:?}");
    }

    #[test]
    fn repository_paths_ignore_hosts_and_suffixes() {
        assert_eq!(repo_path("https://git.example.com/example-org/order-service.git"), "example-org/order-service");
        assert_eq!(repo_path("git@git.example.com:example-org/billing.git"), "example-org/billing");
        assert_eq!(repo_path("Example-Org/Billing/"), "example-org/billing");
        assert_eq!(repo_path("https://git.example.com"), "");
    }
}
