//! Synthetic CI/CD pipeline runs, sent as OTLP traces like a CI provider's OpenTelemetry exporter:
//! one trace per run (pipeline → jobs → steps, with test cases under test jobs) using the OTel
//! `cicd.*` / `vcs.*` / `test.*` attributes plus a few ObserveCraft extensions documented in the
//! web app's CI settings page (`ci.stage.name`, `ci.step.name`, `vcs.revision.*`, `team`).
//!
//! Each run lands right after it "finishes". Flaky tests fail now and then and the run is retried on
//! the same commit, so flaky-test detection has real data; deploy pipelines on `main` produce the
//! deployments behind the DORA metrics. On first start a week of history is backfilled.

use serde_json::{json, Value};

use super::{attrs_json, Rng};

struct Repo {
    name: &'static str,
    service: &'static str,
    team: &'static str,
    provider: &'static str,
    suite: &'static str,
    /// Chance that a commit genuinely breaks the build.
    fail_rate: f64,
    /// Tests that sometimes fail for no reason.
    flaky: &'static [&'static str],
    infra: bool,
}

const REPOS: &[Repo] = &[
    Repo { name: "example-org/storefront-api", service: "storefront-api", team: "storefront", provider: "github_actions", suite: "StorefrontApiTest", fail_rate: 0.08, flaky: &["returns cached product page", "paginates search results"], infra: false },
    Repo { name: "example-org/order-service", service: "order-service", team: "checkout", provider: "github_actions", suite: "OrderServiceTest", fail_rate: 0.12, flaky: &["reserves inventory before payment"], infra: false },
    Repo { name: "example-org/billing", service: "billing", team: "billing", provider: "github_actions", suite: "InvoiceTest", fail_rate: 0.05, flaky: &[], infra: false },
    Repo { name: "example-org/catalog-service", service: "catalog-service", team: "catalog", provider: "github_actions", suite: "CatalogTest", fail_rate: 0.07, flaky: &["reindexes changed prices"], infra: false },
    Repo { name: "example-org/inventory-service", service: "inventory-service", team: "checkout", provider: "github_actions", suite: "StockLevelTest", fail_rate: 0.06, flaky: &["releases expired reservations"], infra: false },
    Repo { name: "example-org/notification-worker", service: "notification-worker", team: "notifications", provider: "github_actions", suite: "test_notifications", fail_rate: 0.1, flaky: &["test_retries_on_provider_timeout"], infra: false },
    Repo { name: "example-org/web-frontend", service: "web-frontend", team: "storefront", provider: "github_actions", suite: "checkout.spec", fail_rate: 0.09, flaky: &["renders cart drawer"], infra: false },
    Repo { name: "example-org/infra-terraform", service: "infra", team: "platform", provider: "gitlab", suite: "", fail_rate: 0.1, flaky: &[], infra: true },
];

const TESTS: &[&str] = &[
    "creates a record", "rejects invalid input", "returns 404 for unknown ids", "handles concurrent updates", "serializes to json",
    "applies discounts", "validates currency", "retries transient errors", "emits audit events", "respects rate limits",
    "migrates schema", "closes connections on shutdown", "parses config", "computes totals", "authorizes requests",
];
const AUTHORS: &[&str] = &["Jordan Patel", "Riley Meyer", "Casey Garcia", "Morgan Silva", "Avery Kim", "Quinn Okafor", "Taylor Novak", "Sage Ito"];
const MESSAGES: &[&str] = &[
    "Fix rounding in totals", "Add retry to provider client", "Bump dependencies", "Refactor handler wiring", "Improve error messages",
    "Add index for lookups", "Speed up test suite", "Tune connection pool", "Remove dead feature flag", "Log request ids",
];
const BRANCHES: &[&str] = &["feature/faster-search", "fix/currency-rounding", "chore/deps", "feature/gift-cards", "fix/timeout-retries", "feature/saved-carts"];

struct Span {
    span_id: String,
    parent: Option<String>,
    name: String,
    start: u64,
    end: u64,
    attrs: Vec<(String, Value)>,
    failed: bool,
}

pub struct Run {
    pub payload: Value,
    pub failed_flaky: bool,
    pub failed: bool,
    pub end: u64,
}

const SEC: u64 = 1_000_000_000;

pub struct Commit {
    pub sha: String,
    pub author: &'static str,
    pub message: &'static str,
    /// When the change was committed (ns).
    pub at: u64,
}

