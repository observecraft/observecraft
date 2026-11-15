//! Cloud Security (Security → Cloud Security): misconfigurations of the infrastructure and the
//! vulnerabilities of its hosts and container images.
//!
//! - **SBOMs**: the Datadog Agent (`sbom.enabled`, host and container image scanning) sends a CycloneDX
//!   bill of materials per host and image to the intake (`/api/v2/sbom`), which passes the packages on
//!   ([`ingest_sbom`]) into Postgres `csm_sbom_entities` / `csm_packages`. OS packages are keyed by
//!   their OSV ecosystem (`Debian:12`, `Alpine:v3.19`, …), language libraries as in Code Security.
//! - **Vulnerabilities**: those packages matched against the advisories Code Security keeps (OSV.dev,
//!   looked up for these packages too, and the demo's fictional ones), with whether the image is in use,
//!   whether a fix exists, and whether the CVE is known exploited (CISA's catalog). The version ranges are compared
//!   by Code Security's matching pass (`cs_version_matches`), so reads are joins and counts in the database.
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
use super::code_security_model as model;
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

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct SbomComponent {
    /// Used when the purl has no version.
    pub version: String,
    pub purl: String,
    /// The source package of an OS package (Trivy's `SrcName`), and its `epoch:version-release`.
    pub src_name: String,
    pub src_version: String,
}

fn qualifier<'a>(purl: &'a str, key: &str) -> Option<&'a str> {
    purl.split_once('?')?.1.split(['&', '#']).find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

/// A package's OSV ecosystem, name and version from its purl: OS packages (deb, rpm, apk) by distribution
/// release with the `epoch=` qualifier in the version (`1:1.5.2-6+deb12u1`), language packages as Code
/// Security keys them.
pub fn package_of(purl: &str) -> Option<(String, String, String)> {
    let rest = purl.strip_prefix("pkg:")?;
    let (kind, after) = rest.split_once('/')?;
    if matches!(kind, "deb" | "apk" | "rpm") {
        let path = after.split(['?', '#']).next()?;
        let (path, version) = path.rsplit_once('@')?;
        let (vendor, name) = path.split_once('/').unwrap_or(("", path));
        let distro = qualifier(purl, "distro").unwrap_or_default();
        // `debian-12.5`, `amazon-2023.3.20240219`, `opensuse.leap-15.5`; Alpine's is the bare release.
        let release = distro.strip_prefix(vendor).map(|r| r.trim_start_matches('-')).unwrap_or_else(|| distro.rsplit('-').next().unwrap_or(distro));
        let major = release.split(['.', ' ']).next().unwrap_or(release);
        let eco = match (kind, vendor) {
            ("deb", "debian") => format!("Debian:{major}"),
            ("deb", "ubuntu") => {
                let lts = release.ends_with(".04") && major.parse::<u32>().is_ok_and(|y| y % 2 == 0);
                format!("Ubuntu:{release}{}", if lts { ":LTS" } else { "" })
            }
            ("apk", "alpine") => format!("Alpine:v{}", release.split('.').take(2).collect::<Vec<_>>().join(".")),
            ("apk", "wolfi") => "Wolfi".into(),
            ("apk", "chainguard") => "Chainguard".into(),
            ("rpm", "alma" | "almalinux") => format!("AlmaLinux:{major}"),
            ("rpm", "rocky") => format!("Rocky Linux:{major}"),
            // OSV's Red Hat advisories are keyed by product stream (`Red Hat:enterprise_linux:9::baseos`): listed, rarely matched.
            ("rpm", "redhat") => format!("Red Hat:{major}"),
            ("rpm", "centos") => format!("CentOS:{major}"),
            ("rpm", "amazon") => format!("Amazon Linux:{major}"),
            ("rpm", "oracle") => format!("Oracle Linux:{major}"),
            ("rpm", "fedora") => format!("Fedora:{major}"),
            ("rpm", "photon") => format!("Photon OS:{}", if release.contains('.') { release.to_string() } else { format!("{release}.0") }),
            ("rpm", "mageia") => format!("Mageia:{major}"),
            ("rpm", "opensuse.leap" | "opensuse-leap") => format!("openSUSE:Leap {release}"),
            ("rpm", "opensuse.tumbleweed" | "opensuse-tumbleweed") => "openSUSE:Tumbleweed".into(),
            ("rpm", "sles" | "suse") => {
                let sp = release.split('.').nth(1).filter(|m| *m != "0").map(|m| format!(" SP{m}")).unwrap_or_default();
                format!("SUSE:Linux Enterprise Server {major}{sp}")
            }
            ("rpm", "cbl-mariner" | "mariner" | "azurelinux") => format!("Azure Linux:{major}"),
            ("deb" | "apk" | "rpm", v) if !v.is_empty() => format!("{v}:{major}"),
            _ => return None,
        };
        let version = model::percent_decode(version);
        let version = match qualifier(purl, "epoch").filter(|e| *e != "0" && !e.is_empty()) {
            Some(e) if !version.contains(':') => format!("{e}:{version}"),
            _ => version,
        };
        return Some((eco, model::percent_decode(name), version));
    }
    model::parse_purl(purl).map(|(e, n, v)| (e.to_string(), n, v))
}

/// [`package_of`] of a component, an OS package keyed by its source package when the SBOM names it:
/// distributions' advisories are by source package (`openssl`, not `libssl3`).
pub fn package_of_component(c: &SbomComponent) -> Option<(String, String, String)> {
    let (eco, name, version) = package_of(&c.purl)?;
    if c.src_name.is_empty() || !model::language_of_ecosystem(&eco).is_empty() {
        return Some((eco, name, version));
    }
    let version = if c.src_version.is_empty() { version } else { c.src_version.clone() };
    Some((eco, c.src_name.clone(), version))
}

