//! GitLab REST API answers (fictional `example-org` projects) for the GitLab CI tests.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};

use super::api::{GitlabApi, GitlabError};

pub fn project() -> Value {
    json!({
        "id": 4211, "name": "storefront-api", "path": "storefront-api", "path_with_namespace": "example-org/storefront-api",
        "web_url": "https://gitlab.example.com/example-org/storefront-api", "default_branch": "main"
    })
}

/// A finished pipeline (`GET /projects/:id/pipelines/:pipeline_id`).
pub fn pipeline(id: u64, status: &str, updated_at: &str) -> Value {
    json!({
        "id": id, "iid": 812, "project_id": 4211, "name": null, "sha": "9fceb02d0ae598e95dc970b74767f19372d61af8", "ref": "main",
        "status": status, "source": "push", "created_at": "2026-11-03T10:00:00.000Z", "updated_at": updated_at,
        "started_at": "2026-11-03T10:00:05.000Z", "finished_at": "2026-11-03T10:06:40.000Z", "duration": 395, "queued_duration": 5,
        "web_url": format!("https://gitlab.example.com/example-org/storefront-api/-/pipelines/{id}")
    })
}

fn job(id: u64, name: &str, stage: &str, status: &str, started: Option<&str>, finished: Option<&str>) -> Value {
    json!({
        "id": id, "name": name, "stage": stage, "status": status, "allow_failure": false, "failure_reason": null,
        "created_at": "2026-11-03T10:00:00.000Z", "started_at": started, "finished_at": finished, "duration": 60.5, "queued_duration": 1.2,
        "web_url": format!("https://gitlab.example.com/example-org/storefront-api/-/jobs/{id}"),
        "runner": { "id": 37, "description": "shared-runner-linux-1", "is_shared": true, "runner_type": "instance_type" },
        "commit": {
            "id": "9fceb02d0ae598e95dc970b74767f19372d61af8", "title": "Fix rounding in totals", "message": "Fix rounding in totals\n\nDetails.",
            "author_name": "Jordan Patel", "author_email": "jordan.patel@example.com", "created_at": "2026-11-03T09:40:00.000+00:00"
        }
    })
}

/// The pipeline's jobs: build, a failing allowed lint, a deploy and a manual job that never ran.
pub fn jobs() -> Value {
    let mut lint = job(9002, "lint", "test", "failed", Some("2026-11-03T10:00:10.000Z"), Some("2026-11-03T10:00:40.000Z"));
    lint["allow_failure"] = json!(true);
    lint["failure_reason"] = json!("script_failure");
    json!([
        job(9001, "build", "build", "success", Some("2026-11-03T10:00:05.000Z"), Some("2026-11-03T10:02:05.000Z")),
        lint,
        job(9003, "deploy-production", "deploy", "success", Some("2026-11-03T10:05:00.000Z"), Some("2026-11-03T10:06:40.000Z")),
        job(9004, "rollback", "deploy", "manual", None, None),
    ])
}

/// The project's deployments: job 9003 deployed to production.
pub fn deployments() -> Value {
    json!([{ "id": 77, "status": "success", "updated_at": "2026-11-03T10:06:40.000Z", "environment": { "id": 5, "name": "production" }, "deployable": { "id": 9003, "name": "deploy-production" } }])
}

/// GitLab answering from a table of paths; counts requests; `throttle_after` answers 429 past that many.
#[derive(Default)]
pub struct FakeGitlab {
    pub answers: StdMutex<HashMap<String, Value>>,
    pub requests: StdMutex<Vec<String>>,
    pub throttle_after: Option<usize>,
}

