//! Code Security from GitLab: the GitLab CI connection, with `import_code_security` on, also imports the results
//! of GitLab's own scanners (its Vulnerability Report) rather than scanning again. On the cloud crawlers' jobs
//! (`crate::cloud::jobs`, provider [`PROVIDER`]), like the GitLab CI poller:
//!
//! - **Discovery** (scope `""`, hourly): the connection's projects ([`super::crawler::discover`]), one job each
//!   (scope `<id>:<path>`).
//! - **A project's import** ([`import_project`], every [`INTERVAL_SECS`]) reads every vulnerability of the project
//!   from GitLab's GraphQL API (`project.vulnerabilities`, every state, [`PAGE_SIZE`] a page, at most
//!   [`MAX_PAGES`] pages) and GitLab's dependency list, maps them ([`parse_vulnerability`]) and stores them
//!   ([`store_import`]): SAST → code findings, KICS (reported as SAST) → IaC, Secret Detection → secrets
//!   (`cs_findings`, source `gitlab`, keyed by GitLab's vulnerability id); Dependency Scanning → library
//!   vulnerabilities (`cs_dependency_findings`) and the libraries (`cs_libraries`, source `gitlab`). Container
//!   scanning and the dynamic scanners (DAST, fuzzing) are left out: their findings are about images and running
//!   apps, not the repository, and Cloud Security's image vulnerabilities are computed from SBOMs × advisories.
//! - GitLab's state carries over: dismissed → muted (with GitLab's reason), resolved → resolved. A complete import
//!   resolves the project's GitLab findings it no longer lists.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::api::{self, GitlabApi, GitlabError, PER_PAGE};
use super::crawler::{connection, discover, issue, scope_project_id, Connection};
use crate::cloud::jobs::{self, CrawlIssue, Crawler, JobKey, Outcome};
use crate::routes::code_security_model::{self as model, Library};
use crate::routes::portal_sources::parse_time;
use crate::state::AppState;

pub const PROVIDER: &str = "gitlab-security";
/// How often a project is imported again.
pub const INTERVAL_SECS: i64 = 6 * 3600;
/// Vulnerabilities per GraphQL page (small requests: the nodes carry descriptions).
pub const PAGE_SIZE: usize = 50;
/// Pages per project import: past them the import is partial and closes nothing.
pub const MAX_PAGES: usize = 80;
/// Pages of the dependency list read per import.
const MAX_DEPENDENCY_PAGES: usize = 10;

/// GitLab's GraphQL query: the project, its default branch and a page of its vulnerabilities, every state.
pub const VULNERABILITIES_QUERY: &str = "query($fullPath: ID!, $first: Int!, $after: String) { project(fullPath: $fullPath) { id fullPath webUrl repository { rootRef } \
vulnerabilities(first: $first, after: $after, state: [DETECTED, CONFIRMED, DISMISSED, RESOLVED]) { pageInfo { hasNextPage endCursor } nodes { \
id title description severity state reportType solution webUrl detectedAt dismissedAt resolvedAt dismissalReason scanner { externalId name vendor } \
primaryIdentifier { externalType externalId name url } identifiers { externalType externalId name url } location { __typename \
... on VulnerabilityLocationSast { file startLine } ... on VulnerabilityLocationSecretDetection { file startLine } \
... on VulnerabilityLocationDependencyScanning { file dependency { package { name } version } } \
... on VulnerabilityLocationContainerScanning { image operatingSystem dependency { package { name } version } } } } } } }";

/// The query's variables for a page.
pub fn variables(full_path: &str, after: Option<&str>) -> Value {
    json!({ "fullPath": full_path, "first": PAGE_SIZE, "after": after })
}

/// A GitLab project as a Code Security repository.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectInfo {
    pub path: String,
    pub web_url: String,
    pub default_branch: String,
    /// `gitlab.example.com/example-org/storefront-api`, as uploads from its CI name it.
    pub repository: String,
}

/// One GitLab vulnerability, mapped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitlabVuln {
    /// GitLab's vulnerability id (the number of `gid://gitlab/Vulnerability/<id>`).
    pub external_id: String,
    /// sast | iac | secret | dependency | container | other
    pub kind: &'static str,
    /// The scanner: semgrep, kics, gitleaks, gemnasium, trivy, …
    pub tool: String,
    pub rule_id: String,
    pub title: String,
    pub description: String,
    /// critical | high | medium | low | unknown
    pub severity: String,
    /// detected | confirmed | dismissed | resolved
    pub state: String,
    /// GitLab's dismissal reason, in words.
    pub reason: String,
    pub url: String,
    /// Every identifier's id (rule ids, CVE, CWE number), for matching uploads of the same finding.
    pub identifiers: Vec<String>,
    pub cwes: Vec<String>,
    /// CVE / GHSA / scanner advisory ids.
    pub advisories: Vec<String>,
    pub help_uri: String,
    pub file: String,
    pub line: i32,
    pub package: String,
    pub version: String,
    pub ecosystem: String,
    pub image: String,
    pub solution: String,
    pub fixed: String,
    pub detected_at: Option<i64>,
    pub resolved_at: Option<i64>,
}

impl GitlabVuln {
    /// The advisory a dependency finding is filed under: its CVE, else the scanner's own id.
    pub fn advisory_id(&self) -> String {
        self.advisories.iter().find(|a| a.starts_with("CVE-")).or(self.advisories.first()).cloned().unwrap_or_else(|| self.rule_id.clone())
    }
}