pub fn commit(rng: &mut Rng, before: u64) -> Commit {
    Commit { sha: rng.hex(20), author: AUTHORS[rng.range(0, AUTHORS.len() as u64) as usize], message: MESSAGES[rng.range(0, MESSAGES.len() as u64) as usize], at: before - rng.range(15 * 60, 30 * 3600) * SEC }
}

/// Builds one pipeline run starting at `start` (ns). `broken`: the commit genuinely breaks the build.
#[allow(clippy::too_many_arguments)]
pub fn run(rng: &mut Rng, repo_idx: usize, pipeline: &str, branch: &str, c: &Commit, attempt: u32, broken: bool, start: u64) -> Run {
    let repo = &REPOS[repo_idx];
    let trace_id = rng.hex(16);
    let root_id = rng.hex(8);
    let mut spans: Vec<Span> = Vec::new();
    let mut t = start + rng.range(2, 15) * SEC;
    let mut failed = false;
    let mut failed_flaky = false;

    let deploy = pipeline == "deploy" || pipeline == "terraform apply";
    let jobs: Vec<(&str, &str, &str, f64)> = if repo.infra {
        if deploy {
            vec![("init", "setup", "build", 20.0), ("plan", "plan", "build", 50.0), ("apply", "apply", "deploy", 110.0)]
        } else {
            vec![("init", "setup", "build", 20.0), ("validate", "verify", "test", 15.0), ("plan", "plan", "build", 55.0)]
        }
    } else if deploy {
        vec![("build-image", "build", "build", 140.0), ("deploy-staging", "staging", "deploy", 70.0), ("smoke-tests", "staging", "test", 45.0), ("deploy-production", "production", "deploy", 90.0)]
    } else {
        let mut j = vec![("lint", "verify", "test", 25.0), ("unit-tests", "test", "test", 120.0), ("build", "build", "build", 80.0)];
        if branch == "main" {
            j.push(("integration-tests", "test", "test", 200.0));
        }
        j
    };

    for (job, stage, kind, median) in jobs {
        if failed {
            break;
        }
        let job_id = rng.hex(8);
        let job_start = t;
        let mut steps_t = job_start + SEC;
        let mut job_failed = false;
        let mut children = Vec::new();
        for step in ["Set up job", "Checkout", "Run", "Post job"] {
            let d = if step == "Run" { (median * (0.7 + rng.f() * 0.8)) as u64 } else { rng.range(1, 6) } * SEC;
            let sid = rng.hex(8);
            if step == "Run" && kind == "test" && job == "unit-tests" && !repo.suite.is_empty() {
                // Test cases inside the Run step.
                let mut tt = steps_t + SEC;
                for (i, name) in TESTS.iter().chain(repo.flaky.iter()).enumerate() {
                    let is_flaky = i >= TESTS.len();
                    let fail = (is_flaky && rng.chance(0.18)) || (broken && i == 3);
                    if fail {
                        job_failed = true;
                        if is_flaky && !broken {
                            failed_flaky = true;
                        }
                    }
                    let td = rng.range(80, 2500) * 1_000_000;
                    children.push(Span {
                        span_id: rng.hex(8),
                        parent: Some(sid.clone()),
                        name: format!("{}.{}", repo.suite, name),
                        start: tt,
                        end: tt + td,
                        attrs: vec![
                            ("test.suite.name".into(), json!(repo.suite)),
                            ("test.case.name".into(), json!(name)),
                            ("test.case.result.status".into(), json!(if fail { "fail" } else { "pass" })),
                            ("test.framework".into(), json!(if repo.service == "notification-worker" { "pytest" } else if repo.service == "web-frontend" { "jest" } else { "junit" })),
                        ],
                        failed: fail,
                    });
                    tt += td + rng.range(5, 60) * 1_000_000;
                }
            } else if step == "Run" && broken && (job == "plan" || (job == "lint" && repo.suite.is_empty())) {
                job_failed = true;
            }
            if step == "Run" && broken && job == "build" && rng.chance(0.3) {
                job_failed = true;
            }
            children.push(Span { span_id: sid, parent: Some(job_id.clone()), name: step.into(), start: steps_t, end: steps_t + d, attrs: vec![("ci.step.name".into(), json!(step))], failed: job_failed && step == "Run" });
            steps_t += d + SEC / 2;
        }
        // Deployments occasionally fail on their own (bad config, health checks).
        if kind == "deploy" && rng.chance(0.04) {
            job_failed = true;
        }
        let job_end = steps_t;
        let mut attrs = vec![
            ("cicd.pipeline.task.name".into(), json!(job)),
            ("cicd.pipeline.task.type".into(), json!(kind)),
            ("ci.stage.name".into(), json!(stage)),
            ("cicd.pipeline.task.run.result".into(), json!(if job_failed { "failure" } else { "success" })),
            ("cicd.pipeline.task.run.id".into(), json!(rng.range(10_000_000, 99_999_999).to_string())),
        ];
        if kind == "deploy" {
            let env = if job.contains("staging") { "staging" } else { "production" };
            attrs.push(("deployment.environment.name".into(), json!(env)));
            attrs.push(("deployment.service.name".into(), json!(repo.service)));
        }
        spans.push(Span { span_id: job_id, parent: Some(root_id.clone()), name: job.into(), start: job_start, end: job_end, attrs, failed: job_failed });
        spans.extend(children);
        failed = job_failed;
        t = job_end + rng.range(3, 25) * SEC;
    }
    // A small share of runs are canceled by a newer push.
    let canceled = !failed && branch != "main" && rng.chance(0.02);
    let end = t;
    let result = if canceled { "cancellation" } else if failed { "failure" } else { "success" };
    spans.push(Span {
        span_id: root_id,
        parent: None,
        name: pipeline.into(),
        start,
        end,
        attrs: vec![("cicd.pipeline.result".into(), json!(result))],
        failed,
    });

    let run_id = (start / SEC / 7) % 1_000_000;
    let resource = attrs_json(&[
        ("service.name".into(), json!(repo.service)),
        ("cicd.pipeline.name".into(), json!(pipeline)),
        ("cicd.pipeline.run.id".into(), json!(run_id.to_string())),
        ("cicd.pipeline.run.attempt".into(), json!(attempt)),
        ("cicd.pipeline.run.url.full".into(), json!(format!("https://ci.example.com/{}/runs/{run_id}", repo.name))),
        ("cicd.provider.name".into(), json!(repo.provider)),
        ("vcs.repository.name".into(), json!(repo.name)),
        ("vcs.repository.url.full".into(), json!(format!("https://git.example.com/{}", repo.name))),
        ("vcs.ref.head.name".into(), json!(branch)),
        ("vcs.ref.head.revision".into(), json!(c.sha)),
        ("vcs.revision.author".into(), json!(c.author)),
        ("vcs.revision.message".into(), json!(c.message)),
        ("vcs.revision.committed_at".into(), json!((c.at / 1_000_000).to_string())),
        ("team".into(), json!(repo.team)),
    ]);
    let spans_json: Vec<Value> = spans
        .iter()
        .map(|s| {
            let mut v = json!({
                "traceId": trace_id,
                "spanId": s.span_id,
                "name": s.name,
                "kind": 1,
                "startTimeUnixNano": s.start.to_string(),
                "endTimeUnixNano": s.end.to_string(),
                "attributes": attrs_json(&s.attrs),
                "status": { "code": if s.failed { 2 } else { 1 } },
            });
            if let Some(p) = &s.parent {
                v["parentSpanId"] = json!(p);
            }
            v
        })
        .collect();
    Run {
        payload: json!({ "resource": { "attributes": resource }, "scopeSpans": [{ "scope": { "name": "oc-tracegen-ci" }, "spans": spans_json }] }),
        failed,
        failed_flaky,
        end,
    }
}

