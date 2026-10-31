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
    let setup = code_security::setup(State(st.clone()), OrgId(org.clone())).await?.0;
    let summary = code_security::summary(State(st.clone()), OrgId(org.clone())).await?.0;
    let repos = code_security::repositories(State(st.clone()), OrgId(org.clone())).await?.0;
    let inv: InventoryRow = st
        .ch(&org)
        .query(
            "SELECT uniqExact(ServiceName) AS services, uniqExactIf(concat(ServiceName, ' ', SpanName), SpanKind = 'Server') AS endpoints, \
             uniqExactIf(ResourceAttributes['host.name'], ResourceAttributes['host.name'] != '') AS hosts \
             FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND Timestamp > now() - INTERVAL 1 HOUR",
        )
        .fetch_one()
        .await?;
    let agent_hosts: i64 = sqlx::query_scalar("SELECT count(*) FROM datadog_hosts WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await?;
    let packs = pack_statuses(&st, &org, &access, now).await?;
    let signals = signals_in(&st, &org, TimeRange { from: now - 7 * DAY, to: now }).await?;
    let total = |c: &code_security::SeverityCounts| c.critical + c.high + c.medium + c.low + c.unknown;
    // Library vulnerabilities, to find the exploited ones and those in attacked services.
    let vulns = code_security::list_library_vulnerabilities(State(st.clone()), OrgId(org.clone()), Json(CsFilter { limit: 5000, ..Default::default() })).await?.0.vulnerabilities;
    let kev: HashSet<String> = sqlx::query_scalar("SELECT cve_id FROM security_kev").fetch_all(&st.postgres).await?.into_iter().collect();
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
        repositories: repos.len().max(setup.repositories_with_code_scans).max(setup.repositories_with_libraries),
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

pub async fn inbox_findings(st: &AppState, org: &str) -> Result<Vec<InboxFinding>, ApiError> {
    let kev: HashSet<String> = sqlx::query_scalar("SELECT cve_id FROM security_kev").fetch_all(&st.postgres).await?.into_iter().collect();
    let severe = vec!["critical".to_string(), "high".to_string()];
    let vulns = code_security::list_library_vulnerabilities(State(st.clone()), OrgId(org.to_string()), Json(CsFilter { severity: severe.clone(), limit: 5000, ..Default::default() })).await?.0.vulnerabilities;
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
            title: format!("{} in {}", v.title, v.package),
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
    for (kind, label) in [("sast", "Code Vulnerability"), ("secret", "Secret"), ("iac", "IaC Misconfiguration")] {
        let filter = FindingFilter { kind: kind.into(), filter: CsFilter { severity: if kind == "secret" { vec![] } else { severe.clone() }, limit: 5000, ..Default::default() } };
        let page = code_security::list_findings(State(st.clone()), OrgId(org.to_string()), Json(filter)).await?.0;
        for f in page.findings {
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
    let (misconfigs, _) = super::cloud_security::misconfigurations_of(st, org).await?;
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
    for v in super::cloud_security::infra_vulnerabilities(st, org).await? {
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
            title: format!("{} in {}", v.title, v.package),
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