fn severity(s: &str) -> &'static str {
    match s {
        "CRITICAL" => "critical",
        "HIGH" => "high",
        "MEDIUM" => "medium",
        "LOW" | "INFO" => "low",
        _ => "unknown",
    }
}

/// GitLab's dismissal reason (`VulnerabilityDismissalReason`) in words.
pub fn dismissal_reason(r: &str) -> String {
    match r {
        "ACCEPTABLE_RISK" => "Acceptable risk".into(),
        "FALSE_POSITIVE" => "False positive".into(),
        "MITIGATING_CONTROL" => "Mitigating control".into(),
        "USED_IN_TESTS" => "Used in tests".into(),
        "NOT_APPLICABLE" => "Not applicable".into(),
        "" => String::new(),
        other => other.replace('_', " ").to_lowercase(),
    }
}

/// The OSV ecosystem of a lockfile or manifest.
pub fn ecosystem_of_file(file: &str) -> Option<&'static str> {
    let name = file.rsplit('/').next().unwrap_or(file).to_ascii_lowercase();
    Some(match name.as_str() {
        "package-lock.json" | "npm-shrinkwrap.json" | "yarn.lock" | "pnpm-lock.yaml" | "package.json" | "bun.lock" => "npm",
        "requirements.txt" | "pipfile.lock" | "poetry.lock" | "pdm.lock" | "uv.lock" | "setup.py" | "pipdeptree.json" | "pyproject.toml" => "PyPI",
        "pom.xml" | "gradle.lockfile" | "build.gradle" | "build.gradle.kts" | "build.sbt" | "maven-dependencies.json" | "gradle-dependencies.json" => "Maven",
        "go.sum" | "go.mod" | "go.graph" => "Go",
        "gemfile.lock" | "gems.locked" => "RubyGems",
        "composer.lock" => "Packagist",
        "packages.lock.json" | "packages.config" => "NuGet",
        "cargo.lock" => "crates.io",
        _ if name.ends_with(".csproj") => "NuGet",
        _ if name.starts_with("requirements") && name.ends_with(".txt") => "PyPI",
        _ => return None,
    })
}

/// The OSV ecosystem of GitLab's `package_manager`.
pub fn ecosystem_of_package_manager(pm: &str) -> Option<&'static str> {
    Some(match pm.to_ascii_lowercase().as_str() {
        "npm" | "yarn" | "pnpm" | "bun" => "npm",
        "pip" | "pipenv" | "poetry" | "setuptools" | "conda" | "uv" | "pdm" => "PyPI",
        "maven" | "gradle" | "sbt" => "Maven",
        "go" => "Go",
        "bundler" => "RubyGems",
        "composer" => "Packagist",
        "nuget" => "NuGet",
        "cargo" => "crates.io",
        _ => return None,
    })
}

/// The version a dependency scanner's solution points to (`Upgrade to version 2.1.4 or above.`, `Upgrade to versions
/// 1.2.3, 2.0.1 or above.`: the first).
pub fn fixed_version(solution: &str) -> String {
    let lower = solution.to_ascii_lowercase();
    let Some(at) = lower.find("version") else { return String::new() };
    let rest = solution[at..].trim_start_matches(|c: char| c.is_ascii_alphabetic()).trim_start();
    let v: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != ',').collect();
    let v = v.trim_end_matches('.').to_string();
    if v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        v
    } else {
        String::new()
    }
}

/// The id of a GitLab finding in `cs_findings` / `cs_dependency_findings`: GitLab's id on that GitLab (never another
/// instance's).
pub fn finding_id(base_url: &str, external_id: &str) -> String {
    let h = Sha256::digest(format!("gitlab|{base_url}|{external_id}").as_bytes());
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    format!("gl{}", &hex[..30])
}

fn ts(v: &Value) -> Option<i64> {
    v.as_str().map(parse_time).filter(|t| *t > 0)
}

