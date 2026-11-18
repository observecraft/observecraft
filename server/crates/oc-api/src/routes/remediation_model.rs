//! Remediation's pure model (ADR-0008, levels Explain and Fix in code): which level a finding gets and why,
//! which of the org's source code connections reaches its repository and whether its token may write, and
//! the merge request's description. No I/O; `remediation.rs` loads the findings and talks to the forges.

use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

use super::remediation_manifests::manifest_candidates;
use crate::assist::code::ForgeKind;

/// What can be done about a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = RemediationLevel)]
pub enum Level {
    /// An explanation and general remediation steps.
    Explain,
    /// A change to the repository, reviewed as a merge request.
    FixInCode,
    /// A change to the infrastructure code that defines the resource (its provenance), reviewed as a merge request.
    FixInInfra,
    /// Fixable once the resource's source (repository and file) is known: tagging policies.
    NeedsProvenance,
}

/// The kinds of findings remediation knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Library,
    Sast,
    Secret,
    Iac,
    CloudMisconfiguration,
    CloudVulnerability,
    /// A resource lacking the tags that name its source (Tag Coverage → Add tags with Assist).
    MissingTags,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Library => "Library vulnerability",
            Kind::Sast => "Code vulnerability",
            Kind::Secret => "Secret",
            Kind::Iac => "IaC misconfiguration",
            Kind::CloudMisconfiguration => "Cloud misconfiguration",
            Kind::CloudVulnerability => "Infrastructure vulnerability",
            Kind::MissingTags => "Resource missing provenance tags",
        }
    }

    /// A static finding's kind (`cs_findings.kind`).
    pub fn of_static(kind: &str) -> Kind {
        match kind {
            "secret" => Kind::Secret,
            "iac" => Kind::Iac,
            _ => Kind::Sast,
        }
    }
}

/// Where a library is declared: a repository and the file a scanner reported (a manifest or a lockfile).
#[derive(Debug, Clone, PartialEq)]
pub struct Declared {
    pub repository: String,
    pub file: String,
}

/// What a finding says about where it comes from.
#[derive(Debug, Clone)]
pub struct Facts {
    pub kind: Kind,
    /// Static findings: the repository, file and line.
    pub repository: String,
    pub file: String,
    /// Libraries: the fixed version and where the library is declared (none: only seen at runtime).
    pub fixed: Option<String>,
    pub declared: Vec<Declared>,
}

/// A repository and the file to change.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FixTarget {
    /// As Code Security names it (`github.com/example-org/storefront`).
    pub repository: String,
    /// The finding's file, or the manifests to try for a library (`dir/*.csproj`: any project file there).
    pub files: Vec<String>,
}

/// The level that applies, why, and what a fix would change.
pub fn level(f: &Facts) -> (Level, String, Vec<FixTarget>) {
    match f.kind {
        Kind::MissingTags => (Level::Explain, "Tags are added from Tag Coverage, to a repository you pick.".into(), vec![]),
        Kind::CloudMisconfiguration | Kind::CloudVulnerability => (
            Level::NeedsProvenance,
            "This finding names a cloud resource, not the code that creates it. Assist can explain it; a fix in code needs the resource's provenance (tags or labels naming its repository and file), which Tagging Policies define.".into(),
            vec![],
        ),
        Kind::Library => {
            let targets: Vec<FixTarget> = f
                .declared
                .iter()
                .filter(|d| !d.repository.is_empty())
                .map(|d| FixTarget { repository: d.repository.clone(), files: manifest_candidates(&d.file) })
                .filter(|t| !t.files.is_empty())
                .fold(Vec::new(), |mut acc, t| {
                    if !acc.iter().any(|x: &FixTarget| x.repository == t.repository && x.files == t.files) {
                        acc.push(t);
                    }
                    acc
                });
            if f.declared.iter().all(|d| d.repository.is_empty()) {
                return (
                    Level::NeedsProvenance,
                    "Only running services load this library: no repository is known to declare it. With git metadata on the services (DD_GIT_REPOSITORY_URL, or a repository upload of its dependencies) it can be fixed in code; Tagging Policies list what's missing.".into(),
                    vec![],
                );
            }
            if f.fixed.is_none() {
                return (Level::Explain, "No fixed version is published for this advisory yet, so there is nothing to upgrade to.".into(), vec![]);
            }
            if targets.is_empty() {
                return (Level::Explain, "The repositories declare it in files fixes aren't written for (no manifest we can edit beside them).".into(), vec![]);
            }
            let to = f.fixed.as_deref().unwrap_or_default();
            (Level::FixInCode, format!("The manifest can be bumped to the first fixed version, {to}, without running the package manager."), targets)
        }
        Kind::Sast | Kind::Secret | Kind::Iac => {
            if f.repository.is_empty() || f.file.is_empty() {
                return (Level::Explain, "The finding has no repository and file to change.".into(), vec![]);
            }
            let why = match f.kind {
                Kind::Secret => "Assist proposes a change to the file that removes the secret from the code; the credential must be rotated as well.",
                Kind::Iac => "Assist proposes a change to the file that fixes the misconfiguration.",
                _ => "Assist proposes a change to the file that fixes the vulnerability.",
            };
            (Level::FixInCode, why.into(), vec![FixTarget { repository: f.repository.clone(), files: vec![f.file.clone()] }])
        }
    }
}

