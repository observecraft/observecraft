//! Cloud Security (Security → Cloud Security): misconfigurations of the infrastructure and the
//! vulnerabilities of its hosts and container images.
//!
//! - **SBOMs**: the Datadog Agent (`sbom.enabled`, host and container image scanning) sends a CycloneDX
//!   bill of materials per host and image to the intake (`/api/v2/sbom`), which passes the packages on
//!   ([`ingest_sbom`]) into Postgres `csm_sbom_entities` / `csm_packages`. OS packages are keyed by
//!   their OSV ecosystem (`Debian:12`, `Alpine:v3.19`, …), language libraries as in Code Security.
//! - **Vulnerabilities**: those packages matched against the advisories Code Security keeps (OSV.dev,
//!   looked up for these packages too, and the demo's fictional ones), with whether the image is in use,
//!   whether a fix exists, and whether the CVE is known exploited (CISA's catalog).
//! - **Misconfigurations**: [`super::cloud_security_rules`] evaluated on the resource catalog and the
//!   Kubernetes objects' manifests, when read.
//! - Triage (mute, reopen) is Code Security's, with `misconfig:<rule>:<resource>` and
//!   `csm-vuln:<advisory>:<resource>:<package>` keys.
//!
//! Tenancy: Postgres rows filter on `org_id`, ClickHouse reads on `OrgId = {oc_org:String}` through
//! `AppState::ch(org)`; advisories and the exploited catalog are public reference data.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::catalog::{self, CatalogResource, CatalogType};
use super::cloud_security_rules::{target_of, Subject, RULES};
use super::code_security::{self, gunzip_if, internal_org};
use super::code_security_model::{self as model, AffectedRange};
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// SBOMs older than this are of resources gone.
const STALE_DAYS: i32 = 7;

// ================================================================ SBOM ingest

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SbomUpload {
    pub host: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub entities: Vec<SbomEntity>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SbomEntity {
    /// host | container_image
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub repo_tags: Vec<String>,
    #[serde(default)]
    pub repo_digests: Vec<String>,
    #[serde(default)]
    pub in_use: bool,
    #[serde(default)]
    pub kernel: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The same SBOM as last time: only "still there".
    #[serde(default)]
    pub heartbeat: bool,
    #[serde(default)]
    pub components: Vec<SbomComponent>,
}

#[derive(Deserialize, Debug)]
pub struct SbomComponent {
    /// Used when the purl has no version.
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub purl: String,
}

fn qualifier<'a>(purl: &'a str, key: &str) -> Option<&'a str> {
    purl.split_once('?')?.1.split(['&', '#']).find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

/// A package's OSV ecosystem, name and version from its purl: OS packages (deb, apk) by distribution
/// release, language packages as Code Security keys them.
pub fn package_of(purl: &str) -> Option<(String, String, String)> {
    let rest = purl.strip_prefix("pkg:")?;
    let (kind, after) = rest.split_once('/')?;
    if matches!(kind, "deb" | "apk") {
        let path = after.split(['?', '#']).next()?;
        let (path, version) = path.rsplit_once('@')?;
        let (vendor, name) = path.split_once('/').unwrap_or(("", path));
        let distro = qualifier(purl, "distro").unwrap_or_default();
        let release = distro.rsplit('-').next().unwrap_or(distro);
        let eco = match (kind, vendor) {
            ("deb", "debian") => format!("Debian:{}", release.split('.').next().unwrap_or(release)),
            ("deb", "ubuntu") => {
                let lts = release.ends_with(".04") && release.split('.').next().and_then(|y| y.parse::<u32>().ok()).is_some_and(|y| y % 2 == 0);
                format!("Ubuntu:{release}{}", if lts { ":LTS" } else { "" })
            }
            ("apk", _) => format!("Alpine:v{}", release.split('.').take(2).collect::<Vec<_>>().join(".")),
            _ => return None,
        };
        return Some((eco, name.to_string(), version.replace("%7E", "~").replace("%2B", "+")));
    }
    model::parse_purl(purl).map(|(e, n, v)| (e.to_string(), n, v))
}

/// The distribution of an SBOM, from its OS packages' purls (`debian 12`).
pub fn os_of(components: &[SbomComponent]) -> String {
    components
        .iter()
        .find_map(|c| {
            let (eco, _, _) = package_of(&c.purl)?;
            let (name, release) = eco.split_once(':')?;
            matches!(name, "Debian" | "Ubuntu" | "Alpine").then(|| format!("{} {}", name.to_lowercase(), release.trim_end_matches(":LTS").trim_start_matches('v')))
        })
        .unwrap_or_default()
}