/// A node of `project.vulnerabilities` (GitLab's `Vulnerability` type), mapped; None without an id.
pub fn parse_vulnerability(n: &Value) -> Option<GitlabVuln> {
    let external_id = n["id"].as_str()?.rsplit('/').next()?.to_string();
    if external_id.is_empty() {
        return None;
    }
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let tool = [n.pointer("/scanner/externalId"), n.pointer("/scanner/name")].into_iter().flatten().filter_map(Value::as_str).find(|t| !t.is_empty()).unwrap_or("gitlab").to_ascii_lowercase();
    let report = n["reportType"].as_str().unwrap_or_default();
    let kind = match report {
        "SAST" if tool == "kics" => "iac",
        "SAST" => "sast",
        "SECRET_DETECTION" => "secret",
        "DEPENDENCY_SCANNING" => "dependency",
        "CONTAINER_SCANNING" | "CLUSTER_IMAGE_SCANNING" => "container",
        _ => "other",
    };
    let ids: Vec<&Value> = n["identifiers"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let primary = if n["primaryIdentifier"].is_object() { &n["primaryIdentifier"] } else { ids.first().copied().unwrap_or(&Value::Null) };
    let of_type = |t: &str| ids.iter().filter(|i| i["externalType"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(t))).filter_map(|i| i["externalId"].as_str()).map(str::to_string).collect::<Vec<_>>();
    let cwes: Vec<String> = of_type("cwe").into_iter().map(|c| format!("CWE-{}", c.trim_start_matches("CWE-"))).collect();
    let advisories: Vec<String> = ids
        .iter()
        .filter(|i| !i["externalType"].as_str().is_some_and(|t| t.eq_ignore_ascii_case("cwe")))
        .filter_map(|i| i["externalId"].as_str().or(i["name"].as_str()))
        .filter(|x| x.starts_with("CVE-") || x.starts_with("GHSA-") || x.starts_with("GMS-"))
        .map(str::to_string)
        .collect();
    let loc = &n["location"];
    let solution = s(&n["solution"]);
    let file = s(&loc["file"]);
    let state = n["state"].as_str().unwrap_or("DETECTED").to_ascii_lowercase();
    let resolved_at = if state == "resolved" { ts(&n["resolvedAt"]).or(Some(0)) } else { None };
    Some(GitlabVuln {
        kind,
        rule_id: primary["externalId"].as_str().filter(|x| !x.is_empty()).map(str::to_string).unwrap_or_else(|| s(&n["title"])),
        title: s(&n["title"]),
        description: s(&n["description"]),
        severity: severity(n["severity"].as_str().unwrap_or_default()).into(),
        reason: if state == "dismissed" { dismissal_reason(n["dismissalReason"].as_str().unwrap_or_default()) } else { String::new() },
        state,
        url: s(&n["webUrl"]),
        identifiers: ids.iter().filter_map(|i| i["externalId"].as_str()).filter(|x| !x.is_empty()).map(str::to_string).collect(),
        cwes,
        advisories,
        help_uri: s(&primary["url"]),
        line: loc["startLine"].as_str().and_then(|l| l.trim().parse().ok()).or_else(|| loc["startLine"].as_i64().map(|l| l as i32)).unwrap_or(0),
        package: s(&loc["dependency"]["package"]["name"]),
        version: s(&loc["dependency"]["version"]),
        ecosystem: if kind == "dependency" { ecosystem_of_file(&file).unwrap_or_default().to_string() } else { String::new() },
        image: s(&loc["image"]),
        fixed: fixed_version(&solution),
        solution,
        file,
        detected_at: ts(&n["detectedAt"]),
        resolved_at,
        external_id,
        tool,
    })
}

/// A page of the GraphQL answer: the project, its vulnerabilities and the next page's cursor.
pub fn parse_page(data: &Value) -> Result<(ProjectInfo, Vec<GitlabVuln>, Option<String>), GitlabError> {
    let p = &data["project"];
    if !p.is_object() {
        return Err(GitlabError::NotFound("GitLab answered no such project (or the token can't see it)".into()));
    }
    let v = &p["vulnerabilities"];
    if !v.is_object() {
        return Err(GitlabError::Auth("GitLab answered no vulnerabilities: the token's user needs at least the Developer role on the project to read its Vulnerability Report".into()));
    }
    let web_url = p["webUrl"].as_str().unwrap_or_default().to_string();
    let info = ProjectInfo {
        path: p["fullPath"].as_str().unwrap_or_default().to_string(),
        repository: model::normalize_repo(&web_url),
        default_branch: p.pointer("/repository/rootRef").and_then(Value::as_str).unwrap_or_default().to_string(),
        web_url,
    };
    let vulns = v["nodes"].as_array().into_iter().flatten().filter_map(parse_vulnerability).collect();
    let next = if v.pointer("/pageInfo/hasNextPage").and_then(Value::as_bool).unwrap_or(false) { v.pointer("/pageInfo/endCursor").and_then(Value::as_str).map(str::to_string) } else { None };
    Ok((info, vulns, next))
}

/// What one project's import read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectImport {
    pub project: ProjectInfo,
    /// The findings Code Security keeps (code, IaC, secrets, dependencies).
    pub vulns: Vec<GitlabVuln>,
    /// Container and dynamic scanners' findings, left out.
    pub skipped: usize,
    /// Every page was read: what the import didn't list is gone from GitLab.
    pub complete: bool,
    /// The project's libraries (GitLab's dependency list and the vulnerable packages).
    pub libraries: Vec<Library>,
}

/// Reads a project's vulnerabilities (every page, up to [`MAX_PAGES`]) and its dependency list (best effort).
pub async fn import_project(api: &dyn GitlabApi, conn: &Connection, project_id: u64, full_path: &str) -> Result<ProjectImport, GitlabError> {
    let mut out = ProjectImport::default();
    let mut after: Option<String> = None;
    for page in 0..MAX_PAGES {
        let data = api.graphql(&conn.base_url, &conn.token, VULNERABILITIES_QUERY, variables(full_path, after.as_deref())).await?;
        let (project, vulns, next) = parse_page(&data)?;
        if page == 0 {
            out.project = project;
        }
        for v in vulns {
            if matches!(v.kind, "sast" | "iac" | "secret" | "dependency") {
                out.vulns.push(v);
            } else {
                out.skipped += 1;
            }
        }
        match next {
            Some(n) => after = Some(n),
            None => {
                out.complete = true;
                break;
            }
        }
    }
    out.libraries = libraries(api, conn, project_id, &out.vulns).await?;
    Ok(out)
}