/// The distribution of an SBOM, from its OS packages' purls (`debian 12`, `amazon linux 2023`, `wolfi`).
pub fn os_of(components: &[SbomComponent]) -> String {
    components
        .iter()
        .find_map(|c| {
            let (eco, _, _) = package_of(&c.purl)?;
            let (name, release) = eco.split_once(':').unwrap_or((&eco, ""));
            model::language_of_ecosystem(name).is_empty().then(|| format!("{} {}", name.to_lowercase(), release.trim_end_matches(":LTS").trim_start_matches('v')).trim().to_string())
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
                let Some((eco, n, v)) = package_of_component(c) else { continue };
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
    fixed: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AdvisoryMeta {
    id: String,
    aliases: Vec<String>,
    summary: String,
    severity: String,
    score: Option<f32>,
    cwes: Vec<String>,
}

/// Which vulnerabilities to read (all by default): the database filters them, so the inbox and a remediation don't
/// read every vulnerability of the org.
#[derive(Default)]
pub struct VulnScope {
    /// Only these severities (empty: all).
    pub severities: Vec<String>,
    /// Only on running images (and hosts).
    pub running: bool,
    pub advisory: Option<String>,
}

/// The org's packages joined to the advisories the matching pass found for their versions
/// ([`code_security::match_due_versions`]): one indexed join, no version comparison per read.
const VULN_ROWS: &str = "SELECT e.id AS entity_id, e.kind, e.name AS entity_name, e.host, e.in_use, e.env, e.os, p.ecosystem, p.name, p.version, \
     (EXTRACT(EPOCH FROM p.first_seen) * 1000)::bigint AS first_seen, (EXTRACT(EPOCH FROM e.last_seen) * 1000)::bigint AS last_seen, m.advisory_id, m.fixed \
     FROM csm_sbom_entities e JOIN csm_packages p ON p.org_id = e.org_id AND p.entity_id = e.id \
     JOIN cs_version_matches m ON m.ecosystem = p.ecosystem AND m.package = p.name AND m.version = p.version JOIN cs_advisories a ON a.id = m.advisory_id \
     WHERE e.org_id = $1 AND e.last_seen > now() - make_interval(days => $2) AND a.withdrawn IS NULL \
     AND (cardinality($3::text[]) = 0 OR a.severity = ANY($3)) AND (NOT $4 OR e.in_use) AND ($5::text IS NULL OR m.advisory_id = $5)";

pub async fn infra_vulnerabilities(st: &AppState, org: &str) -> Result<Vec<InfraVulnerability>, ApiError> {
    infra_vulnerabilities_in(st, org, &VulnScope::default()).await
}

pub async fn infra_vulnerabilities_in(st: &AppState, org: &str, scope: &VulnScope) -> Result<Vec<InfraVulnerability>, ApiError> {
    let rows: Vec<VulnRow> = sqlx::query_as(VULN_ROWS).bind(org).bind(STALE_DAYS).bind(&scope.severities).bind(scope.running).bind(&scope.advisory).fetch_all(&st.postgres).await?;
    // Each advisory's text once, not once per package of every host and image.
    let ids: Vec<&str> = rows.iter().map(|r| r.advisory_id.as_str()).collect::<HashSet<_>>().into_iter().collect();
    let (meta, triage, kev) = tokio::try_join!(
        async { sqlx::query_as::<_, AdvisoryMeta>("SELECT id, aliases, summary, severity, score, cwes FROM cs_advisories WHERE id = ANY($1)").bind(&ids).fetch_all(&st.postgres).await.map_err(ApiError::from) },
        code_security::triage_map(st, org),
        async { sqlx::query_scalar::<_, String>("SELECT cve_id FROM security_kev").fetch_all(&st.postgres).await.map_err(ApiError::from) },
    )?;
    let meta: HashMap<String, AdvisoryMeta> = meta.into_iter().map(|a| (a.id.clone(), a)).collect();
    let kev: HashSet<String> = kev.into_iter().collect();
    Ok(infra_vulnerabilities_of(rows, &meta, &triage, &kev))
}

fn infra_vulnerabilities_of(rows: Vec<VulnRow>, meta: &HashMap<String, AdvisoryMeta>, triage: &HashMap<String, String>, kev: &HashSet<String>) -> Vec<InfraVulnerability> {
    let mut out: Vec<InfraVulnerability> = rows
        .into_iter()
        .filter_map(|r| {
            let a = meta.get(&r.advisory_id)?;
            let key = format!("csm-vuln:{}:{}:{}", r.advisory_id, r.entity_id, r.name);
            Some(InfraVulnerability {
                status: triage.get(&key).cloned().unwrap_or_else(|| "open".into()),
                key,
                cve: a.aliases.iter().find(|x| x.starts_with("CVE-")).cloned(),
                exploit_available: a.aliases.iter().any(|x| kev.contains(x)),
                aliases: a.aliases.clone(),
                title: if a.summary.is_empty() { r.advisory_id.clone() } else { a.summary.clone() },
                severity: a.severity.clone(),
                score: a.score.map(|s| (s as f64 * 10.0).round() / 10.0),
                cwes: a.cwes.clone(),
                fixed: r.fixed,
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
    out
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

/// A vulnerability's advisory, shared by its rows in [`InfraVulnerabilityList`].
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InfraAdvisory {
    pub advisory_id: String,
    pub cve: Option<String>,
    pub aliases: Vec<String>,
    pub title: String,
    pub severity: String,
    pub score: Option<f64>,
    pub cwes: Vec<String>,
    pub exploit_available: bool,
}

/// A host or container image, shared by its rows in [`InfraVulnerabilityList`].
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InfraResource {
    pub resource_id: String,
    pub resource_name: String,
    /// host | container_image
    pub resource_kind: String,
    pub host: String,
    pub env: String,
    pub os: String,
    pub running: bool,
    pub last_seen: i64,
}

/// A package of a resource affected by an advisory: indexes into the list's advisories and resources. Its triage key
/// is `csm-vuln:<advisory id>:<resource id>:<package>`.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InfraVulnerabilityRow {
    pub advisory: usize,
    pub resource: usize,
    pub ecosystem: String,
    pub package: String,
    pub version: String,
    pub fixed: Option<String>,
    /// open | muted
    pub status: String,
    pub first_seen: i64,
}

/// The vulnerabilities of hosts and images with each advisory and resource once: at hundreds of images the same
/// advisory text and image names would otherwise repeat in tens of thousands of rows.
#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct InfraVulnerabilityList {
    pub advisories: Vec<InfraAdvisory>,
    pub resources: Vec<InfraResource>,
    pub rows: Vec<InfraVulnerabilityRow>,
}

pub fn compact_vulnerabilities(list: Vec<InfraVulnerability>) -> InfraVulnerabilityList {
    let mut out = InfraVulnerabilityList::default();
    let (mut advisories, mut resources): (HashMap<String, usize>, HashMap<String, usize>) = (HashMap::new(), HashMap::new());
    for v in list {
        let advisory = *advisories.entry(v.advisory_id.clone()).or_insert_with(|| {
            out.advisories.push(InfraAdvisory {
                advisory_id: v.advisory_id.clone(),
                cve: v.cve.clone(),
                aliases: v.aliases.clone(),
                title: v.title.clone(),
                severity: v.severity.clone(),
                score: v.score,
                cwes: v.cwes.clone(),
                exploit_available: v.exploit_available,
            });
            out.advisories.len() - 1
        });
        let resource = *resources.entry(v.resource_id.clone()).or_insert_with(|| {
            out.resources.push(InfraResource {
                resource_id: v.resource_id.clone(),
                resource_name: v.resource_name.clone(),
                resource_kind: v.resource_kind.clone(),
                host: v.host.clone(),
                env: v.env.clone(),
                os: v.os.clone(),
                running: v.running,
                last_seen: v.last_seen,
            });
            out.resources.len() - 1
        });
        out.rows.push(InfraVulnerabilityRow { advisory, resource, ecosystem: v.ecosystem, package: v.package, version: v.version, fixed: v.fixed, status: v.status, first_seen: v.first_seen });
    }
    out
}

#[utoipa::path(post, path = "/api/v1/cloud-security/vulnerabilities", operation_id = "cloud_security_vulnerabilities", request_body = FindingStatusFilter,
    responses((status = 200, body = InfraVulnerabilityList)), tag = "cloud-security")]
pub async fn vulnerabilities(State(st): State<AppState>, ctx: AuthContext, Json(f): Json<FindingStatusFilter>) -> ApiResult<InfraVulnerabilityList> {
    Ok(Json(compact_vulnerabilities(infra_vulnerabilities(&st, &ctx.org_id).await?.into_iter().filter(|v| keep(&v.status, &f.status)).collect())))
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
    use crate::posture::store;
    // Resources missing the tags the org's tagging policies require (a failure there leaves the other rules shown).
    let tagging = async {
        Ok(super::tagging_policies::misconfig_gaps(st, org).await.unwrap_or_else(|e| {
            tracing::warn!(org = %org, error = %e, "evaluating tagging policies for misconfigurations failed");
            (0, vec![])
        }))
    };
    let (resources, manifests, triage, failing, checks, (evaluated, gaps)) = tokio::try_join!(
        catalog::load_all(st, org),
        manifests(st, org),
        code_security::triage_map(st, org),
        async { store::findings(&st.postgres, org, true).await.map_err(ApiError::from) },
        async { store::check_counts(&st.postgres, org).await.map_err(ApiError::from) },
        tagging,
    )?;
    let (mut findings, mut rules) = evaluate(&resources, &manifests, &triage);
    let (scanned, scanned_rules) = posture_misconfigurations(&failing, &checks);
    findings.extend(scanned);
    let (tagged, tag_rule) = tagging_misconfigurations(evaluated, &gaps, &triage);
    findings.extend(tagged);
    rules.push(tag_rule);
    findings.sort_by(|a, b| model::severity_rank(&b.severity).cmp(&model::severity_rank(&a.severity)).then(a.rule_id.cmp(&b.rule_id)).then(a.resource_name.cmp(&b.resource_name)));
    rules.extend(scanned_rules);
    Ok((findings, rules))
}

/// Resources failing the org's tagging policies as findings of the low-severity rule
/// [`super::tagging_policies::MISCONFIG_RULE`], with the rule's counts.
pub fn tagging_misconfigurations(evaluated: usize, gaps: &[super::tagging_rules::NonCompliantResource], triage: &HashMap<String, String>) -> (Vec<MisconfigFinding>, MisconfigRuleStatus) {
    use super::tagging_policies::{misconfig_detail, MISCONFIG_DESCRIPTION, MISCONFIG_NAME, MISCONFIG_REMEDIATION, MISCONFIG_RULE};
    let findings = gaps
        .iter()
        .map(|g| {
            let r = &g.resource;
            let key = format!("misconfig:{MISCONFIG_RULE}:{}", r.id);
            MisconfigFinding {
                status: triage.get(&key).cloned().unwrap_or_else(|| "open".into()),
                key,
                rule_id: MISCONFIG_RULE.into(),
                rule_name: MISCONFIG_NAME.into(),
                severity: "low".into(),
                description: MISCONFIG_DESCRIPTION.into(),
                remediation: MISCONFIG_REMEDIATION.into(),
                frameworks: vec![],
                detail: misconfig_detail(g),
                resource_id: r.id.clone(),
                resource_name: r.name.clone(),
                resource_type: r.resource_type.clone(),
                cluster: r.cluster.clone(),
                namespace: r.namespace.clone(),
                account: r.account.clone(),
                region: r.region.clone(),
                env: r.env.clone(),
                service: super::provenance::tag_values(&r.tags, "service").first().map(|v| v.to_string()),
                team: r.team.clone(),
                last_seen: 0,
            }
        })
        .collect();
    let rule = MisconfigRuleStatus {
        id: MISCONFIG_RULE.into(),
        name: MISCONFIG_NAME.into(),
        severity: "low".into(),
        description: MISCONFIG_DESCRIPTION.into(),
        remediation: MISCONFIG_REMEDIATION.into(),
        frameworks: vec![],
        evaluated,
        failing: gaps.len(),
    };
    (findings, rule)
}

/// The cloud account scans' results (`crate::posture`) as findings and rules of this page: the failing resources,
/// and per check how many resources it evaluated (counted by the database, passes aren't read).
pub fn posture_misconfigurations(failing: &[crate::posture::store::FindingRow], checks: &[crate::posture::store::CheckCounts]) -> (Vec<MisconfigFinding>, Vec<MisconfigRuleStatus>) {
    let tag = |r: &crate::posture::store::FindingRow, key: &str| {
        r.resource_tags.as_array().into_iter().flatten().filter_map(Value::as_str).find_map(|t| t.strip_prefix(key).and_then(|v| v.strip_prefix(':')).map(str::to_string))
    };
    let frameworks = |c: Option<&Value>| c.and_then(Value::as_object).map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let findings = failing
        .iter()
        .filter(|r| r.evaluation == "fail")
        .map(|r| MisconfigFinding {
            key: format!("posture:{}", r.id),
            rule_id: r.check_id.clone(),
            rule_name: r.title.clone().unwrap_or_else(|| r.check_id.clone()),
            severity: r.severity.clone(),
            description: r.description.clone().unwrap_or_default(),
            remediation: r.remediation.clone().unwrap_or_default(),
            frameworks: frameworks(r.compliance.as_ref()),
            detail: r.detail.clone(),
            resource_id: r.resource_uid.clone(),
            resource_name: r.resource_name.clone(),
            resource_type: r.resource_type.clone(),
            cluster: None,
            namespace: None,
            account: Some(r.account_id.clone()),
            region: Some(r.region.clone()).filter(|s| !s.is_empty()),
            env: tag(r, "env"),
            service: tag(r, "service"),
            team: tag(r, "team"),
            status: if r.muted { "muted" } else { "open" }.into(),
            last_seen: r.last_seen_at,
        })
        .collect();
    let rules = checks
        .iter()
        .map(|c| MisconfigRuleStatus {
            id: c.check_id.clone(),
            name: c.title.clone().unwrap_or_else(|| c.check_id.clone()),
            severity: c.severity.clone(),
            description: c.description.clone().unwrap_or_default(),
            remediation: c.remediation.clone().unwrap_or_default(),
            frameworks: frameworks(c.compliance.as_ref()),
            evaluated: c.evaluated as usize,
            failing: c.failing as usize,
        })
        .collect();
    (findings, rules)
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
    let all = catalog::load_all(&st, &ctx.org_id).await?;
    Ok(Json(categories(&st, &ctx.org_id, &all).await?))
}

pub async fn categories(st: &AppState, org: &str, all: &[CatalogResource]) -> Result<Vec<ResourceCategory>, ApiError> {
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
    for r in all {
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
    vulnerabilities: i64,
    /// 4 critical … 1 low, 0 unknown, NULL none.
    max_rank: Option<i32>,
}

const SEVERITY_RANK_SQL: &str = "CASE a.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 WHEN 'low' THEN 1 ELSE 0 END";

fn severity_of_rank(rank: i32) -> &'static str {
    match rank {
        4 => "critical",
        3 => "high",
        2 => "medium",
        1 => "low",
        _ => "unknown",
    }
}

#[utoipa::path(get, path = "/api/v1/cloud-security/packages", operation_id = "cloud_security_packages", responses((status = 200, body = Vec<InfraPackage>)), tag = "cloud-security")]
pub async fn packages(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<InfraPackage>> {
    // Counted in the database: each package version with its resources, then its matched advisories.
    let rows: Vec<PackageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "WITH pk AS (SELECT p.ecosystem, p.name, p.version, count(DISTINCT p.entity_id) AS resources, count(DISTINCT p.entity_id) FILTER (WHERE e.in_use) AS running, \
           (EXTRACT(EPOCH FROM min(p.first_seen)) * 1000)::bigint AS first_seen \
           FROM csm_packages p JOIN csm_sbom_entities e ON e.org_id = p.org_id AND e.id = p.entity_id \
           WHERE p.org_id = $1 AND e.last_seen > now() - make_interval(days => $2) GROUP BY p.ecosystem, p.name, p.version ORDER BY p.name, p.version LIMIT 20000) \
         SELECT pk.ecosystem, pk.name, pk.version, pk.resources, pk.running, pk.first_seen, count(a.id) AS vulnerabilities, max({SEVERITY_RANK_SQL}) FILTER (WHERE a.id IS NOT NULL) AS max_rank \
         FROM pk LEFT JOIN cs_version_matches m ON m.ecosystem = pk.ecosystem AND m.package = pk.name AND m.version = pk.version \
         LEFT JOIN cs_advisories a ON a.id = m.advisory_id AND a.withdrawn IS NULL \
         GROUP BY pk.ecosystem, pk.name, pk.version, pk.resources, pk.running, pk.first_seen ORDER BY pk.name, pk.version"
    )))
    .bind(&ctx.org_id)
    .bind(STALE_DAYS)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| InfraPackage {
                ecosystem: r.ecosystem,
                name: r.name,
                version: r.version,
                resources: r.resources as usize,
                running: r.running as usize,
                vulnerabilities: r.vulnerabilities as usize,
                max_severity: r.max_rank.map(|k| severity_of_rank(k).to_string()),
                first_seen: r.first_seen,
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
    // What affects the org, by advisory: hosts and images counted in the database.
    let (resources, libs) = tokio::try_join!(
        async {
            sqlx::query_as::<_, (String, i64)>(
                "SELECT m.advisory_id, count(DISTINCT e.id) FROM csm_sbom_entities e JOIN csm_packages p ON p.org_id = e.org_id AND p.entity_id = e.id \
                 JOIN cs_version_matches m ON m.ecosystem = p.ecosystem AND m.package = p.name AND m.version = p.version \
                 WHERE e.org_id = $1 AND e.last_seen > now() - make_interval(days => $2) GROUP BY m.advisory_id",
            )
            .bind(&ctx.org_id)
            .bind(STALE_DAYS)
            .fetch_all(&st.postgres)
            .await
            .map_err(ApiError::from)
        },
        code_security::library_vulnerabilities(&st, &ctx.org_id),
    )?;
    let resources: HashMap<String, usize> = resources.into_iter().map(|(id, n)| (id, n as usize)).collect();
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
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM cs_advisories a {filter}")))
        .bind(&text)
        .bind(&q.severity)
        .bind(&q.impact)
        .bind(&impacted)
        .bind(q.exploited)
        .bind(&kev)
        .fetch_one(&st.postgres)
        .await?;
    let rows: Vec<AdvisoryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT a.id, a.aliases, a.summary, a.severity, a.score, \
         ARRAY(SELECT p.ecosystem || ':' || p.package FROM cs_advisory_packages p WHERE p.advisory_id = a.id ORDER BY 1) AS packages, \
         (EXTRACT(EPOCH FROM a.published) * 1000)::bigint AS published FROM cs_advisories a {filter} \
         ORDER BY (a.id = ANY($4)) DESC, a.published DESC NULLS LAST, a.id LIMIT $7 OFFSET $8"
    )))
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
                impacted_resources: resources.get(&r.id).copied().unwrap_or(0),
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
    /// Hosts of the host list (Infrastructure).
    pub hosts: usize,
    /// Of those, the hosts that sent an SBOM in the last week.
    pub hosts_scanned: usize,
    pub images: usize,
    pub images_running: usize,
    pub clusters: usize,
    /// Kubernetes objects evaluated by the misconfiguration rules.
    pub resources_evaluated: usize,
    /// Rule × resource evaluations of every misconfiguration source (our rules, account scans, tagging policies).
    pub misconfiguration_checks: usize,
    /// The connected cloud accounts (AWS accounts, Azure subscriptions, GCP projects) and those only
    /// seen in the resource catalog.
    pub accounts: Vec<CloudAccount>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudAccount {
    /// aws | azure | gcp
    pub provider: String,
    /// AWS account id, Azure subscription id, GCP project id.
    pub id: String,
    /// The subscription's or project's display name (empty when unknown).
    pub name: String,
    /// Connected in Integrations (else only seen on resources).
    pub connected: bool,
    /// Its resources in the catalog.
    pub resources: usize,
}

/// The connected accounts (provider, id, name) with their resource counts from the catalog's (provider,
/// account) pairs, and the accounts only the catalog knows. Ids compare without case (Azure GUIDs).
pub fn cloud_accounts(connected: Vec<(String, String, String)>, seen: &[(String, String)]) -> Vec<CloudAccount> {
    let key = |p: &str, id: &str| (p.to_ascii_lowercase(), id.to_ascii_lowercase());
    let mut out: BTreeMap<(String, String), CloudAccount> = BTreeMap::new();
    for (provider, id, name) in connected {
        let e = out.entry(key(&provider, &id)).or_insert_with(|| CloudAccount { provider: provider.to_ascii_lowercase(), id: id.clone(), name: String::new(), connected: true, resources: 0 });
        if e.name.is_empty() {
            e.name = name;
        }
    }
    for (provider, id) in seen {
        out.entry(key(provider, id)).or_insert_with(|| CloudAccount { provider: provider.to_ascii_lowercase(), id: id.clone(), name: String::new(), connected: false, resources: 0 }).resources += 1;
    }
    out.into_values().collect()
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

/// How many of the listed hosts sent an SBOM. Both sides count hosts of the host list: SBOMs of hosts
/// gone since (nodes replaced during the week) don't count. Names compare without case and domain.
pub fn hosts_scanned(listed: &[&str], sbom_hosts: &[String]) -> usize {
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_ascii_lowercase();
    let sent: HashSet<String> = sbom_hosts.iter().map(|h| short(h)).collect();
    listed.iter().map(|h| short(h)).collect::<HashSet<_>>().iter().filter(|h| sent.contains(*h)).count()
}

#[derive(sqlx::FromRow, Default)]
struct VulnCounts {
    total: i64,
    running: i64,
    exploit_available: i64,
    fix_available: i64,
    high_or_critical: i64,
    high_exploit_risk: i64,
    critical: i64,
    high: i64,
    medium: i64,
    low: i64,
    unknown: i64,
}

/// The open vulnerabilities of hosts and images counted in the database: unique advisories per prioritization
/// criterion, and (advisory, resource, package) occurrences per severity. Muted ones (Code Security's triage) are left out.
async fn vulnerability_counts(st: &AppState, org: &str) -> Result<VulnCounts, ApiError> {
    let kev: Vec<String> = sqlx::query_scalar("SELECT cve_id FROM security_kev").fetch_all(&st.postgres).await?;
    let high = "a.severity IN ('critical', 'high')";
    let exploited = "a.aliases && $3::text[]";
    let sql = format!(
        "SELECT count(DISTINCT m.advisory_id) AS total, count(DISTINCT m.advisory_id) FILTER (WHERE e.in_use) AS running, \
         count(DISTINCT m.advisory_id) FILTER (WHERE {exploited}) AS exploit_available, count(DISTINCT m.advisory_id) FILTER (WHERE m.fixed IS NOT NULL) AS fix_available, \
         count(DISTINCT m.advisory_id) FILTER (WHERE {high}) AS high_or_critical, count(DISTINCT m.advisory_id) FILTER (WHERE e.in_use AND {exploited} AND {high}) AS high_exploit_risk, \
         count(*) FILTER (WHERE a.severity = 'critical') AS critical, count(*) FILTER (WHERE a.severity = 'high') AS high, count(*) FILTER (WHERE a.severity = 'medium') AS medium, \
         count(*) FILTER (WHERE a.severity = 'low') AS low, count(*) FILTER (WHERE a.severity NOT IN ('critical', 'high', 'medium', 'low')) AS unknown \
         FROM csm_sbom_entities e JOIN csm_packages p ON p.org_id = e.org_id AND p.entity_id = e.id \
         JOIN cs_version_matches m ON m.ecosystem = p.ecosystem AND m.package = p.name AND m.version = p.version JOIN cs_advisories a ON a.id = m.advisory_id \
         LEFT JOIN cs_triage t ON t.org_id = e.org_id AND t.key = 'csm-vuln:' || m.advisory_id || ':' || e.id || ':' || p.name \
         WHERE e.org_id = $1 AND e.last_seen > now() - make_interval(days => $2) AND a.withdrawn IS NULL AND COALESCE(t.status, 'open') = 'open'"
    );
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(org).bind(STALE_DAYS).bind(&kev).fetch_one(&st.postgres).await?)
}

/// Open misconfigurations by severity, and how many rule × resource evaluations there were, over every source the
/// Findings page shows (our rules, the account scans' checks, the tagging policies).
pub fn misconfiguration_summary(findings: &[MisconfigFinding], rules: &[MisconfigRuleStatus]) -> (BTreeMap<String, usize>, usize) {
    let mut by_severity: BTreeMap<String, usize> = ["critical", "high", "medium", "low"].iter().map(|s| (s.to_string(), 0)).collect();
    for f in findings.iter().filter(|f| f.status == "open") {
        *by_severity.entry(f.severity.clone()).or_default() += 1;
    }
    (by_severity, rules.iter().map(|r| r.evaluated).sum())
}

#[utoipa::path(get, path = "/api/v1/cloud-security/summary", operation_id = "cloud_security_summary", responses((status = 200, body = CloudSecuritySummary)), tag = "cloud-security")]
pub async fn summary(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<CloudSecuritySummary> {
    let org = &ctx.org_id;
    let pg = &st.postgres;
    let (counts, resources, manifests, (findings, rules), (images, images_running), sbom_hosts, connected) = tokio::try_join!(
        vulnerability_counts(&st, org),
        catalog::load_all(&st, org),
        manifests(&st, org),
        misconfigurations_of(&st, org),
        async {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT count(*) FILTER (WHERE kind = 'container_image'), count(*) FILTER (WHERE kind = 'container_image' AND in_use) \
                 FROM csm_sbom_entities WHERE org_id = $1 AND last_seen > now() - make_interval(days => $2)",
            )
            .bind(org)
            .bind(STALE_DAYS)
            .fetch_one(pg)
            .await
            .map_err(ApiError::from)
        },
        async {
            sqlx::query_scalar::<_, String>("SELECT id FROM csm_sbom_entities WHERE org_id = $1 AND kind = 'host' AND last_seen > now() - make_interval(days => $2)")
                .bind(org)
                .bind(STALE_DAYS)
                .fetch_all(pg)
                .await
                .map_err(ApiError::from)
        },
        async {
            sqlx::query_as::<_, (String, String, String)>(
                "SELECT 'aws', aws_account_id, '' FROM aws_accounts WHERE org_id = $1 \
                 UNION SELECT j.provider, j.scope, COALESCE(n.name, '') FROM cloud_crawl_jobs j \
                 LEFT JOIN cloud_scope_names n ON n.org_id = j.org_id AND n.provider = j.provider AND n.scope = j.scope \
                 WHERE j.org_id = $1 AND j.provider IN ('azure', 'gcp') AND j.scope <> ''",
            )
            .bind(org)
            .fetch_all(pg)
            .await
            .map_err(ApiError::from)
        },
    )?;
    let prioritization = VulnerabilityPrioritization {
        total: counts.total as usize,
        running: counts.running as usize,
        exploit_available: counts.exploit_available as usize,
        fix_available: counts.fix_available as usize,
        high_or_critical: counts.high_or_critical as usize,
        high_exploit_risk: counts.high_exploit_risk as usize,
    };
    let mut by_severity: BTreeMap<String, usize> = [("critical", counts.critical), ("high", counts.high), ("medium", counts.medium), ("low", counts.low)].iter().map(|(s, n)| (s.to_string(), *n as usize)).collect();
    if counts.unknown > 0 {
        by_severity.insert("unknown".into(), counts.unknown as usize);
    }
    // The Findings page's misconfigurations: our rules, the account scans and the tagging policies.
    let (misconfigs, misconfiguration_checks) = misconfiguration_summary(&findings, &rules);
    let listed: Vec<&str> = resources.iter().filter(|r| r.resource_type == CatalogType::Host).map(|r| r.name.as_str()).collect();
    let seen: Vec<(String, String)> = resources.iter().filter_map(|r| Some((r.cloud.clone()?, r.account.clone()?))).collect();
    let clusters: BTreeSet<&str> = resources.iter().filter_map(|r| r.cluster.as_deref()).collect();
    Ok(Json(CloudSecuritySummary {
        vulnerabilities: prioritization,
        coverage: CloudCoverage {
            hosts: listed.len(),
            hosts_scanned: hosts_scanned(&listed, &sbom_hosts),
            images: images as usize,
            images_running: images_running as usize,
            clusters: clusters.len(),
            resources_evaluated: resources.iter().filter(|r| target_of(r).is_some() && (r.resource_type == CatalogType::Host || manifests.contains_key(&r.id))).count(),
            misconfiguration_checks,
            accounts: cloud_accounts(connected, &seen),
        },
        misconfigurations: misconfigs,
        vulnerabilities_by_severity: by_severity,
        inventory: categories(&st, org, &resources).await?,
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
    fn resources_missing_required_tags_are_low_severity_findings_with_their_triage() {
        use super::super::provenance::{TagTarget, TaggedResource};
        use super::super::tagging_rules::{NonCompliantResource, PolicyGaps, TagGap};
        let resource = TaggedResource {
            id: "arn:aws:s3:::example-shop-assets".into(),
            target: TagTarget::Cloud,
            name: "example-shop-assets".into(),
            resource_type: "AwsS3Bucket".into(),
            provider: Some("aws".into()),
            account: Some("123456789012".into()),
            region: Some("eu-west-1".into()),
            cluster: None,
            namespace: None,
            env: Some("prod".into()),
            team: None,
            tags: vec!["env:prod".into(), "service:storefront-api".into()],
            provenance: None,
        };
        let gaps = vec![NonCompliantResource { resource, policies: vec![PolicyGaps { policy_id: "p".into(), policy_name: "Owners".into(), gaps: vec![TagGap { key: "team".into(), value: None }] }] }];
        let triage = HashMap::from([("misconfig:ocs-tag-001:arn:aws:s3:::example-shop-assets".to_string(), "muted".to_string())]);
        let (findings, rule) = tagging_misconfigurations(4, &gaps, &triage);
        assert_eq!((rule.id.as_str(), rule.severity.as_str(), rule.evaluated, rule.failing), ("ocs-tag-001", "low", 4, 1));
        let f = &findings[0];
        assert_eq!((f.rule_name.as_str(), f.detail.as_str(), f.status.as_str()), ("Resource missing required tags", "team missing (Owners)", "muted"));
        assert_eq!((f.account.as_deref(), f.service.as_deref(), f.resource_type.as_str()), (Some("123456789012"), Some("storefront-api"), "AwsS3Bucket"));
    }

    #[test]
    fn purls_map_to_osv_ecosystems() {
        assert_eq!(package_of("pkg:deb/debian/libexample-ssl3@3.0.11-1~deb12u2?arch=amd64&distro=debian-12").unwrap(), ("Debian:12".into(), "libexample-ssl3".into(), "3.0.11-1~deb12u2".into()));
        assert_eq!(package_of("pkg:deb/ubuntu/example-sudo@1.9.9-1ubuntu2.4?distro=ubuntu-22.04").unwrap().0, "Ubuntu:22.04:LTS");
        assert_eq!(package_of("pkg:deb/ubuntu/x@1?distro=ubuntu-23.10").unwrap().0, "Ubuntu:23.10");
        assert_eq!(package_of("pkg:apk/alpine/example-musl@1.2.4-r2?arch=x86_64&distro=3.19.1").unwrap(), ("Alpine:v3.19".into(), "example-musl".into(), "1.2.4-r2".into()));
        assert_eq!(package_of("pkg:maven/com.example.json/json-core@2.15.0").unwrap().0, "Maven");
        let comps = vec![SbomComponent { version: "1".into(), purl: "pkg:deb/debian/x@1?distro=debian-12".into(), ..Default::default() }];
        assert_eq!(os_of(&comps), "debian 12");
    }

    #[test]
    fn os_purls_keep_epochs_rpm_and_wolfi() {
        let p = |purl: &str| package_of(purl).unwrap();
        // Trivy puts a non-zero epoch in a qualifier; advisories compare `epoch:version-release`.
        assert_eq!(p("pkg:deb/debian/libpam0g@1.5.2-6%2Bdeb12u1?arch=amd64&distro=debian-12.5&epoch=1").2, "1:1.5.2-6+deb12u1");
        assert_eq!(p("pkg:deb/debian/libssl3@3.0.11-1%7Edeb12u2?arch=amd64&distro=debian-12.5&epoch=0").2, "3.0.11-1~deb12u2");
        assert_eq!(p("pkg:rpm/rocky/openssl-libs@3.0.7-27.el9?arch=x86_64&distro=rocky-9.3&epoch=1"), ("Rocky Linux:9".into(), "openssl-libs".into(), "1:3.0.7-27.el9".into()));
        assert_eq!(p("pkg:rpm/alma/glibc@2.34-100.el9?arch=x86_64&distro=alma-9.4").0, "AlmaLinux:9");
        assert_eq!(p("pkg:rpm/redhat/openssl@3.0.7-27.el9?distro=redhat-9.3").0, "Red Hat:9");
        assert_eq!(p("pkg:rpm/amazon/curl@8.5.0-1.amzn2023.0.2?distro=amazon-2023.3.20240219").0, "Amazon Linux:2023");
        assert_eq!(p("pkg:rpm/centos/bash@4.2.46-35.el7_9?distro=centos-7.9.2009").0, "CentOS:7");
        assert_eq!(p("pkg:rpm/opensuse.leap/zlib@1.2.13-150500.4.3.1?distro=opensuse.leap-15.5").0, "openSUSE:Leap 15.5");
        assert_eq!(p("pkg:apk/wolfi/glibc@2.38-r10?arch=x86_64&distro=20230201"), ("Wolfi".into(), "glibc".into(), "2.38-r10".into()));
        assert_eq!(p("pkg:apk/chainguard/openssl@3.3.0-r8?distro=20230214").0, "Chainguard");
        // Only ecosystems OSV knows are looked up there.
        assert!(model::osv_queryable("Debian:12") && model::osv_queryable("Wolfi") && model::osv_queryable("PyPI") && model::osv_queryable("Rocky Linux:9"));
        assert!(!model::osv_queryable("Amazon Linux:2023") && !model::osv_queryable("CentOS:7"));
        let os = |purl: &str| os_of(&[SbomComponent { purl: purl.into(), ..Default::default() }]);
        assert_eq!(os("pkg:rpm/amazon/curl@8.5.0-1.amzn2023?distro=amazon-2023.3"), "amazon linux 2023");
        assert_eq!(os("pkg:apk/wolfi/glibc@2.38-r10?distro=20230201"), "wolfi");
        assert_eq!(os("pkg:deb/ubuntu/x@1?distro=ubuntu-22.04"), "ubuntu 22.04");
        assert_eq!(os("pkg:pypi/requests@2.31.0"), "");
    }

    #[test]
    fn os_packages_are_keyed_by_their_source_package() {
        let c = SbomComponent {
            version: "3.0.11-1~deb12u2".into(),
            purl: "pkg:deb/debian/libssl3@3.0.11-1~deb12u2?arch=amd64&distro=debian-12.5".into(),
            src_name: "openssl".into(),
            src_version: "3.0.11-1~deb12u2".into(),
        };
        assert_eq!(package_of_component(&c).unwrap(), ("Debian:12".into(), "openssl".into(), "3.0.11-1~deb12u2".into()));
        // No source version: the binary's.
        let c2 = SbomComponent { src_version: String::new(), ..c };
        assert_eq!(package_of_component(&c2).unwrap().2, "3.0.11-1~deb12u2");
        // Language packages keep their own name.
        let lib = SbomComponent { purl: "pkg:pypi/requests@2.31.0".into(), src_name: "x".into(), ..Default::default() };
        assert_eq!(package_of_component(&lib).unwrap().1, "requests");
        // Through the upload's JSON.
        let up: SbomUpload = serde_json::from_value(serde_json::json!({ "host": "h", "entities": [{ "kind": "host", "id": "h", "components": [
            { "version": "1.2.4-r2", "purl": "pkg:apk/alpine/musl-utils@1.2.4-r2?distro=3.19.1", "srcName": "musl", "srcVersion": "1.2.4-r2" }] }] }))
        .unwrap();
        assert_eq!(package_of_component(&up.entities[0].components[0]).unwrap(), ("Alpine:v3.19".into(), "musl".into(), "1.2.4-r2".into()));
    }

    #[test]
    fn host_coverage_counts_listed_hosts_only() {
        // Hosts gone from the host list (nodes replaced during the week) still have SBOMs: they don't count.
        let listed = ["i-0a1b2c3d4e5f60001", "i-0a1b2c3d4e5f60002", "ip-10-0-1-7.ec2.internal"];
        let sboms: Vec<String> = ["i-0a1b2c3d4e5f60001", "IP-10-0-1-7", "i-0a1b2c3d4e5f60009", "i-0a1b2c3d4e5f60010"].iter().map(|s| s.to_string()).collect();
        assert_eq!(hosts_scanned(&listed, &sboms), 2);
        assert_eq!(hosts_scanned(&listed, &[]), 0);
        assert_eq!(hosts_scanned(&[], &sboms), 0);
    }

    #[test]
    fn cloud_accounts_list_every_connected_account() {
        let connected = vec![
            ("aws".to_string(), "123456789012".to_string(), String::new()),
            ("azure".to_string(), "0b1c2d3e-0000-4000-8000-000000000001".to_string(), "Storefront Production".to_string()),
            ("gcp".to_string(), "example-storefront".to_string(), "Example Storefront".to_string()),
        ];
        // The catalog's resources count per account; an account only seen there (an Agent's cloud tags) is listed too.
        let seen = vec![
            ("aws".to_string(), "123456789012".to_string()),
            ("aws".to_string(), "123456789012".to_string()),
            ("azure".to_string(), "0B1C2D3E-0000-4000-8000-000000000001".to_string()),
            ("aws".to_string(), "210987654321".to_string()),
        ];
        let a = cloud_accounts(connected, &seen);
        let rows: Vec<(&str, &str, &str, bool, usize)> = a.iter().map(|a| (a.provider.as_str(), a.id.as_str(), a.name.as_str(), a.connected, a.resources)).collect();
        assert_eq!(
            rows,
            [
                ("aws", "123456789012", "", true, 2),
                ("aws", "210987654321", "", false, 1),
                ("azure", "0b1c2d3e-0000-4000-8000-000000000001", "Storefront Production", true, 1),
                ("gcp", "example-storefront", "Example Storefront", true, 0),
            ]
        );
    }

    #[test]
    fn account_scans_list_failing_resources_and_count_each_check_from_its_totals() {
        use crate::posture::store::{CheckCounts, FindingRow};
        let failing = FindingRow {
            id: "f1".into(),
            provider: "aws".into(),
            account_id: "123456789012".into(),
            check_id: "s3_bucket_public_access".into(),
            title: Some("S3 bucket is public".into()),
            description: None,
            risk: None,
            remediation: None,
            service: None,
            compliance: Some(serde_json::json!({"CIS-2.0": ["2.1.4"]})),
            reference_urls: None,
            resource_uid: "arn:aws:s3:::example-shop-assets".into(),
            resource_name: "example-shop-assets".into(),
            resource_type: "AwsS3Bucket".into(),
            region: "eu-west-1".into(),
            evaluation: "fail".into(),
            severity: "high".into(),
            detail: "public".into(),
            resource_tags: serde_json::json!(["env:prod", "team:storefront"]),
            first_seen_at: 1,
            last_seen_at: 2,
            evaluation_changed_at: 2,
            muted: false,
            mute_reason: None,
            mute_description: None,
            mute_expire_at: None,
            muted_by: None,
            muted_at: None,
        };
        let check = |id: &str, evaluated: i64, failing: i64| CheckCounts { check_id: id.into(), title: None, severity: "medium".into(), evaluated, failing, ..Default::default() };
        let (findings, rules) = posture_misconfigurations(&[failing], &[check("s3_bucket_public_access", 40, 1), check("iam_root_mfa", 1, 0)]);
        assert_eq!(findings.iter().map(|f| (f.key.as_str(), f.rule_name.as_str(), f.env.as_deref(), f.team.as_deref())).collect::<Vec<_>>(), vec![("posture:f1", "S3 bucket is public", Some("prod"), Some("storefront"))]);
        assert_eq!(findings[0].frameworks, vec!["CIS-2.0"]);
        // Checks that only passed are listed too, named by their id when the scanner sent no title.
        assert_eq!(rules.iter().map(|r| (r.id.as_str(), r.name.as_str(), r.evaluated, r.failing)).collect::<Vec<_>>(), vec![("s3_bucket_public_access", "s3_bucket_public_access", 40, 1), ("iam_root_mfa", "iam_root_mfa", 1, 0)]);
    }

    #[test]
    fn the_summary_counts_open_misconfigurations_and_checks_of_every_source() {
        let finding = |rule: &str, severity: &str, status: &str| MisconfigFinding {
            key: format!("misconfig:{rule}"),
            rule_id: rule.into(),
            rule_name: rule.into(),
            severity: severity.into(),
            description: String::new(),
            remediation: String::new(),
            frameworks: vec![],
            detail: String::new(),
            resource_id: "r".into(),
            resource_name: "r".into(),
            resource_type: "AwsS3Bucket".into(),
            cluster: None,
            namespace: None,
            account: None,
            region: None,
            env: None,
            service: None,
            team: None,
            status: status.into(),
            last_seen: 0,
        };
        let rule = |id: &str, evaluated: usize| MisconfigRuleStatus { id: id.into(), name: id.into(), severity: "high".into(), description: String::new(), remediation: String::new(), frameworks: vec![], evaluated, failing: 0 };
        // A Kubernetes rule, an account scan's check and the tagging rule; the muted one isn't counted.
        let findings = [finding("ocs-k8s-001", "high", "open"), finding("s3_bucket_public_access", "critical", "open"), finding("ocs-tag-001", "low", "open"), finding("ocs-k8s-002", "medium", "muted")];
        let (by_severity, checks) = misconfiguration_summary(&findings, &[rule("ocs-k8s-001", 12), rule("s3_bucket_public_access", 40), rule("ocs-tag-001", 0)]);
        assert_eq!(by_severity, BTreeMap::from([("critical".into(), 1), ("high".into(), 1), ("medium".into(), 0), ("low".into(), 1)]));
        assert_eq!(checks, 52);
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
        // Read from the matching pass's results.
        while code_security::match_due_versions(&st.postgres).await.unwrap() > 0 {}
        let v = infra_vulnerabilities(st, &org).await.unwrap();
        let ids: BTreeSet<&str> = v.iter().map(|x| x.advisory_id.as_str()).collect();
        assert_eq!(ids, ["OCSA-DEMO-2026-0101", "OCSA-DEMO-2026-0102", "OCSA-DEMO-2026-0105"].into_iter().collect());
        // The summary counts the same in the database; the list carries each advisory and resource once.
        let c = vulnerability_counts(st, &org).await.unwrap();
        assert_eq!((c.total, c.running, c.fix_available), (3, 3, 3));
        assert_eq!(c.critical + c.high + c.medium + c.low + c.unknown, 3);
        let list = compact_vulnerabilities(v.clone());
        assert_eq!((list.advisories.len(), list.resources.len(), list.rows.len()), (3, 2, 3));
        let scoped = infra_vulnerabilities_in(st, &org, &VulnScope { advisory: Some("OCSA-DEMO-2026-0105".into()), ..Default::default() }).await.unwrap();
        assert_eq!(scoped.iter().map(|x| x.package.as_str()).collect::<Vec<_>>(), vec!["example-musl"]);
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