// ---------------------------------------------------------------- connections

/// The host a connection's API serves repositories for (`https://api.github.com` → `github.com`,
/// `https://ghe.example.com/api/v3` → `ghe.example.com`).
pub fn repository_host(base_url: &str) -> Option<String> {
    let host = url::Url::parse(base_url).ok()?.host_str()?.to_ascii_lowercase();
    Some(if host == "api.github.com" { "github.com".into() } else { host })
}

/// The repository as `(host, owner/name)` (`gitlab.example.com/platform/order-service` → group path kept).
pub fn split_repository(repository: &str) -> Option<(String, String)> {
    let (host, path) = repository.trim().trim_end_matches('/').split_once('/')?;
    crate::assist::code::check_repo(path).ok()?;
    Some((host.to_ascii_lowercase(), path.to_string()))
}

/// Which of the org's connections (GitHub's and GitLab's API base URLs) reaches the repository, and its path there;
/// `Err` says which connection is missing.
pub fn connection_for(repository: &str, github: Option<&str>, gitlab: Option<&str>) -> Result<(ForgeKind, String), String> {
    let (host, path) = split_repository(repository).ok_or_else(|| format!("{repository} isn't a repository on a source code host"))?;
    for (kind, base) in [(ForgeKind::Github, github), (ForgeKind::Gitlab, gitlab)] {
        if base.and_then(repository_host).as_deref() == Some(host.as_str()) {
            return Ok((kind, path));
        }
    }
    let gitlab_hosts: Vec<String> = gitlab.and_then(repository_host).into_iter().collect();
    let provider = match super::code_security::repository_link(repository, &gitlab_hosts).0 {
        "github" => "GitHub",
        "gitlab" => "GitLab",
        _ => "GitHub or GitLab",
    };
    Err(format!("No {provider} connection reaches {host}: add one in Developer Portal → Settings. Read access lets Assist propose a fix; opening a merge request also needs write access."))
}

/// Why the connection's token can't open a merge request on the repository (none: it can, or we can't tell).
/// GitHub answers the token's permissions on the repository; GitLab the token's scopes and the project's access level.
pub fn missing_write(kind: ForgeKind, repo: &Value, scopes: Option<&[String]>) -> Option<String> {
    match kind {
        ForgeKind::Github => (repo.pointer("/permissions/push") == Some(&Value::Bool(false))).then(|| {
            "The GitHub token can read but not write this repository: give it the `repo` scope (classic token) or Contents and Pull requests: Read and write (fine-grained token).".to_string()
        }),
        ForgeKind::Gitlab => {
            if let Some(scopes) = scopes {
                if !scopes.iter().any(|s| s == "api") {
                    let has = if scopes.is_empty() { "none".to_string() } else { scopes.join(", ") };
                    return Some(format!("The GitLab token lacks the `api` scope, which opening a merge request needs (it has: {has})."));
                }
            }
            let level = ["/permissions/project_access/access_level", "/permissions/group_access/access_level"].iter().filter_map(|p| repo.pointer(p).and_then(Value::as_i64)).max();
            match level {
                Some(l) if l < 30 => Some("The GitLab token's user needs at least the Developer role on the project to push a branch and open a merge request.".into()),
                _ => None,
            }
        }
    }
}