/// The project's libraries: GitLab's dependency list (Ultimate; skipped when GitLab refuses it) and the packages
/// its dependency findings name.
async fn libraries(api: &dyn GitlabApi, conn: &Connection, project_id: u64, vulns: &[GitlabVuln]) -> Result<Vec<Library>, GitlabError> {
    let mut out: Vec<Library> = Vec::new();
    let mut push = |l: Library| {
        if !l.ecosystem.is_empty() && !l.version.is_empty() && !out.iter().any(|o| o.ecosystem == l.ecosystem && o.name == l.name && o.version == l.version) {
            out.push(l);
        }
    };
    for page in 1..=MAX_DEPENDENCY_PAGES {
        let items = match api.get(&conn.base_url, &conn.token, &api::dependencies_path(project_id, page)).await {
            Ok(Value::Array(items)) => items,
            Err(e @ GitlabError::Throttled(_)) => return Err(e),
            _ => break,
        };
        for d in &items {
            let file = d["dependency_file_path"].as_str().unwrap_or_default();
            let Some(eco) = d["package_manager"].as_str().and_then(ecosystem_of_package_manager).or_else(|| ecosystem_of_file(file)) else { continue };
            let licenses = d["licenses"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().map(str::to_string)).collect();
            push(Library { ecosystem: eco.into(), name: model::normalize_name(eco, d["name"].as_str().unwrap_or_default()), version: d["version"].as_str().unwrap_or_default().into(), file: file.into(), licenses, ..Default::default() });
        }
        if items.len() < PER_PAGE {
            break;
        }
    }
    for v in vulns.iter().filter(|v| v.kind == "dependency" && !v.package.is_empty()) {
        push(Library { ecosystem: v.ecosystem.clone(), name: model::normalize_name(&v.ecosystem, &v.package), version: v.version.clone(), file: v.file.clone(), ..Default::default() });
    }
    Ok(out)
}

