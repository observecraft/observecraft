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
}