/// One "push": a CI run (retried once when only flaky tests failed), followed on main by a deploy
/// when CI passed. Returns the resource-spans and when the last run ended.
pub fn push(rng: &mut Rng, start: u64) -> (Vec<Value>, u64) {
    let repo_idx = rng.range(0, REPOS.len() as u64) as usize;
    let repo = &REPOS[repo_idx];
    let branch = if rng.chance(0.45) { "main" } else { BRANCHES[rng.range(0, BRANCHES.len() as u64) as usize] };
    let c = commit(rng, start);
    let broken = rng.chance(repo.fail_rate);
    let mut out = Vec::new();
    let ci_name = if repo.infra { "terraform plan" } else { "ci" };
    let mut r = run(rng, repo_idx, ci_name, branch, &c, 1, broken, start);
    let mut end = r.end;
    let mut passed = !r.failed;
    out.push(r.payload);
    if r.failed_flaky && !broken {
        let retry_at = end + rng.range(30, 300) * SEC;
        r = run(rng, repo_idx, ci_name, branch, &c, 2, false, retry_at);
        end = r.end;
        passed = !r.failed;
        out.push(r.payload);
    }
    if passed && branch == "main" {
        let deploy_at = end + rng.range(10, 120) * SEC;
        let d = run(rng, repo_idx, if repo.infra { "terraform apply" } else { "deploy" }, branch, &c, 1, false, deploy_at);
        end = d.end;
        out.push(d.payload);
    }
    (out, end)
}