/// Stores a project's import (see the module documentation); returns the findings stored.
pub async fn store_import(st: &AppState, org: &str, base_url: &str, imp: &ProjectImport) -> Result<usize, sqlx::Error> {
    let repo = &imp.project.repository;
    let branch = &imp.project.default_branch;
    let mut tx = st.postgres.begin().await?;
    let mut static_ids: Vec<String> = Vec::new();
    let mut dependency_ids: Vec<String> = Vec::new();
    for v in &imp.vulns {
        let id = finding_id(base_url, &v.external_id);
        let resolved = v.resolved_at.map(|t| t as f64);
        if v.kind == "dependency" {
            dependency_ids.push(id.clone());
            sqlx::query(
                "INSERT INTO cs_dependency_findings (org_id, id, source, external_id, tool, repository, branch, ecosystem, package, version, file, advisory_id, aliases, title, description, \
                 severity, cwes, solution, fixed, url, upstream_state, upstream_reason, first_seen, resolved_at) \
                 VALUES ($1, $2, 'gitlab', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, COALESCE(to_timestamp($22::double precision / 1000), now()), \
                 CASE WHEN $23::double precision IS NULL THEN NULL WHEN $23 > 0 THEN to_timestamp($23 / 1000) ELSE now() END) \
                 ON CONFLICT (org_id, id) DO UPDATE SET tool = EXCLUDED.tool, repository = EXCLUDED.repository, branch = EXCLUDED.branch, ecosystem = EXCLUDED.ecosystem, \
                 package = EXCLUDED.package, version = EXCLUDED.version, file = EXCLUDED.file, advisory_id = EXCLUDED.advisory_id, aliases = EXCLUDED.aliases, title = EXCLUDED.title, \
                 description = EXCLUDED.description, severity = EXCLUDED.severity, cwes = EXCLUDED.cwes, solution = EXCLUDED.solution, fixed = EXCLUDED.fixed, url = EXCLUDED.url, \
                 upstream_state = EXCLUDED.upstream_state, upstream_reason = EXCLUDED.upstream_reason, last_seen = now(), \
                 resolved_at = CASE WHEN EXCLUDED.resolved_at IS NULL THEN NULL ELSE COALESCE(cs_dependency_findings.resolved_at, EXCLUDED.resolved_at) END",
            )
            .bind(org)
            .bind(&id)
            .bind(&v.external_id)
            .bind(&v.tool)
            .bind(repo)
            .bind(branch)
            .bind(&v.ecosystem)
            .bind(model::normalize_name(&v.ecosystem, &v.package))
            .bind(&v.version)
            .bind(&v.file)
            .bind(v.advisory_id())
            .bind(&v.advisories)
            .bind(&v.title)
            .bind(&v.description)
            .bind(&v.severity)
            .bind(&v.cwes)
            .bind(&v.solution)
            .bind(&v.fixed)
            .bind(&v.url)
            .bind(&v.state)
            .bind(&v.reason)
            .bind(v.detected_at.map(|t| t as f64))
            .bind(resolved)
            .execute(&mut *tx)
            .await?;
            continue;
        }
        static_ids.push(id.clone());
        sqlx::query(
            "INSERT INTO cs_rules (org_id, kind, tool, rule_id, name, description, severity, cwes, help_uri, language) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '') \
             ON CONFLICT (org_id, tool, rule_id) DO UPDATE SET kind = EXCLUDED.kind, name = EXCLUDED.name, description = EXCLUDED.description, severity = EXCLUDED.severity, \
             cwes = EXCLUDED.cwes, help_uri = EXCLUDED.help_uri",
        )
        .bind(org)
        .bind(v.kind)
        .bind(&v.tool)
        .bind(&v.rule_id)
        .bind(&v.title)
        .bind(&v.description)
        .bind(&v.severity)
        .bind(&v.cwes)
        .bind(&v.help_uri)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO cs_findings (org_id, id, kind, tool, rule_id, severity, message, repository, branch, file, line, source, external_id, url, upstream_state, upstream_reason, identifiers, first_seen, resolved_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'gitlab', $12, $13, $14, $15, $16, COALESCE(to_timestamp($17::double precision / 1000), now()), \
             CASE WHEN $18::double precision IS NULL THEN NULL WHEN $18 > 0 THEN to_timestamp($18 / 1000) ELSE now() END) \
             ON CONFLICT (org_id, id) DO UPDATE SET kind = EXCLUDED.kind, tool = EXCLUDED.tool, rule_id = EXCLUDED.rule_id, severity = EXCLUDED.severity, message = EXCLUDED.message, \
             repository = EXCLUDED.repository, branch = EXCLUDED.branch, file = EXCLUDED.file, line = EXCLUDED.line, url = EXCLUDED.url, upstream_state = EXCLUDED.upstream_state, \
             upstream_reason = EXCLUDED.upstream_reason, identifiers = EXCLUDED.identifiers, last_seen = now(), \
             resolved_at = CASE WHEN EXCLUDED.resolved_at IS NULL THEN NULL ELSE COALESCE(cs_findings.resolved_at, EXCLUDED.resolved_at) END",
        )
        .bind(org)
        .bind(&id)
        .bind(v.kind)
        .bind(&v.tool)
        .bind(&v.rule_id)
        .bind(&v.severity)
        .bind(&v.title)
        .bind(repo)
        .bind(branch)
        .bind(&v.file)
        .bind(v.line)
        .bind(&v.external_id)
        .bind(&v.url)
        .bind(&v.state)
        .bind(&v.reason)
        .bind(&v.identifiers)
        .bind(v.detected_at.map(|t| t as f64))
        .bind(resolved)
        .execute(&mut *tx)
        .await?;
    }
    if imp.complete {
        // Gone from GitLab (deleted, or the project's report was cleared): closed.
        sqlx::query("UPDATE cs_findings SET resolved_at = now() WHERE org_id = $1 AND source = 'gitlab' AND repository = $2 AND NOT (id = ANY($3)) AND resolved_at IS NULL")
            .bind(org)
            .bind(repo)
            .bind(&static_ids)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE cs_dependency_findings SET resolved_at = now() WHERE org_id = $1 AND source = 'gitlab' AND repository = $2 AND NOT (id = ANY($3)) AND resolved_at IS NULL")
            .bind(org)
            .bind(repo)
            .bind(&dependency_ids)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cs_libraries WHERE org_id = $1 AND source = 'gitlab' AND repository = $2").bind(org).bind(repo).execute(&mut *tx).await?;
    }
    for l in &imp.libraries {
        sqlx::query(
            "INSERT INTO cs_libraries (org_id, source, ecosystem, name, version, repository, language, branch, file, licenses) VALUES ($1, 'gitlab', $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (org_id, source, ecosystem, name, version, service, env, repository) DO UPDATE SET last_seen = now(), branch = EXCLUDED.branch, \
             file = CASE WHEN EXCLUDED.file = '' THEN cs_libraries.file ELSE EXCLUDED.file END, licenses = CASE WHEN cardinality(EXCLUDED.licenses) = 0 THEN cs_libraries.licenses ELSE EXCLUDED.licenses END",
        )
        .bind(org)
        .bind(&l.ecosystem)
        .bind(&l.name)
        .bind(&l.version)
        .bind(repo)
        .bind(model::language_of_ecosystem(&l.ecosystem))
        .bind(branch)
        .bind(&l.file)
        .bind(&l.licenses)
        .execute(&mut *tx)
        .await?;
    }
    // The repository's latest import, as a scan (the Repositories page lists repositories by their scans).
    sqlx::query(
        "INSERT INTO cs_scans (org_id, id, kind, tool, repository, branch, results) VALUES ($1, $2, 'gitlab', 'gitlab', $3, $4, $5) \
         ON CONFLICT (org_id, id) DO UPDATE SET repository = EXCLUDED.repository, branch = EXCLUDED.branch, results = EXCLUDED.results, received_at = now()",
    )
    .bind(org)
    .bind(format!("gitlab:{}", finding_id(base_url, &imp.project.path)))
    .bind(repo)
    .bind(branch)
    .bind(imp.vulns.len() as i32)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(imp.vulns.len())
}

/// Removes the org's import: its jobs and every imported result (the import was turned off, the connection removed
/// or moved to another GitLab).
pub async fn forget(tx: &mut sqlx::PgConnection, org: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(org).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cs_findings WHERE org_id = $1 AND source = 'gitlab'").bind(org).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cs_dependency_findings WHERE org_id = $1 AND source = 'gitlab'").bind(org).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cs_libraries WHERE org_id = $1 AND source = 'gitlab'").bind(org).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cs_scans WHERE org_id = $1 AND kind = 'gitlab'").bind(org).execute(&mut *tx).await?;
    Ok(())
}