impl FakeGitlab {
    pub fn answer(&self, path: String, v: Value) {
        self.answers.lock().unwrap().insert(path, v);
    }
    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

/// The key a GraphQL request is answered under: the project path and the page cursor.
pub fn graphql_key(full_path: &str, after: Option<&str>) -> String {
    format!("graphql {full_path} {}", after.unwrap_or("-"))
}

fn identifier(kind: &str, id: &str, name: &str, url: &str) -> Value {
    json!({ "externalType": kind, "externalId": id, "name": name, "url": url })
}

/// One vulnerability of GitLab's GraphQL `Vulnerability` type (the fields the import asks for).
#[allow(clippy::too_many_arguments)]
fn vulnerability(id: u64, report_type: &str, scanner: (&str, &str), title: &str, severity: &str, state: &str, identifiers: Vec<Value>, location: Value) -> Value {
    json!({
        "id": format!("gid://gitlab/Vulnerability/{id}"), "title": title, "description": format!("{title}: description written by the scanner."),
        "severity": severity, "state": state, "reportType": report_type, "solution": null,
        "webUrl": format!("https://gitlab.example.com/example-org/storefront-api/-/security/vulnerabilities/{id}"),
        "detectedAt": "2026-10-01T08:00:00Z", "dismissedAt": null, "resolvedAt": null, "dismissalReason": null,
        "scanner": { "externalId": scanner.0, "name": scanner.1, "vendor": "GitLab" },
        "primaryIdentifier": identifiers[0].clone(), "identifiers": identifiers, "location": location
    })
}

/// A page of `project.vulnerabilities` for `example-org/storefront-api`: one of each report type the import reads
/// (Semgrep SAST, KICS IaC, a dismissed Gitleaks secret, a Gemnasium dependency, a resolved SAST finding) and a
/// Trivy container finding it leaves out.
pub fn vulnerabilities_page(has_next: bool, cursor: &str) -> Value {
    let sast = |file: &str, line: &str| json!({ "__typename": "VulnerabilityLocationSast", "file": file, "startLine": line });
    let mut secret = vulnerability(
        7003, "SECRET_DETECTION", ("gitleaks", "Gitleaks"), "AWS access token", "CRITICAL", "DISMISSED",
        vec![identifier("gitleaks_rule_id", "AWS", "Gitleaks rule ID AWS", "")],
        json!({ "__typename": "VulnerabilityLocationSecretDetection", "file": "tests/fixtures/aws.env", "startLine": "3" }),
    );
    secret["dismissedAt"] = json!("2026-10-05T09:00:00Z");
    secret["dismissalReason"] = json!("USED_IN_TESTS");
    let mut dependency = vulnerability(
        7004, "DEPENDENCY_SCANNING", ("gemnasium", "Gemnasium"), "Prototype pollution in example-merge", "HIGH", "CONFIRMED",
        vec![identifier("gemnasium", "8c3f2a10-1b2c-4d5e-8f90-a1b2c3d4e5f6", "Gemnasium-8c3f2a10", ""), identifier("cve", "CVE-2026-90010", "CVE-2026-90010", "https://cve.example.com/CVE-2026-90010"), identifier("cwe", "1321", "CWE-1321", "")],
        json!({ "__typename": "VulnerabilityLocationDependencyScanning", "file": "package-lock.json", "dependency": { "package": { "name": "example-merge" }, "version": "2.1.0" } }),
    );
    dependency["solution"] = json!("Upgrade to version 2.1.4 or above.");
    let mut resolved = vulnerability(
        7005, "SAST", ("semgrep", "Semgrep"), "Use of a broken hash (MD5)", "MEDIUM", "RESOLVED",
        vec![identifier("semgrep_id", "bandit.B303-1", "bandit.B303-1", ""), identifier("cwe", "327", "CWE-327", "https://cwe.mitre.org/data/definitions/327.html")],
        sast("app/utils/hash.py", "12"),
    );
    resolved["resolvedAt"] = json!("2026-10-20T10:00:00Z");
    json!({ "project": {
        "id": "gid://gitlab/Project/4211", "fullPath": "example-org/storefront-api", "webUrl": "https://gitlab.example.com/example-org/storefront-api",
        "repository": { "rootRef": "main" },
        "vulnerabilities": {
            "pageInfo": { "hasNextPage": has_next, "endCursor": cursor },
            "nodes": [
                vulnerability(7001, "SAST", ("semgrep", "Semgrep"), "Improper neutralization of special elements in an SQL command", "HIGH", "DETECTED",
                    vec![identifier("semgrep_id", "python.django.security.injection.sql.sql-injection-using-raw", "sql-injection-using-raw", ""), identifier("cwe", "89", "CWE-89", "https://cwe.mitre.org/data/definitions/89.html")],
                    sast("app/orders/views.py", "41")),
                vulnerability(7002, "SAST", ("kics", "KICS"), "S3 bucket allows public read", "HIGH", "DETECTED",
                    vec![identifier("kics_id", "57b9893d-33b1-4419-bcea-a717ea87e139", "S3 bucket ACL allows read to all users", "https://docs.kics.io/latest/queries/terraform-queries/")],
                    sast("infra/s3.tf", "12")),
                secret,
                dependency,
                resolved,
                vulnerability(7006, "CONTAINER_SCANNING", ("trivy", "Trivy"), "CVE-2026-90020 in libexample", "HIGH", "DETECTED",
                    vec![identifier("cve", "CVE-2026-90020", "CVE-2026-90020", "")],
                    json!({ "__typename": "VulnerabilityLocationContainerScanning", "image": "registry.example.com/storefront-api:1.4.0", "operatingSystem": "debian 12", "dependency": { "package": { "name": "libexample" }, "version": "1.0-1" } })),
            ]
        }
    } })
}

/// GitLab's dependency list of the project (`GET /projects/:id/dependencies`).
pub fn dependencies() -> Value {
    json!([
        { "name": "example-merge", "version": "2.1.0", "package_manager": "npm", "dependency_file_path": "package-lock.json", "licenses": [{ "name": "MIT", "url": "" }] },
        { "name": "example-router", "version": "4.2.0", "package_manager": "npm", "dependency_file_path": "package-lock.json", "licenses": [] },
        { "name": "Example_Http", "version": "2.31.0", "package_manager": "pip", "dependency_file_path": "requirements.txt", "licenses": [] },
    ])
}

#[async_trait::async_trait]
impl GitlabApi for FakeGitlab {
    async fn get(&self, _base: &str, token: &str, path: &str) -> Result<Value, GitlabError> {
        assert_eq!(token, "glpat-test");
        let n = {
            let mut r = self.requests.lock().unwrap();
            r.push(path.to_string());
            r.len()
        };
        if self.throttle_after.is_some_and(|t| n > t) {
            return Err(GitlabError::Throttled("The provider answered: HTTP 429".into()));
        }
        self.answers.lock().unwrap().get(path).cloned().ok_or_else(|| GitlabError::NotFound(format!("no fixture for {path}")))
    }

    async fn graphql(&self, base: &str, token: &str, _query: &str, variables: Value) -> Result<Value, GitlabError> {
        let key = graphql_key(variables["fullPath"].as_str().unwrap_or_default(), variables["after"].as_str());
        self.get(base, token, &key).await
    }
}