/// Intake → server: the SBOMs of one Agent payload, as JSON ([`SbomUpload`]).
pub async fn ingest_sbom(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let org = match internal_org(&st, &headers) {
        Ok(o) => o,
        Err(s) => return s,
    };
    let Some(raw) = gunzip_if(&headers, &body) else { return StatusCode::BAD_REQUEST };
    let Ok(upload) = serde_json::from_slice::<SbomUpload>(&raw) else { return StatusCode::BAD_REQUEST };
    match store_sbom(&st, &org, &upload).await {
        Ok(()) => StatusCode::ACCEPTED,
        Err(e) => {
            tracing::warn!(error = %e, "storing SBOMs failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

pub async fn store_sbom(st: &AppState, org: &str, upload: &SbomUpload) -> Result<(), sqlx::Error> {
    for e in &upload.entities {
        if e.id.is_empty() || !matches!(e.kind.as_str(), "host" | "container_image") {
            continue;
        }
        let name = if e.kind == "host" { upload.host.clone() } else { e.repo_tags.first().cloned().unwrap_or_else(|| e.id.clone()) };
        let mut tx = st.postgres.begin().await?;
        sqlx::query(
            "INSERT INTO csm_sbom_entities (org_id, id, kind, name, host, repo_tags, repo_digests, in_use, os, kernel, arch, env, tags) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             ON CONFLICT (org_id, id) DO UPDATE SET name = $4, host = $5, repo_tags = $6, repo_digests = $7, in_use = $8, \
             os = CASE WHEN $9 = '' THEN csm_sbom_entities.os ELSE $9 END, kernel = $10, arch = $11, env = $12, tags = $13, last_seen = now()",
        )
        .bind(org)
        .bind(&e.id)
        .bind(&e.kind)
        .bind(&name)
        .bind(&upload.host)
        .bind(&e.repo_tags)
        .bind(&e.repo_digests)
        .bind(e.in_use || e.kind == "host")
        .bind(os_of(&e.components))
        .bind(&e.kernel)
        .bind(&e.arch)
        .bind(&upload.env)
        .bind(&e.tags)
        .execute(&mut *tx)
        .await?;
        if !(e.heartbeat && e.components.is_empty()) {
            let mut seen = HashSet::new();
            let (mut ecos, mut names, mut versions, mut purls) = (vec![], vec![], vec![], vec![]);
            for c in &e.components {
                let Some((eco, n, v)) = package_of(&c.purl) else { continue };
                let v = if v.is_empty() { c.version.clone() } else { v };
                if v.is_empty() || !seen.insert((eco.clone(), n.clone(), v.clone())) {
                    continue;
                }
                ecos.push(eco);
                names.push(n);
                versions.push(v);
                purls.push(c.purl.clone());
            }
            sqlx::query("DELETE FROM csm_packages WHERE org_id = $1 AND entity_id = $2 AND NOT ((ecosystem, name, version) IN (SELECT * FROM UNNEST($3::text[], $4::text[], $5::text[])))")
                .bind(org)
                .bind(&e.id)
                .bind(&ecos)
                .bind(&names)
                .bind(&versions)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO csm_packages (org_id, entity_id, ecosystem, name, version, purl) SELECT $1, $2, * FROM UNNEST($3::text[], $4::text[], $5::text[], $6::text[]) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(org)
            .bind(&e.id)
            .bind(&ecos)
            .bind(&names)
            .bind(&versions)
            .bind(&purls)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

// ================================================================ vulnerabilities

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InfraVulnerability {
    /// Triage key.
    pub key: String,
    pub advisory_id: String,
    pub cve: Option<String>,
    pub aliases: Vec<String>,
    pub title: String,
    pub severity: String,
    pub score: Option<f64>,
    pub cwes: Vec<String>,
    pub ecosystem: String,
    pub package: String,
    pub version: String,
    pub fixed: Option<String>,
    pub resource_id: String,
    pub resource_name: String,
    /// host | container_image
    pub resource_kind: String,
    pub host: String,
    pub env: String,
    pub os: String,
    /// A running container uses the image (hosts always run).
    pub running: bool,
    /// The CVE is in CISA's known exploited catalog.
    pub exploit_available: bool,
    /// open | muted
    pub status: String,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(sqlx::FromRow)]
struct VulnRow {
    entity_id: String,
    kind: String,
    entity_name: String,
    host: String,
    in_use: bool,
    env: String,
    os: String,
    ecosystem: String,
    name: String,
    version: String,
    first_seen: i64,
    last_seen: i64,
    advisory_id: String,
    ranges: String,
    versions: Vec<String>,
    aliases: Vec<String>,
    summary: String,
    severity: String,
    score: Option<f32>,
    cwes: Vec<String>,
}

pub async fn infra_vulnerabilities(st: &AppState, org: &str) -> Result<Vec<InfraVulnerability>, ApiError> {
    let rows: Vec<VulnRow> = sqlx::query_as(
        "SELECT e.id AS entity_id, e.kind, e.name AS entity_name, e.host, e.in_use, e.env, e.os, p.ecosystem, p.name, p.version, \
         (EXTRACT(EPOCH FROM p.first_seen) * 1000)::bigint AS first_seen, (EXTRACT(EPOCH FROM e.last_seen) * 1000)::bigint AS last_seen, \
         ap.advisory_id, ap.ranges::text AS ranges, ap.versions, a.aliases, a.summary, a.severity, a.score, a.cwes \
         FROM csm_packages p JOIN csm_sbom_entities e ON e.org_id = p.org_id AND e.id = p.entity_id \
         JOIN cs_advisory_packages ap ON ap.ecosystem = p.ecosystem AND ap.package = p.name JOIN cs_advisories a ON a.id = ap.advisory_id \
         WHERE p.org_id = $1 AND a.withdrawn IS NULL AND e.last_seen > now() - make_interval(days => $2)",
    )
    .bind(org)
    .bind(STALE_DAYS)
    .fetch_all(&st.postgres)
    .await?;
    let triage = code_security::triage_map(st, org).await?;
    let kev: HashSet<String> = sqlx::query_scalar("SELECT cve_id FROM security_kev").fetch_all(&st.postgres).await?.into_iter().collect();
    let mut out: Vec<InfraVulnerability> = rows
        .into_iter()
        .filter_map(|r| {
            let ranges: Vec<AffectedRange> = serde_json::from_str(&r.ranges).unwrap_or_default();
            if !model::is_affected(&r.version, &ranges, &r.versions) {
                return None;
            }
            let key = format!("csm-vuln:{}:{}:{}", r.advisory_id, r.entity_id, r.name);
            Some(InfraVulnerability {
                status: triage.get(&key).cloned().unwrap_or_else(|| "open".into()),
                key,
                cve: r.aliases.iter().find(|a| a.starts_with("CVE-")).cloned(),
                exploit_available: r.aliases.iter().any(|a| kev.contains(a)),
                aliases: r.aliases,
                title: if r.summary.is_empty() { r.advisory_id.clone() } else { r.summary },
                severity: r.severity,
                score: r.score.map(|s| (s as f64 * 10.0).round() / 10.0),
                cwes: r.cwes,
                fixed: model::fix_for(&r.version, &ranges),
                ecosystem: r.ecosystem,
                package: r.name,
                version: r.version,
                advisory_id: r.advisory_id,
                resource_id: r.entity_id,
                resource_name: r.entity_name,
                resource_kind: r.kind,
                host: r.host,
                env: r.env,
                os: r.os,
                running: r.in_use,
                first_seen: r.first_seen,
                last_seen: r.last_seen,
            })
        })
        .collect();
    out.sort_by(|a, b| model::severity_rank(&b.severity).cmp(&model::severity_rank(&a.severity)).then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)).then(a.key.cmp(&b.key)));
    Ok(out)
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FindingStatusFilter {
    /// open (default) | muted | all
    pub status: String,
}

fn keep(status: &str, filter: &str) -> bool {
    match filter {
        "all" => true,
        "" | "open" => status == "open",
        f => status == f,
    }
}

#[utoipa::path(post, path = "/api/v1/cloud-security/vulnerabilities", operation_id = "cloud_security_vulnerabilities", request_body = FindingStatusFilter,
    responses((status = 200, body = Vec<InfraVulnerability>)), tag = "cloud-security")]
pub async fn vulnerabilities(State(st): State<AppState>, ctx: AuthContext, Json(f): Json<FindingStatusFilter>) -> ApiResult<Vec<InfraVulnerability>> {
    Ok(Json(infra_vulnerabilities(&st, &ctx.org_id).await?.into_iter().filter(|v| keep(&v.status, &f.status)).collect()))
}

// ================================================================ misconfigurations

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MisconfigFinding {
    /// Triage key.
    pub key: String,
    pub rule_id: String,
    pub rule_name: String,
    pub severity: String,
    pub description: String,
    pub remediation: String,
    pub frameworks: Vec<String>,
    /// What fails on this resource.
    pub detail: String,
    pub resource_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub cluster: Option<String>,
    pub namespace: Option<String>,
    pub account: Option<String>,
    pub region: Option<String>,
    pub env: Option<String>,
    pub service: Option<String>,
    pub team: Option<String>,
    /// open | muted
    pub status: String,
    pub last_seen: i64,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MisconfigRuleStatus {
    pub id: String,
    pub name: String,
    pub severity: String,
    pub description: String,
    pub remediation: String,
    pub frameworks: Vec<String>,
    /// Resources the rule applies to, and how many fail it.
    pub evaluated: usize,
    pub failing: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MisconfigPage {
    pub findings: Vec<MisconfigFinding>,
    pub rules: Vec<MisconfigRuleStatus>,
}

#[derive(clickhouse::Row, Deserialize)]
struct ManifestRow {
    uid: String,
    body: String,
}

/// Kubernetes objects' current manifests, by uid.
async fn manifests(st: &AppState, org: &str) -> Result<HashMap<String, Value>, ApiError> {
    Ok(st
        .ch(org)
        .query("SELECT Uid AS uid, Body AS body FROM otel.k8s_resources WHERE OrgId = {oc_org:String} AND LastSeen > now() - INTERVAL 15 MINUTE ORDER BY LastSeen DESC LIMIT 1 BY Uid")
        .fetch_all::<ManifestRow>()
        .await?
        .into_iter()
        .filter_map(|b| serde_json::from_str(&b.body).ok().map(|v| (b.uid, v)))
        .collect())
}

pub fn type_name(t: CatalogType) -> String {
    serde_json::to_value(t).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Every rule on every resource it applies to: the failures, and per rule how many were evaluated.
pub fn evaluate(resources: &[CatalogResource], manifests: &HashMap<String, Value>, triage: &HashMap<String, String>) -> (Vec<MisconfigFinding>, Vec<MisconfigRuleStatus>) {
    let mut findings = Vec::new();
    let mut stats: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for r in resources {
        let Some(target) = target_of(r) else { continue };
        let manifest = manifests.get(&r.id);
        // Kubernetes rules need the object's manifest.
        if target != super::cloud_security_rules::Target::Host && manifest.is_none() {
            continue;
        }
        let subject = Subject { resource: r, manifest };
        for rule in RULES.iter().filter(|x| x.target == target) {
            let s = stats.entry(rule.id).or_default();
            s.0 += 1;
            let Some(detail) = (rule.check)(&subject) else { continue };
            s.1 += 1;
            let key = format!("misconfig:{}:{}", rule.id, r.id);
            findings.push(MisconfigFinding {
                status: triage.get(&key).cloned().unwrap_or_else(|| "open".into()),
                key,
                rule_id: rule.id.into(),
                rule_name: rule.name.into(),
                severity: rule.severity.into(),
                description: rule.description.into(),
                remediation: rule.remediation.into(),
                frameworks: rule.frameworks.iter().map(|f| f.to_string()).collect(),
                detail,
                resource_id: r.id.clone(),
                resource_name: r.name.clone(),
                resource_type: type_name(r.resource_type),
                cluster: r.cluster.clone(),
                namespace: r.namespace.clone(),
                account: r.account.clone(),
                region: r.region.clone(),
                env: r.env.clone(),
                service: r.service.clone(),
                team: r.team.clone(),
                last_seen: r.last_seen,
            });
        }
    }
    findings.sort_by(|a, b| model::severity_rank(&b.severity).cmp(&model::severity_rank(&a.severity)).then(a.rule_id.cmp(&b.rule_id)).then(a.resource_name.cmp(&b.resource_name)));
    let rules = RULES
        .iter()
        .map(|r| {
            let (evaluated, failing) = stats.get(r.id).copied().unwrap_or_default();
            MisconfigRuleStatus {
                id: r.id.into(),
                name: r.name.into(),
                severity: r.severity.into(),
                description: r.description.into(),
                remediation: r.remediation.into(),
                frameworks: r.frameworks.iter().map(|f| f.to_string()).collect(),
                evaluated,
                failing,
            }
        })
        .collect();
    (findings, rules)
}

pub async fn misconfigurations_of(st: &AppState, org: &str) -> Result<(Vec<MisconfigFinding>, Vec<MisconfigRuleStatus>), ApiError> {
    let resources = catalog::load_all(st, org).await?;
    let manifests = manifests(st, org).await?;
    let triage = code_security::triage_map(st, org).await?;
    Ok(evaluate(&resources, &manifests, &triage))
}

#[utoipa::path(post, path = "/api/v1/cloud-security/misconfigurations", operation_id = "cloud_security_misconfigurations", request_body = FindingStatusFilter,
    responses((status = 200, body = MisconfigPage)), tag = "cloud-security")]
pub async fn misconfigurations(State(st): State<AppState>, ctx: AuthContext, Json(f): Json<FindingStatusFilter>) -> ApiResult<MisconfigPage> {
    let (findings, rules) = misconfigurations_of(&st, &ctx.org_id).await?;
    Ok(Json(MisconfigPage { findings: findings.into_iter().filter(|x| keep(&x.status, &f.status)).collect(), rules }))
}

// ================================================================ inventory

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCategory {
    /// Hosts, Containers, Kubernetes, Networking, Storage, Container Images, …
    pub category: String,
    pub count: usize,
    pub types: BTreeMap<String, usize>,
    pub regions: Vec<String>,
    pub envs: Vec<String>,
    pub teams: Vec<String>,
    pub services: Vec<String>,
}

fn category_of(t: CatalogType) -> &'static str {
    match t {
        CatalogType::Host | CatalogType::KubeNode => "Hosts",
        CatalogType::Container | CatalogType::KubePod | CatalogType::EcsTask => "Containers",
        CatalogType::KubeService | CatalogType::KubeIngress => "Networking",
        CatalogType::KubePersistentVolume | CatalogType::KubePersistentVolumeClaim => "Storage",
        _ => "Orchestration",
    }
}

#[utoipa::path(get, path = "/api/v1/cloud-security/resources", operation_id = "cloud_security_resources", responses((status = 200, body = Vec<ResourceCategory>)), tag = "cloud-security")]
pub async fn resources(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<ResourceCategory>> {
    Ok(Json(categories(&st, &ctx.org_id).await?))
}

pub async fn categories(st: &AppState, org: &str) -> Result<Vec<ResourceCategory>, ApiError> {
    let all = catalog::load_all(st, org).await?;
    #[derive(Default)]
    struct Acc {
        count: usize,
        types: BTreeMap<String, usize>,
        regions: BTreeSet<String>,
        envs: BTreeSet<String>,
        teams: BTreeSet<String>,
        services: BTreeSet<String>,
    }
    let mut m: BTreeMap<&str, Acc> = BTreeMap::new();
    for r in &all {
        let a = m.entry(category_of(r.resource_type)).or_default();
        a.count += 1;
        *a.types.entry(type_name(r.resource_type)).or_default() += 1;
        a.regions.extend(r.region.clone());
        a.envs.extend(r.env.clone());
        a.teams.extend(r.team.clone());
        a.services.extend(r.service.clone());
    }
    let images: Vec<(String, bool)> = sqlx::query_as("SELECT env, in_use FROM csm_sbom_entities WHERE org_id = $1 AND kind = 'container_image' AND last_seen > now() - make_interval(days => $2)")
        .bind(org)
        .bind(STALE_DAYS)
        .fetch_all(&st.postgres)
        .await?;
    if !images.is_empty() {
        let a = m.entry("Container Images").or_default();
        for (env, in_use) in images {
            a.count += 1;
            *a.types.entry(if in_use { "running" } else { "not running" }.into()).or_default() += 1;
            if !env.is_empty() {
                a.envs.insert(env);
            }
        }
    }
    let mut out: Vec<ResourceCategory> = m
        .into_iter()
        .map(|(k, a)| ResourceCategory { category: k.into(), count: a.count, types: a.types, regions: a.regions.into_iter().collect(), envs: a.envs.into_iter().collect(), teams: a.teams.into_iter().collect(), services: a.services.into_iter().collect() })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.count));
    Ok(out)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InfraPackage {
    pub ecosystem: String,
    pub name: String,
    pub version: String,
    /// Hosts and images with the package, and how many of them run.
    pub resources: usize,
    pub running: usize,
    pub vulnerabilities: usize,
    pub max_severity: Option<String>,
    pub first_seen: i64,
}

#[derive(sqlx::FromRow)]
struct PackageRow {
    ecosystem: String,
    name: String,
    version: String,
    resources: i64,
    running: i64,
    first_seen: i64,
}

#[utoipa::path(get, path = "/api/v1/cloud-security/packages", operation_id = "cloud_security_packages", responses((status = 200, body = Vec<InfraPackage>)), tag = "cloud-security")]
pub async fn packages(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<InfraPackage>> {
    let rows: Vec<PackageRow> = sqlx::query_as(
        "SELECT p.ecosystem, p.name, p.version, count(DISTINCT p.entity_id) AS resources, count(DISTINCT p.entity_id) FILTER (WHERE e.in_use) AS running, \
         (EXTRACT(EPOCH FROM min(p.first_seen)) * 1000)::bigint AS first_seen \
         FROM csm_packages p JOIN csm_sbom_entities e ON e.org_id = p.org_id AND e.id = p.entity_id \
         WHERE p.org_id = $1 AND e.last_seen > now() - make_interval(days => $2) GROUP BY p.ecosystem, p.name, p.version ORDER BY p.name, p.version LIMIT 20000",
    )
    .bind(&ctx.org_id)
    .bind(STALE_DAYS)
    .fetch_all(&st.postgres)
    .await?;
    let vulns = infra_vulnerabilities(&st, &ctx.org_id).await?;
    // (ecosystem, name, version) → advisories and the most severe.
    type PackageKey = (String, String, String);
    let mut by_pkg: HashMap<PackageKey, (HashSet<String>, Option<String>)> = HashMap::new();
    for v in &vulns {
        let e = by_pkg.entry((v.ecosystem.clone(), v.package.clone(), v.version.clone())).or_default();
        e.0.insert(v.advisory_id.clone());
        if e.1.as_ref().is_none_or(|s| model::severity_rank(&v.severity) > model::severity_rank(s)) {
            e.1 = Some(v.severity.clone());
        }
    }
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let (ids, max) = by_pkg.remove(&(r.ecosystem.clone(), r.name.clone(), r.version.clone())).unwrap_or_default();
                InfraPackage { ecosystem: r.ecosystem, name: r.name, version: r.version, resources: r.resources as usize, running: r.running as usize, vulnerabilities: ids.len(), max_severity: max, first_seen: r.first_seen }
            })
            .collect(),
    ))
}

// ================================================================ CVE explorer

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CveQuery {
    /// CVE, advisory id or package name.
    pub query: String,
    /// critical | high | medium | low (empty: all)
    pub severity: String,
    /// impacted | not_impacted (empty: all)
    pub impact: String,
    /// true: only CVEs in CISA's known exploited catalog.
    pub exploited: bool,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CveRow {
    pub id: String,
    pub cve: Option<String>,
    pub summary: String,
    pub severity: String,
    pub score: Option<f64>,
    /// `ecosystem:package`
    pub packages: Vec<String>,
    pub published: Option<i64>,
    /// Libraries (Code Security) or hosts and images (Cloud Security) the advisory affects.
    pub impacted_libraries: usize,
    pub impacted_resources: usize,
    pub exploited: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CvePage {
    pub total: i64,
    pub rows: Vec<CveRow>,
}

#[derive(sqlx::FromRow)]
struct AdvisoryRow {
    id: String,
    aliases: Vec<String>,
    summary: String,
    severity: String,
    score: Option<f32>,
    packages: Vec<String>,
    published: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/cloud-security/cves", operation_id = "cloud_security_cves", params(("query" = Option<String>, Query), ("severity" = Option<String>, Query),
    ("impact" = Option<String>, Query), ("exploited" = Option<bool>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)),
    responses((status = 200, body = CvePage)), tag = "cloud-security")]
pub async fn cves(State(st): State<AppState>, ctx: AuthContext, Query(q): Query<CveQuery>) -> ApiResult<CvePage> {
    // What affects the org, by advisory.
    let infra = infra_vulnerabilities(&st, &ctx.org_id).await?;
    let libs = code_security::list_library_vulnerabilities(State(st.clone()), crate::state::OrgId(ctx.org_id.clone()), Json(code_security::CsFilter { status: "all".into(), scope: "all".into(), limit: 5000, ..Default::default() }))
        .await?
        .0
        .vulnerabilities;
    let mut resources: HashMap<String, HashSet<String>> = HashMap::new();
    for v in &infra {
        resources.entry(v.advisory_id.clone()).or_default().insert(v.resource_id.clone());
    }
    let mut libraries: HashMap<String, HashSet<String>> = HashMap::new();
    for v in &libs {
        libraries.entry(v.advisory_id.clone()).or_default().insert(v.key.clone());
    }
    let impacted: Vec<String> = resources.keys().chain(libraries.keys()).cloned().collect::<BTreeSet<_>>().into_iter().collect();
    let kev: Vec<String> = sqlx::query_scalar("SELECT cve_id FROM security_kev").fetch_all(&st.postgres).await?;
    let text = format!("%{}%", q.query.trim().to_lowercase());
    let limit = if q.limit <= 0 { 25 } else { q.limit.min(200) };
    // Public advisories (no org data); the org's impact is bound as a list of ids.
    let filter = "WHERE a.withdrawn IS NULL AND ($1 = '%%' OR lower(a.id) LIKE $1 OR lower(a.summary) LIKE $1 OR EXISTS (SELECT 1 FROM unnest(a.aliases) x WHERE lower(x) LIKE $1) \
         OR EXISTS (SELECT 1 FROM cs_advisory_packages p WHERE p.advisory_id = a.id AND lower(p.package) LIKE $1)) \
         AND ($2 = '' OR a.severity = $2) AND ($3 = '' OR ($3 = 'impacted') = (a.id = ANY($4))) AND (NOT $5 OR a.aliases && $6)";
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM cs_advisories a {filter}"))
        .bind(&text)
        .bind(&q.severity)
        .bind(&q.impact)
        .bind(&impacted)
        .bind(q.exploited)
        .bind(&kev)
        .fetch_one(&st.postgres)
        .await?;
    let rows: Vec<AdvisoryRow> = sqlx::query_as(&format!(
        "SELECT a.id, a.aliases, a.summary, a.severity, a.score, \
         ARRAY(SELECT p.ecosystem || ':' || p.package FROM cs_advisory_packages p WHERE p.advisory_id = a.id ORDER BY 1) AS packages, \
         (EXTRACT(EPOCH FROM a.published) * 1000)::bigint AS published FROM cs_advisories a {filter} \
         ORDER BY (a.id = ANY($4)) DESC, a.published DESC NULLS LAST, a.id LIMIT $7 OFFSET $8"
    ))
    .bind(&text)
    .bind(&q.severity)
    .bind(&q.impact)
    .bind(&impacted)
    .bind(q.exploited)
    .bind(&kev)
    .bind(limit)
    .bind(q.offset.max(0))
    .fetch_all(&st.postgres)
    .await?;
    let kev: HashSet<String> = kev.into_iter().collect();
    Ok(Json(CvePage {
        total,
        rows: rows
            .into_iter()
            .map(|r| CveRow {
                cve: r.aliases.iter().find(|a| a.starts_with("CVE-")).cloned(),
                exploited: r.aliases.iter().any(|a| kev.contains(a)),
                impacted_libraries: libraries.get(&r.id).map_or(0, HashSet::len),
                impacted_resources: resources.get(&r.id).map_or(0, HashSet::len),
                id: r.id,
                summary: r.summary,
                severity: r.severity,
                score: r.score.map(|s| (s as f64 * 10.0).round() / 10.0),
                packages: r.packages,
                published: r.published,
            })
            .collect(),
    }))
}

// ================================================================ summary

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct VulnerabilityPrioritization {
    /// Unique vulnerabilities (advisories) of hosts and images.
    pub total: usize,
    pub running: usize,
    pub exploit_available: usize,
    pub fix_available: usize,
    pub high_or_critical: usize,
    /// Running, high or critical, and known exploited.
    pub high_exploit_risk: usize,
}

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloudCoverage {
    pub hosts: usize,
    /// Hosts that sent an SBOM in the last week.
    pub hosts_scanned: usize,
    pub images: usize,
    pub images_running: usize,
    pub clusters: usize,
    /// Kubernetes objects evaluated by the misconfiguration rules.
    pub resources_evaluated: usize,
    /// Cloud accounts and their providers seen in the catalog.
    pub accounts: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudSecuritySummary {
    pub vulnerabilities: VulnerabilityPrioritization,
    pub coverage: CloudCoverage,
    /// Open misconfigurations by severity.
    pub misconfigurations: BTreeMap<String, usize>,
    pub vulnerabilities_by_severity: BTreeMap<String, usize>,
    pub inventory: Vec<ResourceCategory>,
}

#[utoipa::path(get, path = "/api/v1/cloud-security/summary", operation_id = "cloud_security_summary", responses((status = 200, body = CloudSecuritySummary)), tag = "cloud-security")]
pub async fn summary(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<CloudSecuritySummary> {
    let org = &ctx.org_id;
    let vulns: Vec<InfraVulnerability> = infra_vulnerabilities(&st, org).await?.into_iter().filter(|v| v.status == "open").collect();
    let uniq = |f: &dyn Fn(&InfraVulnerability) -> bool| vulns.iter().filter(|v| f(v)).map(|v| v.advisory_id.as_str()).collect::<HashSet<_>>().len();
    let high = |v: &InfraVulnerability| matches!(v.severity.as_str(), "critical" | "high");
    let prioritization = VulnerabilityPrioritization {
        total: uniq(&|_| true),
        running: uniq(&|v| v.running),
        exploit_available: uniq(&|v| v.exploit_available),
        fix_available: uniq(&|v| v.fixed.is_some()),
        high_or_critical: uniq(&high),
        high_exploit_risk: uniq(&|v| v.running && v.exploit_available && high(v)),
    };
    let mut by_severity: BTreeMap<String, usize> = ["critical", "high", "medium", "low"].iter().map(|s| (s.to_string(), 0)).collect();
    for v in &vulns {
        *by_severity.entry(v.severity.clone()).or_default() += 1;
    }
    let resources = catalog::load_all(&st, org).await?;
    let manifests = manifests(&st, org).await?;
    let triage = code_security::triage_map(&st, org).await?;
    let (findings, _) = evaluate(&resources, &manifests, &triage);
    let mut misconfigs: BTreeMap<String, usize> = ["critical", "high", "medium", "low"].iter().map(|s| (s.to_string(), 0)).collect();
    for f in findings.iter().filter(|f| f.status == "open") {
        *misconfigs.entry(f.severity.clone()).or_default() += 1;
    }
    let (hosts_scanned, images, images_running): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE kind = 'host'), count(*) FILTER (WHERE kind = 'container_image'), count(*) FILTER (WHERE kind = 'container_image' AND in_use) \
         FROM csm_sbom_entities WHERE org_id = $1 AND last_seen > now() - make_interval(days => $2)",
    )
    .bind(org)
    .bind(STALE_DAYS)
    .fetch_one(&st.postgres)
    .await?;
    let accounts: BTreeSet<String> = resources.iter().filter_map(|r| Some(format!("{}:{}", r.cloud.clone()?, r.account.clone()?))).collect();
    let clusters: BTreeSet<&str> = resources.iter().filter_map(|r| r.cluster.as_deref()).collect();
    Ok(Json(CloudSecuritySummary {
        vulnerabilities: prioritization,
        coverage: CloudCoverage {
            hosts: resources.iter().filter(|r| r.resource_type == CatalogType::Host).count(),
            hosts_scanned: hosts_scanned as usize,
            images: images as usize,
            images_running: images_running as usize,
            clusters: clusters.len(),
            resources_evaluated: resources.iter().filter(|r| target_of(r).is_some() && (r.resource_type == CatalogType::Host || manifests.contains_key(&r.id))).count(),
            accounts: accounts.into_iter().collect(),
        },
        misconfigurations: misconfigs,
        vulnerabilities_by_severity: by_severity,
        inventory: categories(&st, org).await?,
    }))
}

// ================================================================ demo

/// Fictional advisories on the demo SBOMs' fictional OS packages (oc-tracegen's `sbom.rs`).
pub async fn seed_demo_advisories(st: &AppState) -> Result<(), ApiError> {
    let n = "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H";
    let advisories = [
        ("OCSA-DEMO-2026-0101", "CVE-2026-90101", "libexample-ssl3: certificate chain verification bypass", n, "CWE-295", "Debian:12", "libexample-ssl3", "3.0.0", "3.0.15-1~deb12u1", 18),
        ("OCSA-DEMO-2026-0102", "CVE-2026-90102", "libexample-ssl3: out-of-bounds read in the record layer", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:H", "CWE-125", "Debian:12", "libexample-ssl3", "3.0.0", "3.0.14-1~deb12u2", 64),
        ("OCSA-DEMO-2026-0103", "CVE-2026-90103", "example-zlib: heap overflow when inflating crafted streams", "CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:U/C:H/I:H/A:H", "CWE-787", "Debian:12", "example-zlib", "1.2.0", "1.2.13.dfsg-1+deb12u1", 40),
        ("OCSA-DEMO-2026-0104", "CVE-2026-90104", "examplecurl: credentials leaked on cross-origin redirects", "CVSS:3.1/AV:N/AC:H/PR:N/UI:R/S:U/C:H/I:N/A:N", "CWE-200", "Debian:12", "examplecurl", "7.0.0", "7.88.1-10+deb12u8", 9),
        ("OCSA-DEMO-2026-0105", "CVE-2026-90105", "example-musl: stack overflow in the DNS resolver", n, "CWE-121", "Alpine:v3.19", "example-musl", "1.2.0", "1.2.4-r3", 27),
        ("OCSA-DEMO-2026-0106", "CVE-2026-90106", "example-busybox: command injection in the wget applet", "CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:H/A:N", "CWE-78", "Alpine:v3.19", "example-busybox", "1.36.0", "1.36.1-r16", 3),
        ("OCSA-DEMO-2026-0107", "CVE-2026-90107", "example-sudo: local privilege escalation through the editor option", "CVSS:3.1/AV:L/AC:L/PR:L/UI:N/S:U/C:H/I:H/A:H", "CWE-269", "Ubuntu:22.04:LTS", "example-sudo", "1.9.0", "1.9.9-1ubuntu2.5", 33),
        ("OCSA-DEMO-2026-0108", "CVE-2026-90108", "example-kernel-tools: use after free in the perf helper", "CVSS:3.1/AV:L/AC:L/PR:L/UI:N/S:U/C:H/I:H/A:H", "CWE-416", "Ubuntu:22.04:LTS", "example-kernel-tools", "5.15.0", "5.15.0-125.135", 120),
    ];
    for a in advisories {
        let adv = code_security::demo_advisory(a);
        code_security::store_advisory(&st.postgres, code_security::AdvisoryWrite(&adv, "demo")).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purls_map_to_osv_ecosystems() {
        assert_eq!(package_of("pkg:deb/debian/libexample-ssl3@3.0.11-1~deb12u2?arch=amd64&distro=debian-12").unwrap(), ("Debian:12".into(), "libexample-ssl3".into(), "3.0.11-1~deb12u2".into()));
        assert_eq!(package_of("pkg:deb/ubuntu/example-sudo@1.9.9-1ubuntu2.4?distro=ubuntu-22.04").unwrap().0, "Ubuntu:22.04:LTS");
        assert_eq!(package_of("pkg:deb/ubuntu/x@1?distro=ubuntu-23.10").unwrap().0, "Ubuntu:23.10");
        assert_eq!(package_of("pkg:apk/alpine/example-musl@1.2.4-r2?arch=x86_64&distro=3.19.1").unwrap(), ("Alpine:v3.19".into(), "example-musl".into(), "1.2.4-r2".into()));
        assert_eq!(package_of("pkg:maven/com.example.json/json-core@2.15.0").unwrap().0, "Maven");
        assert!(package_of("pkg:rpm/redhat/openssl@3.0.7").is_none());
        let comps = vec![SbomComponent { version: "1".into(), purl: "pkg:deb/debian/x@1?distro=debian-12".into() }];
        assert_eq!(os_of(&comps), "debian 12");
    }

    #[test]
    fn findings_filter_by_status() {
        assert!(keep("open", ""));
        assert!(!keep("muted", "open"));
        assert!(keep("muted", "muted"));
        assert!(keep("muted", "all"));
    }

    /// An image with a vulnerable package shows up as a vulnerability, a host SBOM's OS is kept, and a
    /// heartbeat keeps the packages. Live: skipped unless OC_TEST_POSTGRES_URL and
    /// OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn sboms_become_vulnerabilities() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "csm").await;
        seed_demo_advisories(st).await.unwrap();
        let upload: SbomUpload = serde_json::from_value(serde_json::json!({
            "host": "i-0a1b2c3d4e5f60001", "env": "prod",
            "entities": [
                {"kind": "host", "id": "i-0a1b2c3d4e5f60001", "components": [
                    {"name": "libexample-ssl3", "version": "3.0.11-1~deb12u2", "purl": "pkg:deb/debian/libexample-ssl3@3.0.11-1~deb12u2?distro=debian-12"},
                    {"name": "bash", "version": "5.2.15-2+b2", "purl": "pkg:deb/debian/bash@5.2.15-2+b2?distro=debian-12"}]},
                {"kind": "container_image", "id": "sha256:abc", "repoTags": ["registry.example.com/storefront-api:1.42.0"], "inUse": true, "components": [
                    {"name": "example-musl", "version": "1.2.4-r2", "purl": "pkg:apk/alpine/example-musl@1.2.4-r2?distro=3.19.1"}]}
            ]
        }))
        .unwrap();
        store_sbom(st, &org, &upload).await.unwrap();
        let v = infra_vulnerabilities(st, &org).await.unwrap();
        let ids: BTreeSet<&str> = v.iter().map(|x| x.advisory_id.as_str()).collect();
        assert_eq!(ids, ["OCSA-DEMO-2026-0101", "OCSA-DEMO-2026-0102", "OCSA-DEMO-2026-0105"].into_iter().collect());
        let musl = v.iter().find(|x| x.package == "example-musl").unwrap();
        assert!(musl.running);
        assert_eq!(musl.resource_name, "registry.example.com/storefront-api:1.42.0");
        assert_eq!(musl.fixed.as_deref(), Some("1.2.4-r3"));
        let os: String = sqlx::query_scalar("SELECT os FROM csm_sbom_entities WHERE org_id = $1 AND kind = 'host'").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(os, "debian 12");
        let heartbeat: SbomUpload = serde_json::from_value(serde_json::json!({"host": "i-0a1b2c3d4e5f60001", "entities": [{"kind": "host", "id": "i-0a1b2c3d4e5f60001", "heartbeat": true}]})).unwrap();
        store_sbom(st, &org, &heartbeat).await.unwrap();
        assert_eq!(infra_vulnerabilities(st, &org).await.unwrap().len(), v.len());
    }
}