/// A project's import.
#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GitlabImportedProject {
    pub id: u64,
    pub path: String,
    /// Epoch ms of the last complete import.
    pub last_import_at: Option<i64>,
    /// Findings the last import stored.
    pub findings: i32,
    pub error: Option<CrawlIssue>,
}

/// Code Security's import from GitLab (Code Security → Settings).
#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitlabImportStatus {
    /// GitLab is connected (Software Delivery → Settings → GitLab CI).
    pub connected: bool,
    /// The connection imports Code Security results.
    pub enabled: bool,
    pub base_url: String,
    pub projects: Vec<GitlabImportedProject>,
    /// Epoch ms of the last project list.
    pub discovered_at: Option<i64>,
    pub discovery_error: Option<CrawlIssue>,
    /// Epoch ms of the latest project import.
    pub last_import_at: Option<i64>,
    /// Open code, secret and IaC findings, and open library vulnerabilities, imported.
    pub open_findings: i64,
    pub open_library_vulnerabilities: i64,
}

/// The org's import status.
pub async fn status(st: &AppState, org: &str) -> Result<GitlabImportStatus, sqlx::Error> {
    let row: Option<(String, String, bool)> = sqlx::query_as("SELECT id::text, base_url, import_code_security FROM gitlab_ci_connections WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    let Some((id, base_url, enabled)) = row else { return Ok(GitlabImportStatus::default()) };
    let (open_findings, open_library_vulnerabilities): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM cs_findings WHERE org_id = $1 AND source = 'gitlab' AND resolved_at IS NULL), \
         (SELECT count(*) FROM cs_dependency_findings WHERE org_id = $1 AND source = 'gitlab' AND resolved_at IS NULL)",
    )
    .bind(org)
    .fetch_one(&st.postgres)
    .await?;
    let mut out = GitlabImportStatus { connected: true, enabled, base_url, open_findings, open_library_vulnerabilities, ..Default::default() };
    for s in jobs::status(st, PROVIDER, org, &id).await? {
        if s.scope.is_empty() {
            out.discovered_at = s.last_success_at;
            out.discovery_error = s.error;
        } else if let Some(pid) = scope_project_id(&s.scope) {
            out.last_import_at = out.last_import_at.max(s.last_success_at);
            let path = s.scope.split_once(':').map(|(_, p)| p.to_string()).unwrap_or_default();
            out.projects.push(GitlabImportedProject { id: pid, path, last_import_at: s.last_success_at, findings: s.metrics, error: s.error });
        }
    }
    out.projects.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The importer: GitLab through `api`, findings into Postgres.
pub struct GitlabSecurityCrawler {
    pub api: Arc<dyn GitlabApi>,
}