/// Why a repository can't be read. GitLab and GitHub answer 404, not 403, for a repository the
/// connection's token has no access to, which "not found at this URL" would misdescribe.
pub fn unreadable(kind: ForgeKind, path: &str, status: Option<u16>, error: &str) -> String {
    match (kind, status) {
        (ForgeKind::Gitlab, Some(404)) => format!(
            "The GitLab connection's token can't see {path}: GitLab answers 404 for a project the token has no access to. Give the token's user (or the group or project access token) at least the Reporter role on it to read it, Developer to open merge requests, or check the repository's path."
        ),
        (ForgeKind::Github, Some(404)) => format!(
            "The GitHub connection's token can't see {path}: GitHub answers 404 for a repository the token has no access to. Give it the `repo` scope (classic token) or access to this repository with Contents and Pull requests: Read and write (fine-grained token), or check the repository's path."
        ),
        _ => format!("Can't read {path}: {error}"),
    }
}

// ---------------------------------------------------------------- the merge request

/// What the merge request's description says.
pub struct Description<'a> {
    pub kind: Kind,
    pub title: &'a str,
    pub severity: &'a str,
    /// The rule id or the advisory and its CVE.
    pub ids: &'a [String],
    pub location: &'a str,
    /// What the change does (Markdown).
    pub change: &'a str,
    /// What must still happen (regenerating a lockfile, rotating a credential).
    pub notes: &'a [String],
    /// The finding in ObserveCraft.
    pub link: &'a str,
    /// Infrastructure fixes: the resource and its source (Markdown list items).
    pub source: &'a str,
    /// What the repository's CI must show (empty: the generic note).
    pub ci: &'a str,
}

pub fn merge_request_description(d: &Description) -> String {
    let mut s = format!("Fixes a {} found by ObserveCraft: **{}**", d.kind.label().to_lowercase(), d.title.trim());
    if !d.severity.is_empty() {
        s.push_str(&format!(" ({} severity)", d.severity));
    }
    s.push_str(".\n\n");
    if !d.ids.is_empty() {
        s.push_str(&format!("- {}: {}\n", if d.kind == Kind::Library { "Advisory" } else { "Rule" }, d.ids.iter().map(|i| format!("`{i}`")).collect::<Vec<_>>().join(", ")));
    }
    if !d.location.is_empty() {
        s.push_str(&format!("- Location: `{}`\n", d.location));
    }
    s.push_str(d.source);
    s.push_str(&format!("\n### Change\n\n{}\n", d.change.trim()));
    let mut notes: Vec<String> = d.notes.to_vec();
    if d.kind == Kind::Secret && !notes.iter().any(|n| n.contains("otate")) {
        notes.push(SECRET_NOTE.into());
    }
    notes.push(if d.ci.is_empty() { "The repository's CI must pass before this is merged: review the change and the pipeline's results. Nothing is merged automatically.".into() } else { d.ci.to_string() });
    s.push_str("\n### Before merging\n\n");
    for n in notes {
        s.push_str(&format!("- {n}\n"));
    }
    s.push_str(&format!("\n---\nProposed from the finding in ObserveCraft: {}\n", d.link));
    s
}

pub const SECRET_NOTE: &str = "Rotate the credential: removing it from the code doesn't revoke it, and it stays in the repository's history.";

