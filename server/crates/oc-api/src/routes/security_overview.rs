//! Security → Overview and Security → Inbox.
//!
//! - **Overview**: what is covered, from development to runtime: repositories, services and
//!   endpoints, hosts, log sources; per product (Code Security, Cloud SIEM, App & API Protection)
//!   whether it is set up and what it found.
//! - **Inbox**: the findings worth fixing first: critical and high library vulnerabilities loaded in
//!   production or known to be exploited (CISA's catalog), critical and high code vulnerabilities and
//!   secrets in repositories, and (Cloud Security) critical and high misconfigurations and the
//!   vulnerabilities of running hosts and images, still open. Muting is Code Security's triage.

use std::collections::{BTreeMap, HashSet};

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use super::code_security::{self, CsFilter, FindingFilter};
use super::logs::TimeRange;
use super::security_siem::pack_statuses;
use super::security_signals::signals_in;
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::{AppState, OrgId};

const DAY: i64 = 86_400_000;

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeSecurityCoverage {
    /// Static code analysis: repositories send SARIF scans.
    pub sast: bool,
    /// Software composition analysis of repositories (SBOM uploads).
    pub sca_static: bool,
    pub iac: bool,
    pub secrets: bool,
    /// Runtime code analysis (tracers report vulnerable code paths).
    pub iast: bool,
    /// Libraries loaded by running services (tracer telemetry).
    pub sca_runtime: bool,
    pub static_code_vulnerabilities: usize,
    pub iac_findings: usize,
    pub secrets_found: usize,
    pub static_library_vulnerabilities: usize,
    /// Library vulnerabilities listed as exploited in the wild.
    pub with_exploit: usize,
    pub runtime_code_vulnerabilities: usize,
    pub runtime_library_vulnerabilities: usize,
    pub runtime_libraries_in_production: usize,
    /// Runtime library vulnerabilities in services that attack signals targeted.
    pub exposed_to_attacks: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityOverview {
    pub repositories: usize,
    pub services: u64,
    pub endpoints: u64,
    pub hosts: u64,
    /// Log sources of Cloud SIEM's content packs sending logs.
    pub log_sources: usize,
    pub code_security: CodeSecurityCoverage,
    /// Cloud SIEM: content packs on, and open high or critical signals.
    pub siem_packs_enabled: usize,
    pub important_signals: usize,
    /// App & API Protection signals over the last 7 days, per day `[epoch ms, count]`.
    pub app_sec_signals: Vec<(i64, usize)>,
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct InventoryRow {
    services: u64,
    endpoints: u64,
    hosts: u64,
}

#[utoipa::path(get, path = "/api/v1/security/overview", operation_id = "security_overview", responses((status = 200, body = SecurityOverview)), tag = "security")]
pub async fn overview(State(st): State<AppState>, ctx: AuthContext, access: LogAccess) -> ApiResult<SecurityOverview> {
    let org = ctx.org_id.clone();
    let now = chrono::Utc::now().timestamp_millis();
    let pg = &st.postgres;
    // Independent reads, at once; library vulnerabilities are read once (the matching pass precomputed them).
    let (setup, summary, repositories, inv, agent_hosts, packs, signals, vulns, kev) = tokio::try_join!(
        async { Ok::<_, ApiError>(code_security::setup(State(st.clone()), OrgId(org.clone())).await?.0) },
        async { Ok::<_, ApiError>(code_security::summary(State(st.clone()), OrgId(org.clone())).await?.0) },
        async {
            sqlx::query_scalar::<_, i64>("SELECT count(DISTINCT repository) FROM cs_scans WHERE org_id = $1 AND repository <> '' AND pr_number = ''").bind(&org).fetch_one(pg).await.map_err(ApiError::from)
        },
        async {
            st.ch(&org)
                .query(
                    "SELECT uniqExact(ServiceName) AS services, uniqExactIf(concat(ServiceName, ' ', SpanName), SpanKind = 'Server') AS endpoints, \
                     uniqExactIf(ResourceAttributes['host.name'], ResourceAttributes['host.name'] != '') AS hosts \
                     FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND Timestamp > now() - INTERVAL 1 HOUR",
                )
                .fetch_one::<InventoryRow>()
                .await
                .map_err(ApiError::from)
        },
        async { sqlx::query_scalar::<_, i64>("SELECT count(*) FROM datadog_hosts WHERE org_id = $1").bind(&org).fetch_one(pg).await.map_err(ApiError::from) },
        pack_statuses(&st, &org, &access, now),
        signals_in(&st, &org, TimeRange { from: now - 7 * DAY, to: now }),
        code_security::library_vulnerabilities(&st, &org),
        async { sqlx::query_scalar::<_, String>("SELECT cve_id FROM security_kev").fetch_all(pg).await.map_err(ApiError::from) },
    )?;
    let total = |c: &code_security::SeverityCounts| c.critical + c.high + c.medium + c.low + c.unknown;
    // Open library vulnerabilities, to find the exploited ones and those in attacked services.
    let vulns: Vec<&code_security::LibraryVulnerability> = vulns.iter().filter(|v| v.status == "open").collect();
    let kev: HashSet<String> = kev.into_iter().collect();
    let attacked: HashSet<&str> = signals.iter().filter(|s| s.tags.iter().any(|t| t.starts_with("tactic:"))).flat_map(|s| s.services.iter().map(String::as_str)).collect();
    let code = CodeSecurityCoverage {
        sast: setup.repositories_with_code_scans > 0,
        sca_static: setup.repositories_with_libraries > 0,
        iac: setup.repositories_with_code_scans > 0,
        secrets: setup.repositories_with_code_scans > 0,
        iast: setup.iast_services > 0,
        sca_runtime: setup.runtime_services > 0,
        static_code_vulnerabilities: total(&summary.code),
        iac_findings: total(&summary.iac),
        secrets_found: total(&summary.secrets),
        static_library_vulnerabilities: summary.in_repositories,
        with_exploit: vulns.iter().filter(|v| v.cve.iter().chain(v.aliases.iter()).any(|c| kev.contains(c))).count(),
        runtime_code_vulnerabilities: total(&summary.runtime_code),
        runtime_library_vulnerabilities: vulns.iter().filter(|v| !v.services.is_empty()).count(),
        runtime_libraries_in_production: summary.in_production,
        exposed_to_attacks: vulns.iter().filter(|v| v.in_production && v.services.iter().any(|s| attacked.contains(s.as_str()))).count(),
    };
    let start = now - now.rem_euclid(DAY) - 6 * DAY;
    let mut app_sec: Vec<(i64, usize)> = (0..7).map(|i| (start + i * DAY, 0)).collect();
    for s in signals.iter().filter(|s| matches!(s.rule_type.as_str(), "application_security" | "api_security")) {
        let i = ((s.created_at - start) / DAY) as usize;
        if let Some(b) = app_sec.get_mut(i) {
            b.1 += 1;
        }
    }
    Ok(Json(SecurityOverview {
        repositories: (repositories as usize).max(setup.repositories_with_code_scans).max(setup.repositories_with_libraries),
        services: inv.services,
        endpoints: inv.endpoints,
        hosts: inv.hosts.max(agent_hosts as u64),
        log_sources: packs.iter().filter(|p| p.logs24h > 0).count(),
        code_security: code,
        siem_packs_enabled: packs.iter().filter(|p| p.enabled).count(),
        important_signals: signals.iter().filter(|s| s.state == "open" && matches!(s.severity.as_str(), "high" | "critical")).count(),
        app_sec_signals: app_sec,
    }))
}

// ================================================================ inbox

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InboxFinding {
    /// Code Security's triage key.
    pub key: String,
    pub title: String,
    /// Library Vulnerability | Code Vulnerability | Secret | IaC Misconfiguration
    pub finding_type: String,
    pub severity: String,
    /// Service | Repository
    pub resource_type: String,
    pub resources: Vec<String>,
    pub services: Vec<String>,
    pub repositories: Vec<String>,
    /// Why it is in the inbox.
    pub reasons: Vec<String>,
    pub first_seen: i64,
    /// pending | in_flight
    pub triage: String,
    pub cve: Option<String>,
    /// Where to open it.
    pub link: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityInbox {
    pub findings: Vec<InboxFinding>,
    /// Critical and high findings now and a week ago (first seen before then).
    pub critical: usize,
    pub high: usize,
    pub critical_week_ago: usize,
    pub high_week_ago: usize,
    pub by_type: BTreeMap<String, usize>,
}

#[utoipa::path(get, path = "/api/v1/security/inbox", operation_id = "security_inbox", responses((status = 200, body = SecurityInbox)), tag = "security")]
pub async fn inbox(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<SecurityInbox> {
    let findings = inbox_findings(&st, &ctx.org_id).await?;
    let week_ago = chrono::Utc::now().timestamp_millis() - 7 * DAY;
    let count = |sev: &str, before: Option<i64>| findings.iter().filter(|f| f.severity == sev && before.is_none_or(|t| f.first_seen <= t)).count();
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for f in &findings {
        *by_type.entry(f.finding_type.clone()).or_default() += 1;
    }
    Ok(Json(SecurityInbox {
        critical: count("critical", None),
        high: count("high", None),
        critical_week_ago: count("critical", Some(week_ago)),
        high_week_ago: count("high", Some(week_ago)),
        by_type,
        findings,
    }))
}

/// A vulnerability's title naming its package once: `<title> in <package>` unless the title starts with it.
fn titled(title: &str, package: &str) -> String {
    if title.strip_prefix(package).is_some_and(|rest| rest.starts_with(':')) {
        title.to_string()
    } else {
        format!("{title} in {package}")
    }
}

/// Resources missing the tags the org's tagging policies require, as one low-severity item (listed one by one they
/// would bury the severe findings); it opens the tag coverage page.
pub fn tagging_inbox_item(misconfigs: &[super::cloud_security::MisconfigFinding]) -> Option<InboxFinding> {
    use super::tagging_policies::{MISCONFIG_NAME, MISCONFIG_RULE};
    const MAX_RESOURCES: usize = 20;
    let open: Vec<&super::cloud_security::MisconfigFinding> = misconfigs.iter().filter(|m| m.rule_id == MISCONFIG_RULE && m.status == "open").collect();
    if open.is_empty() {
        return None;
    }
    let mut services: Vec<String> = open.iter().filter_map(|m| m.service.clone()).collect();
    services.sort();
    services.dedup();
    Some(InboxFinding {
        key: format!("misconfig:{MISCONFIG_RULE}"),
        title: MISCONFIG_NAME.into(),
        finding_type: "Misconfiguration".into(),
        severity: "low".into(),
        resource_type: "Resource".into(),
        resources: open.iter().take(MAX_RESOURCES).map(|m| m.resource_name.clone()).collect(),
        services,
        repositories: vec![],
        reasons: vec![format!("{} resource{} lack tags the tagging policies require", open.len(), if open.len() == 1 { "" } else { "s" })],
        first_seen: 0,
        triage: "pending".into(),
        cve: None,
        link: "/infrastructure/tagging".into(),
    })
}

pub async fn inbox_findings(st: &AppState, org: &str) -> Result<Vec<InboxFinding>, ApiError> {
    let severe = vec!["critical".to_string(), "high".to_string()];
    let findings = |kind: &str| {
        let filter = FindingFilter { kind: kind.into(), filter: CsFilter { severity: if kind == "secret" { vec![] } else { severe.clone() }, limit: 5000, ..Default::default() } };
        async move { Ok::<_, ApiError>(code_security::list_findings(State(st.clone()), OrgId(org.to_string()), Json(filter)).await?.0.findings) }
    };
    // Every source at once; Cloud Security's vulnerabilities only critical and high on running resources, filtered by the database.
    let running_severe = super::cloud_security::VulnScope { severities: severe.clone(), running: true, ..Default::default() };
    let (kev, vulns, sast, secrets, iac, (misconfigs, _), infra) = tokio::try_join!(
        async { sqlx::query_scalar::<_, String>("SELECT cve_id FROM security_kev").fetch_all(&st.postgres).await.map_err(ApiError::from) },
        code_security::library_vulnerabilities(st, org),
        findings("sast"),
        findings("secret"),
        findings("iac"),
        super::cloud_security::misconfigurations_of(st, org),
        super::cloud_security::infra_vulnerabilities_in(st, org, &running_severe),
    )?;
    let kev: HashSet<String> = kev.into_iter().collect();
    let vulns = vulns.into_iter().filter(|v| v.status == "open" && severe.contains(&v.severity));
    let mut out = Vec::new();
    for v in vulns {
        let exploited = v.cve.iter().chain(v.aliases.iter()).any(|c| kev.contains(c));
        if !v.in_production && !exploited {
            continue;
        }
        let mut reasons = Vec::new();
        if v.in_production {
            reasons.push("Loaded in production".to_string());
        }
        if exploited {
            reasons.push("Exploited in the wild".to_string());
        }
        out.push(InboxFinding {
            key: format!("vuln:{}:{}:{}", v.advisory_id, v.ecosystem, v.package),
            title: titled(&v.title, &v.package),
            finding_type: "Library Vulnerability".into(),
            severity: v.severity,
            resource_type: if v.services.is_empty() { "Repository".into() } else { "Service".into() },
            resources: v.services.iter().chain(v.repositories.iter()).cloned().collect(),
            services: v.services,
            repositories: v.repositories,
            reasons,
            first_seen: v.first_seen,
            triage: "pending".into(),
            cve: v.cve,
            link: format!("/code-security/vulnerabilities/libraries?q={}", v.advisory_id),
        });
    }
    for (kind, label, found) in [("sast", "Code Vulnerability", sast), ("secret", "Secret", secrets), ("iac", "IaC Misconfiguration", iac)] {
        for f in found {
            let severity = if kind == "secret" && !matches!(f.severity.as_str(), "critical" | "high") { "high".to_string() } else { f.severity.clone() };
            out.push(InboxFinding {
                key: format!("finding:{}", f.id),
                title: if f.rule_name.is_empty() { f.message.clone() } else { f.rule_name.clone() },
                finding_type: label.into(),
                severity,
                resource_type: "Repository".into(),
                resources: vec![f.repository.clone()],
                services: vec![],
                repositories: vec![f.repository.clone()],
                reasons: vec![format!("{}:{}", f.file, f.line)],
                first_seen: f.first_seen,
                triage: "pending".into(),
                cve: None,
                link: format!("/code-security/vulnerabilities/{}", if kind == "sast" { "static-code" } else if kind == "secret" { "secrets" } else { "iac" }),
            });
        }
    }
    // Cloud Security: critical and high misconfigurations, and vulnerabilities of running hosts and images.
    out.extend(tagging_inbox_item(&misconfigs));
    for m in misconfigs.into_iter().filter(|m| m.status == "open" && matches!(m.severity.as_str(), "critical" | "high")) {
        out.push(InboxFinding {
            key: m.key,
            title: m.rule_name,
            finding_type: "Misconfiguration".into(),
            severity: m.severity,
            resource_type: m.resource_type.clone(),
            resources: vec![m.resource_name.clone()],
            services: m.service.into_iter().collect(),
            repositories: vec![],
            reasons: vec![m.detail],
            first_seen: m.last_seen,
            triage: "pending".into(),
            cve: None,
            link: format!("/security/cloud/findings?rule={}", m.rule_id),
        });
    }
    for v in infra {
        if v.status != "open" || !v.running || !matches!(v.severity.as_str(), "critical" | "high") {
            continue;
        }
        let mut reasons = vec!["Running".to_string()];
        if v.exploit_available {
            reasons.push("Exploited in the wild".into());
        }
        if let Some(f) = &v.fixed {
            reasons.push(format!("Fixed in {f}"));
        }
        out.push(InboxFinding {
            key: v.key,
            title: titled(&v.title, &v.package),
            finding_type: "Infrastructure Vulnerability".into(),
            severity: v.severity,
            resource_type: if v.resource_kind == "host" { "Host".into() } else { "Container Image".into() },
            resources: vec![v.resource_name],
            services: vec![],
            repositories: vec![],
            reasons,
            first_seen: v.first_seen,
            triage: "pending".into(),
            cve: v.cve,
            link: format!("/security/cloud/vulnerabilities?q={}", v.advisory_id),
        });
    }
    out.sort_by(|a, b| super::security_rules::severity_rank(&b.severity).cmp(&super::security_rules::severity_rank(&a.severity)).then(b.first_seen.cmp(&a.first_seen)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::cloud_security::tagging_misconfigurations;
    use crate::routes::provenance::{TagTarget, TaggedResource};
    use crate::routes::tagging_rules::{NonCompliantResource, PolicyGaps, TagGap};

    fn gap(name: &str, service: Option<&str>) -> NonCompliantResource {
        let resource = TaggedResource {
            id: format!("arn:aws:s3:::{name}"),
            target: TagTarget::Cloud,
            name: name.into(),
            resource_type: "AwsS3Bucket".into(),
            provider: Some("aws".into()),
            account: Some("123456789012".into()),
            region: None,
            cluster: None,
            namespace: None,
            env: None,
            team: None,
            tags: service.map(|s| vec![format!("service:{s}")]).unwrap_or_default(),
            provenance: None,
        };
        NonCompliantResource { resource, policies: vec![PolicyGaps { policy_id: "p".into(), policy_name: "Owners".into(), gaps: vec![TagGap { key: "team".into(), value: None }] }] }
    }

    #[test]
    fn titles_name_the_package_once() {
        assert_eq!(titled("example-zlib: heap overflow when inflating crafted streams", "example-zlib"), "example-zlib: heap overflow when inflating crafted streams");
        assert_eq!(titled("Heap overflow when inflating crafted streams", "example-zlib"), "Heap overflow when inflating crafted streams in example-zlib");
    }

    #[test]
    fn resources_missing_tags_are_one_low_inbox_item_without_the_muted_ones() {
        let triage = std::collections::HashMap::from([("misconfig:ocs-tag-001:arn:aws:s3:::example-shop-logs".to_string(), "muted".to_string())]);
        let (findings, _) = tagging_misconfigurations(3, &[gap("example-shop-assets", Some("storefront-api")), gap("example-shop-logs", None), gap("example-shop-backups", Some("storefront-api"))], &triage);
        let item = tagging_inbox_item(&findings).unwrap();
        assert_eq!((item.severity.as_str(), item.title.as_str(), item.link.as_str()), ("low", "Resource missing required tags", "/infrastructure/tagging"));
        assert_eq!(item.resources, vec!["example-shop-assets", "example-shop-backups"]);
        assert_eq!(item.services, vec!["storefront-api"]);
        assert_eq!(item.reasons, vec!["2 resources lack tags the tagging policies require"]);
        assert!(tagging_inbox_item(&[]).is_none());
    }
}