#[async_trait::async_trait]
impl Crawler for GitlabSecurityCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One discovery job per connection that imports Code Security results.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT c.org_id, c.id::text FROM gitlab_ci_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active' WHERE c.import_code_security")
                .fetch_all(&st.postgres)
                .await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, String::new())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some((id, conn)) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        let enabled: bool = sqlx::query_scalar("SELECT import_code_security FROM gitlab_ci_connections WHERE org_id = $1").bind(&job.org_id).fetch_one(&st.postgres).await?;
        if id != job.account_id || !enabled {
            return Ok(Outcome::Idle);
        }
        if job.scope.is_empty() {
            return Ok(match discover(self.api.as_ref(), &conn, now_ms).await {
                Ok(projects) => Outcome::Discovered(projects.iter().map(|(id, path)| super::crawler::scope(*id, path)).collect()),
                Err(e) => Outcome::Failed(issue(&e)),
            });
        }
        let (Some(project_id), Some((_, path))) = (scope_project_id(&job.scope), job.scope.split_once(':')) else { return Ok(Outcome::Idle) };
        Ok(match import_project(self.api.as_ref(), &conn, project_id, path).await {
            Ok(imp) => {
                let stored = store_import(st, &job.org_id, &conn.base_url, &imp).await?;
                let warning = (!imp.complete).then(|| crate::cloud::jobs::Issue {
                    kind: "other",
                    message: format!("The project has more than {} vulnerabilities: the import read the first ones only", MAX_PAGES * PAGE_SIZE),
                    missing_action: None,
                    backoff: crate::cloud::jobs::Backoff::Interval,
                });
                Outcome::Collected { end_ms: now_ms, metrics: stored, points: imp.libraries.len(), warning }
            }
            Err(e) => Outcome::Failed(issue(&e)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{self, FakeGitlab};
    use super::*;

    fn conn() -> Connection {
        Connection { base_url: "https://gitlab.example.com".into(), token: "glpat-test".into(), group_path: String::new(), include_subgroups: true, project_paths: vec![], all_projects: false }
    }

    const PATH: &str = "example-org/storefront-api";

    fn page() -> (ProjectInfo, Vec<GitlabVuln>, Option<String>) {
        parse_page(&fixtures::vulnerabilities_page(false, "c1")).unwrap()
    }

    fn by_id(vulns: &[GitlabVuln], id: &str) -> GitlabVuln {
        vulns.iter().find(|v| v.external_id == id).cloned().unwrap()
    }

    #[test]
    fn the_project_becomes_a_repository_named_like_uploads_from_its_ci() {
        let (p, _, next) = page();
        assert_eq!(p, ProjectInfo { path: PATH.into(), web_url: "https://gitlab.example.com/example-org/storefront-api".into(), default_branch: "main".into(), repository: "gitlab.example.com/example-org/storefront-api".into() });
        assert_eq!(p.repository, model::normalize_repo("https://gitlab.example.com/example-org/storefront-api.git"));
        assert_eq!(next, None);
        let (_, _, next) = parse_page(&fixtures::vulnerabilities_page(true, "c1")).unwrap();
        assert_eq!(next.as_deref(), Some("c1"));
    }

    #[test]
    fn sast_findings_keep_their_scanner_rule_file_and_line() {
        let (_, vulns, _) = page();
        let v = by_id(&vulns, "7001");
        assert_eq!((v.kind, v.tool.as_str(), v.rule_id.as_str(), v.file.as_str(), v.line), ("sast", "semgrep", "python.django.security.injection.sql.sql-injection-using-raw", "app/orders/views.py", 41));
        assert_eq!((v.severity.as_str(), v.state.as_str(), v.cwes.clone()), ("high", "detected", vec!["CWE-89".to_string()]));
        assert_eq!(v.url, "https://gitlab.example.com/example-org/storefront-api/-/security/vulnerabilities/7001");
        assert!(v.identifiers.contains(&"89".to_string()) && v.resolved_at.is_none() && v.detected_at.is_some());
    }

    #[test]
    fn kics_reported_as_sast_is_infrastructure_as_code() {
        let v = by_id(&page().1, "7002");
        assert_eq!((v.kind, v.tool.as_str(), v.file.as_str(), v.line), ("iac", "kics", "infra/s3.tf", 12));
        assert_eq!(v.help_uri, "https://docs.kics.io/latest/queries/terraform-queries/");
    }

    #[test]
    fn dismissed_secrets_keep_gitlabs_reason() {
        let v = by_id(&page().1, "7003");
        assert_eq!((v.kind, v.tool.as_str(), v.rule_id.as_str(), v.severity.as_str()), ("secret", "gitleaks", "AWS", "critical"));
        assert_eq!((v.state.as_str(), v.reason.as_str()), ("dismissed", "Used in tests"));
        assert_eq!(dismissal_reason("FALSE_POSITIVE"), "False positive");
    }

    #[test]
    fn dependency_findings_carry_the_package_cve_and_fix() {
        let v = by_id(&page().1, "7004");
        assert_eq!((v.kind, v.tool.as_str(), v.ecosystem.as_str(), v.package.as_str(), v.version.as_str()), ("dependency", "gemnasium", "npm", "example-merge", "2.1.0"));
        assert_eq!((v.advisory_id(), v.fixed.as_str(), v.state.as_str()), ("CVE-2026-90010".to_string(), "2.1.4", "confirmed"));
        assert_eq!(v.cwes, vec!["CWE-1321"]);
    }

    #[test]
    fn resolved_findings_carry_gitlabs_resolution_time() {
        let v = by_id(&page().1, "7005");
        assert_eq!((v.state.as_str(), v.resolved_at), ("resolved", Some(parse_time("2026-10-20T10:00:00Z"))));
    }

    #[test]
    fn container_findings_are_recognised_to_be_left_out() {
        let v = by_id(&page().1, "7006");
        assert_eq!((v.kind, v.tool.as_str(), v.image.as_str()), ("container", "trivy", "registry.example.com/storefront-api:1.4.0"));
    }

    #[test]
    fn helpers_map_lockfiles_package_managers_and_solutions() {
        assert_eq!(ecosystem_of_file("services/api/Gemfile.lock"), Some("RubyGems"));
        assert_eq!(ecosystem_of_file("requirements-dev.txt"), Some("PyPI"));
        assert_eq!(ecosystem_of_file("src/Catalog.csproj"), Some("NuGet"));
        assert_eq!(ecosystem_of_file("Dockerfile"), None);
        assert_eq!(ecosystem_of_package_manager("bundler"), Some("RubyGems"));
        assert_eq!(ecosystem_of_package_manager("conan"), None);
        assert_eq!(fixed_version("Upgrade to version 2.1.4 or above."), "2.1.4");
        assert_eq!(fixed_version("Upgrade to versions 1.2.3, 2.0.1 or above."), "1.2.3");
        assert_eq!(fixed_version("Unfortunately there is no solution available yet."), "");
        // GitLab's id on one instance is never another instance's.
        assert_ne!(finding_id("https://gitlab.example.com", "7001"), finding_id("https://gitlab.other.example.com", "7001"));
        assert_eq!(finding_id("https://gitlab.example.com", "7001").len(), 32);
    }

    #[test]
    fn unauthorised_and_missing_projects_are_reported() {
        let mut no_role = fixtures::vulnerabilities_page(false, "x");
        no_role["project"]["vulnerabilities"] = Value::Null;
        assert!(matches!(parse_page(&no_role), Err(GitlabError::Auth(m)) if m.contains("Developer")));
        assert!(matches!(parse_page(&json!({"project": null})), Err(GitlabError::NotFound(_))));
    }

    #[tokio::test]
    async fn an_import_reads_every_page_and_the_dependency_list() {
        let g = FakeGitlab::default();
        g.answer(fixtures::graphql_key(PATH, None), fixtures::vulnerabilities_page(true, "c1"));
        let mut second = fixtures::vulnerabilities_page(false, "c2");
        second["project"]["vulnerabilities"]["nodes"] = json!([]);
        g.answer(fixtures::graphql_key(PATH, Some("c1")), second);
        g.answer(api::dependencies_path(4211, 1), fixtures::dependencies());
        let imp = import_project(&g, &conn(), 4211, PATH).await.unwrap();
        assert!(imp.complete);
        assert_eq!((imp.vulns.len(), imp.skipped), (5, 1));
        assert_eq!(imp.project.repository, "gitlab.example.com/example-org/storefront-api");
        // The dependency list, deduplicated with the vulnerable package; PyPI names normalized.
        let names: Vec<(&str, &str)> = imp.libraries.iter().map(|l| (l.ecosystem.as_str(), l.name.as_str())).collect();
        assert_eq!(names, vec![("npm", "example-merge"), ("npm", "example-router"), ("PyPI", "example-http")]);
        assert_eq!(imp.libraries[0].licenses, vec!["MIT"]);
    }

    #[tokio::test]
    async fn an_import_cut_at_the_page_limit_is_partial_and_a_refused_dependency_list_is_skipped() {
        let g = FakeGitlab::default();
        g.answer(fixtures::graphql_key(PATH, None), fixtures::vulnerabilities_page(true, "c1"));
        g.answer(fixtures::graphql_key(PATH, Some("c1")), fixtures::vulnerabilities_page(true, "c1"));
        let imp = import_project(&g, &conn(), 4211, PATH).await.unwrap();
        assert!(!imp.complete);
        assert_eq!(g.requests.lock().unwrap().iter().filter(|r| r.starts_with("graphql")).count(), MAX_PAGES);
        // No dependency list (not Ultimate, or refused): the vulnerable package only.
        assert_eq!(imp.libraries.len(), 1);
    }

    #[tokio::test]
    async fn throttling_fails_the_import() {
        let g = FakeGitlab { throttle_after: Some(0), ..Default::default() };
        assert!(matches!(import_project(&g, &conn(), 4211, PATH).await, Err(GitlabError::Throttled(_))));
    }

    /// Re-imports update in place and close what GitLab no longer lists; uploads are never touched; another org
    /// sees nothing (skipped without a live database).
    #[tokio::test]
    async fn reimports_update_in_place_and_close_what_disappeared() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "cs-gitlab").await;
        let other = crate::test_support::new_org(st, "cs-gitlab-other").await;
        let base = "https://gitlab.example.com";
        let (project, vulns, _) = page();
        let keep: Vec<GitlabVuln> = vulns.into_iter().filter(|v| v.kind != "container").collect();
        let imp = ProjectImport { project: project.clone(), vulns: keep.clone(), complete: true, libraries: vec![], skipped: 1 };
        assert_eq!(store_import(st, &org, base, &imp).await.unwrap(), 5);
        let state = |id: &str| {
            let id = finding_id(base, id);
            let org = org.clone();
            async move {
                sqlx::query_as::<_, (String, String, bool)>(
                    "SELECT upstream_state, upstream_reason, resolved_at IS NOT NULL FROM cs_findings WHERE org_id = $1 AND id = $2 UNION ALL \
                     SELECT upstream_state, upstream_reason, resolved_at IS NOT NULL FROM cs_dependency_findings WHERE org_id = $1 AND id = $2",
                )
                .bind(&org)
                .bind(&id)
                .fetch_optional(&st.postgres)
                .await
                .unwrap()
            }
        };
        assert_eq!(state("7003").await, Some(("dismissed".into(), "Used in tests".into(), false)));
        assert_eq!(state("7005").await.map(|s| s.2), Some(true));
        assert_eq!(state("7004").await.map(|s| s.0), Some("confirmed".into()));

        // An upload of the same tool to the same repository leaves GitLab's findings alone.
        let ctx = model::ScanContext { repository: project.repository.clone(), branch: "main".into(), ..Default::default() };
        let report = json!({ "runs": [{ "tool": { "driver": { "name": "semgrep" } }, "results": [] }] });
        crate::routes::code_security::store_sarif(st, &org, &ctx, &report).await.unwrap();
        assert_eq!(state("7001").await.map(|s| s.2), Some(false));

        // GitLab no longer lists 7001 and 7004, and 7003 was reopened: closed, updated.
        let mut later: Vec<GitlabVuln> = keep.into_iter().filter(|v| v.external_id != "7001" && v.external_id != "7004").collect();
        for v in later.iter_mut().filter(|v| v.external_id == "7003") {
            v.state = "detected".into();
            v.reason = String::new();
        }
        store_import(st, &org, base, &ProjectImport { vulns: later.clone(), ..imp.clone() }).await.unwrap();
        assert_eq!(state("7001").await.map(|s| s.2), Some(true));
        assert_eq!(state("7004").await.map(|s| s.2), Some(true));
        assert_eq!(state("7003").await, Some(("detected".into(), String::new(), false)));
        // A partial import closes nothing.
        store_import(st, &org, base, &ProjectImport { vulns: vec![], complete: false, ..imp.clone() }).await.unwrap();
        assert_eq!(state("7003").await.map(|s| s.2), Some(false));

        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cs_findings WHERE org_id = $1 AND source = 'gitlab'").bind(&other).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn vulnerabilities_are_imported_every_six_hours() {
        // Each import reads every vulnerability of the project through GraphQL (its completeness resolves
        // what GitLab no longer lists), heavy on GitLab's database every 30 minutes.
        assert_eq!(INTERVAL_SECS, 6 * 3600);
    }
}