/// A note when the fix is a new major version.
pub fn major_note(package: &str, from: &str, to: &str) -> Option<String> {
    let major = |v: &str| v.trim_start_matches('v').split(['.', '-']).next().unwrap_or("").to_string();
    (major(from) != major(to) && !major(from).is_empty()).then(|| format!("{package} {to} is a new major version (from {from}): check its release notes for breaking changes."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(kind: Kind) -> Facts {
        Facts { kind, repository: String::new(), file: String::new(), fixed: None, declared: vec![] }
    }

    #[test]
    fn cloud_findings_need_provenance_and_can_be_explained() {
        for k in [Kind::CloudMisconfiguration, Kind::CloudVulnerability] {
            // (remediation_infra decides the level once the resource's provenance is known.)
            let (level, why, targets) = level(&facts(k));
            assert_eq!(level, Level::NeedsProvenance);
            assert!(why.contains("Tagging Policies") && targets.is_empty());
        }
    }

    #[test]
    fn libraries_are_fixed_in_their_manifest_when_a_fix_exists() {
        let declared = |repo: &str, file: &str| Declared { repository: repo.into(), file: file.into() };
        let mut f = facts(Kind::Library);
        f.fixed = Some("4.17.21".into());
        f.declared = vec![declared("", ""), declared("github.com/example-org/storefront", "web/package-lock.json"), declared("github.com/example-org/storefront", "web/yarn.lock")];
        let (level, why, targets) = level(&f);
        assert_eq!(level, Level::FixInCode, "{why}");
        assert_eq!(targets, vec![FixTarget { repository: "github.com/example-org/storefront".into(), files: vec!["web/package.json".into()] }], "one target per manifest");
        // Runtime only: no repository is known.
        let mut runtime = f.clone();
        runtime.declared = vec![declared("", "")];
        assert_eq!(super::level(&runtime).0, Level::NeedsProvenance);
        // No fix published.
        let mut unfixed = f.clone();
        unfixed.fixed = None;
        assert_eq!(super::level(&unfixed).0, Level::Explain);
        // A file we can't map to a manifest.
        let mut odd = f.clone();
        odd.declared = vec![declared("github.com/example-org/storefront", "vendor/modules.txt")];
        assert_eq!(super::level(&odd).0, Level::Explain);
    }

    #[test]
    fn static_findings_are_fixed_in_their_file() {
        let mut f = facts(Kind::Secret);
        assert_eq!(level(&f).0, Level::Explain, "no location");
        f.repository = "gitlab.example.com/platform/billing".into();
        f.file = "config/settings.py".into();
        let (level, why, targets) = level(&f);
        assert_eq!(level, Level::FixInCode);
        assert!(why.contains("rotated"));
        assert_eq!(targets[0].files, vec!["config/settings.py"]);
    }

    #[test]
    fn a_repository_is_reached_by_the_connection_for_its_host() {
        assert_eq!(repository_host("https://api.github.com"), Some("github.com".into()));
        assert_eq!(repository_host("https://ghe.example.com/api/v3"), Some("ghe.example.com".into()));
        let gh = Some("https://api.github.com");
        let gl = Some("https://gitlab.example.com");
        assert_eq!(connection_for("github.com/example-org/storefront", gh, gl), Ok((ForgeKind::Github, "example-org/storefront".into())));
        assert_eq!(connection_for("gitlab.example.com/platform/payments/billing", gh, gl), Ok((ForgeKind::Gitlab, "platform/payments/billing".into())));
        let missing = connection_for("gitlab.com/example-org/catalog-service", gh, gl).unwrap_err();
        assert!(missing.contains("No GitLab connection reaches gitlab.com") && missing.contains("Developer Portal"), "{missing}");
        assert!(connection_for("github.com/example-org/storefront", None, None).unwrap_err().contains("No GitHub connection"));
        assert!(connection_for("storefront", gh, gl).is_err());
    }

    #[test]
    fn write_access_names_the_missing_scope() {
        assert_eq!(missing_write(ForgeKind::Github, &json!({"permissions": {"pull": true, "push": true}}), None), None);
        assert!(missing_write(ForgeKind::Github, &json!({"permissions": {"pull": true, "push": false}}), None).unwrap().contains("Contents and Pull requests"));
        assert_eq!(missing_write(ForgeKind::Github, &json!({}), None), None, "unknown: try");
        let read_only = vec!["read_api".to_string(), "read_repository".to_string()];
        let why = missing_write(ForgeKind::Gitlab, &json!({}), Some(&read_only)).unwrap();
        assert!(why.contains("`api` scope") && why.contains("read_api, read_repository"), "{why}");
        let api = vec!["api".to_string()];
        assert_eq!(missing_write(ForgeKind::Gitlab, &json!({"permissions": {"project_access": {"access_level": 40}}}), Some(&api)), None);
        assert!(missing_write(ForgeKind::Gitlab, &json!({"permissions": {"project_access": {"access_level": 20}, "group_access": null}}), Some(&api)).unwrap().contains("Developer role"));
    }

    #[test]
    fn the_description_names_the_finding_the_change_and_ci() {
        let ids = vec!["GHSA-35jh-r3h4-6jhm".to_string(), "CVE-2021-23337".to_string()];
        let notes = vec!["Regenerate the lockfile.".to_string()];
        let d = merge_request_description(&Description {
            kind: Kind::Library,
            title: "Command injection in lodash",
            severity: "high",
            ids: &ids,
            location: "web/package.json",
            change: "Bumps lodash from ^4.17.20 to ^4.17.21.",
            notes: &notes,
            link: "https://observecraft.example.com/code-security/vulnerabilities/libraries",
            source: "",
            ci: "",
        });
        assert!(d.starts_with("Fixes a library vulnerability found by ObserveCraft: **Command injection in lodash** (high severity)."), "{d}");
        assert!(d.contains("- Advisory: `GHSA-35jh-r3h4-6jhm`, `CVE-2021-23337`") && d.contains("- Location: `web/package.json`"));
        assert!(d.contains("### Change\n\nBumps lodash") && d.contains("- Regenerate the lockfile."));
        assert!(d.contains("CI must pass") && d.contains("Nothing is merged automatically"));
        let secret = merge_request_description(&Description { kind: Kind::Secret, title: "AWS access key", severity: "critical", ids: &[], location: "config/settings.py:12", change: "Reads the key from the environment.", notes: &[], link: "x", source: "", ci: "" });
        assert!(secret.contains(SECRET_NOTE));
    }

    #[test]
    fn major_upgrades_are_called_out() {
        assert!(major_note("jackson-databind", "2.13.0", "3.0.0").unwrap().contains("new major version"));
        assert_eq!(major_note("lodash", "4.17.20", "4.17.21"), None);
        assert_eq!(major_note("golang.org/x/net", "v0.17.0", "v0.23.0"), None);
    }

    #[test]
    fn an_infrastructure_fix_names_its_source_and_the_ci_that_checks_it() {
        let ids = vec!["s3_bucket_level_public_access_block".to_string()];
        let source = "- Resource: `arn:aws:s3:::example-shop-assets`\n- Provenance: exact, via tags\n";
        let d = merge_request_description(&Description {
            kind: Kind::CloudMisconfiguration,
            title: "S3 bucket allows public ACLs",
            severity: "high",
            ids: &ids,
            location: "",
            change: "Blocks public access.",
            notes: &[],
            link: "x",
            source,
            ci: "`terraform plan` must show only the intended change.",
        });
        assert!(d.starts_with("Fixes a cloud misconfiguration found by ObserveCraft") && d.contains("- Rule: `s3_bucket_level_public_access_block`\n- Resource: `arn:aws:s3:::example-shop-assets`"), "{d}");
        assert!(d.contains("- `terraform plan` must show only the intended change.") && !d.contains("review the change and the pipeline"), "{d}");
        assert_eq!(Kind::MissingTags.label(), "Resource missing provenance tags");
    }

    #[test]
    fn a_repository_the_token_cant_see_says_so_rather_than_a_wrong_url() {
        // GitLab and GitHub answer 404, not 403, for a project the token has no access to.
        let gl = unreadable(ForgeKind::Gitlab, "example-org/storefront-api", Some(404), "The provider answered: the API was not found at this URL (404 Project Not Found)");
        assert!(gl.contains("example-org/storefront-api") && gl.contains("GitLab connection's token can't see") && gl.contains("Developer"), "{gl}");
        let gh = unreadable(ForgeKind::Github, "example-org/storefront-api", Some(404), "The provider answered: the API was not found at this URL (Not Found)");
        assert!(gh.contains("GitHub connection's token can't see") && gh.contains("Contents"), "{gh}");
        // Other failures keep the provider's own words.
        assert_eq!(unreadable(ForgeKind::Gitlab, "example-org/storefront-api", Some(401), "The provider answered: the credentials were refused"), "Can't read example-org/storefront-api: The provider answered: the credentials were refused");
        assert_eq!(unreadable(ForgeKind::Gitlab, "example-org/storefront-api", None, "timed out"), "Can't read example-org/storefront-api: timed out");
    }
}
