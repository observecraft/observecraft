//! A finished GitLab pipeline as one CI/CD trace (OTLP JSON `resourceSpans`), with the attributes
//! `routes::ci` and the Software Delivery pages read: the run, repository and commit on the resource
//! (`cicd.*`, `vcs.*`), the pipeline as the root span (`cicd.pipeline.result`) and one child span per job
//! that ran (`cicd.pipeline.task.*`, `ci.stage.name`; deploy jobs `deployment.environment.name`).
//!
//! Trace and span ids are derived from the project, pipeline and job ids, so the same pipeline always
//! gets the same ids.

use std::collections::HashMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::routes::portal_sources::parse_time;

pub const PROVIDER_NAME: &str = "gitlab";

/// Statuses of a pipeline that won't change any more (GitLab: success, failed, canceled, skipped).
pub fn is_finished(status: &str) -> bool {
    matches!(status, "success" | "failed" | "canceled" | "canceling" | "skipped")
}

/// `cicd.pipeline.result` / `cicd.pipeline.task.run.result` of a GitLab status.
pub fn result(status: &str) -> &'static str {
    match status {
        "success" => "success",
        "failed" => "failure",
        "canceled" | "canceling" => "cancellation",
        _ => "skip",
    }
}

fn hex_id(seed: &str, bytes: usize) -> String {
    Sha256::digest(seed.as_bytes())[..bytes].iter().map(|b| format!("{b:02x}")).collect()
}

/// The trace id of a pipeline (32 hex digits).
pub fn trace_id(project_id: u64, pipeline_id: u64) -> String {
    hex_id(&format!("gitlab-ci/{project_id}/{pipeline_id}"), 16)
}

/// The span id of a pipeline (`job` None) or of one of its jobs (16 hex digits).
pub fn span_id(project_id: u64, pipeline_id: u64, job: Option<u64>) -> String {
    match job {
        None => hex_id(&format!("gitlab-ci/{project_id}/{pipeline_id}/pipeline"), 8),
        Some(j) => hex_id(&format!("gitlab-ci/{project_id}/{pipeline_id}/job/{j}"), 8),
    }
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Epoch ms of a GitLab time, None when absent or unreadable.
fn ms(v: &Value, key: &str) -> Option<i64> {
    Some(parse_time(v.get(key)?.as_str()?)).filter(|t| *t > 0)
}

fn attr(key: &str, value: Value) -> Value {
    let value = match value {
        Value::Bool(b) => json!({ "boolValue": b }),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({ "intValue": n.to_string() }),
        Value::String(s) => json!({ "stringValue": s }),
        other => json!({ "stringValue": other.to_string() }),
    };
    json!({ "key": key, "value": value })
}

fn nanos(ms: i64) -> String {
    (ms.max(0) as u128 * 1_000_000).to_string()
}

/// `build`, `test` or `deploy`: deploy when the job deployed to an environment, test for test and lint jobs.
fn task_type(job: &Value, environment: Option<&String>) -> &'static str {
    if environment.is_some() {
        return "deploy";
    }
    let words = format!("{} {}", s(job, "name"), s(job, "stage")).to_ascii_lowercase();
    if ["test", "lint", "check", "verify", "scan"].iter().any(|w| words.contains(w)) {
        "test"
    } else {
        "build"
    }
}

/// The pipeline as a CI/CD trace: `project` (`GET /projects/:id`), `pipeline` (`GET .../pipelines/:id`),
/// its `jobs`, the `commit` (fields of a job's `commit` or `GET .../repository/commits/:sha`) and the
/// environments jobs deployed to (job id → environment name, from the project's deployments).
pub fn pipeline_trace(project: &Value, pipeline: &Value, jobs: &[Value], commit: &Value, environments: &HashMap<u64, String>) -> Value {
    let project_id = project["id"].as_u64().unwrap_or_default();
    let pipeline_id = pipeline["id"].as_u64().unwrap_or_default();
    let repository = s(project, "path_with_namespace");
    let name = Some(s(pipeline, "name")).filter(|n| !n.is_empty()).unwrap_or(repository);
    let status = s(pipeline, "status");
    let started_jobs: Vec<&Value> = jobs.iter().filter(|j| ms(j, "started_at").is_some()).collect();
    let start = ms(pipeline, "started_at").or_else(|| started_jobs.iter().filter_map(|j| ms(j, "started_at")).min()).or_else(|| ms(pipeline, "created_at")).unwrap_or_default();
    let end = ms(pipeline, "finished_at").or_else(|| ms(pipeline, "updated_at")).unwrap_or(start).max(start);
    let committed_at = ms(commit, "committed_date").or_else(|| ms(commit, "created_at"));
    let mut resource = vec![
        attr("service.name", json!(s(project, "path"))),
        attr("cicd.pipeline.name", json!(name)),
        attr("cicd.pipeline.run.id", json!(pipeline_id.to_string())),
        attr("cicd.pipeline.run.url.full", json!(s(pipeline, "web_url"))),
        attr("cicd.pipeline.run.attempt", json!("1")),
        attr("cicd.provider.name", json!(PROVIDER_NAME)),
        attr("vcs.repository.name", json!(repository)),
        attr("vcs.repository.url.full", json!(s(project, "web_url"))),
        attr("vcs.ref.head.name", json!(s(pipeline, "ref"))),
        attr("vcs.ref.head.revision", json!(s(pipeline, "sha"))),
        attr("vcs.revision.author", json!(s(commit, "author_name"))),
        attr("vcs.revision.message", json!(Some(s(commit, "title")).filter(|t| !t.is_empty()).unwrap_or(s(commit, "message")))),
    ];
    if let Some(at) = committed_at {
        resource.push(attr("vcs.revision.committed_at", json!(at.to_string())));
    }
    let root_id = span_id(project_id, pipeline_id, None);
    let trace = trace_id(project_id, pipeline_id);
    let mut spans = vec![json!({
        "traceId": trace, "spanId": root_id, "name": name, "kind": 1,
        "startTimeUnixNano": nanos(start), "endTimeUnixNano": nanos(end),
        "attributes": [attr("cicd.pipeline.result", json!(result(status))), attr("ci.pipeline.source", json!(s(pipeline, "source")))],
        "status": { "code": if status == "failed" { 2 } else { 1 } },
    })];
    for job in started_jobs {
        let job_id = job["id"].as_u64().unwrap_or_default();
        let job_status = s(job, "status");
        let job_start = ms(job, "started_at").unwrap_or(start);
        let job_end = ms(job, "finished_at").unwrap_or(end).max(job_start);
        let environment = environments.get(&job_id);
        let mut attributes = vec![
            attr("cicd.pipeline.task.name", json!(s(job, "name"))),
            attr("cicd.pipeline.task.type", json!(task_type(job, environment))),
            attr("cicd.pipeline.task.run.id", json!(job_id.to_string())),
            attr("cicd.pipeline.task.run.url.full", json!(s(job, "web_url"))),
            attr("cicd.pipeline.task.run.result", json!(result(job_status))),
            attr("ci.stage.name", json!(s(job, "stage"))),
            attr("ci.job.allow_failure", json!(job["allow_failure"].as_bool().unwrap_or(false))),
        ];
        if let Some(reason) = job["failure_reason"].as_str().filter(|r| !r.is_empty()) {
            attributes.push(attr("error.type", json!(reason)));
        }
        if let Some(runner) = job.get("runner").filter(|r| r.is_object()) {
            attributes.push(attr("cicd.worker.id", json!(runner["id"].as_u64().unwrap_or_default().to_string())));
            attributes.push(attr("cicd.worker.name", json!(s(runner, "description"))));
        }
        if let Some(env) = environment {
            attributes.push(attr("deployment.environment.name", json!(env)));
            attributes.push(attr("deployment.service.name", json!(s(project, "path"))));
        }
        spans.push(json!({
            "traceId": trace, "spanId": span_id(project_id, pipeline_id, Some(job_id)), "parentSpanId": root_id, "name": s(job, "name"), "kind": 1,
            "startTimeUnixNano": nanos(job_start), "endTimeUnixNano": nanos(job_end), "attributes": attributes,
            "status": { "code": if job_status == "failed" { 2 } else { 1 } },
        }));
    }
    json!({ "resource": { "attributes": resource }, "scopeSpans": [{ "scope": { "name": "observecraft-gitlab-ci" }, "spans": spans }] })
}

#[cfg(test)]
mod tests {
    use super::super::fixtures;
    use super::*;

    fn attrs(v: &Value) -> HashMap<String, Value> {
        v.as_array().unwrap().iter().map(|a| (a["key"].as_str().unwrap().to_string(), a["value"].as_object().unwrap().values().next().unwrap().clone())).collect()
    }

    fn trace() -> Value {
        let jobs = fixtures::jobs();
        let jobs = jobs.as_array().unwrap();
        let envs = HashMap::from([(9003, "production".to_string())]);
        pipeline_trace(&fixtures::project(), &fixtures::pipeline(55_001, "success", "2026-11-03T10:06:41.000Z"), jobs, &jobs[0]["commit"], &envs)
    }

    #[test]
    fn the_resource_carries_the_run_repository_and_commit() {
        let t = trace();
        let r = attrs(&t["resource"]["attributes"]);
        let want = [
            ("cicd.pipeline.name", "example-org/storefront-api"),
            ("cicd.pipeline.run.id", "55001"),
            ("cicd.pipeline.run.url.full", "https://gitlab.example.com/example-org/storefront-api/-/pipelines/55001"),
            ("cicd.pipeline.run.attempt", "1"),
            ("cicd.provider.name", "gitlab"),
            ("vcs.repository.name", "example-org/storefront-api"),
            ("vcs.repository.url.full", "https://gitlab.example.com/example-org/storefront-api"),
            ("vcs.ref.head.name", "main"),
            ("vcs.ref.head.revision", "9fceb02d0ae598e95dc970b74767f19372d61af8"),
            ("vcs.revision.author", "Jordan Patel"),
            ("vcs.revision.message", "Fix rounding in totals"),
            ("vcs.revision.committed_at", "1793698800000"),
        ];
        for (k, v) in want {
            assert_eq!(r.get(k).and_then(Value::as_str), Some(v), "{k}");
        }
    }

    #[test]
    fn the_pipeline_is_the_root_span_and_each_job_that_ran_a_child() {
        let t = trace();
        let spans = t["scopeSpans"][0]["spans"].as_array().unwrap();
        // The manual job never ran.
        assert_eq!(spans.len(), 4);
        let root = &spans[0];
        assert!(root.get("parentSpanId").is_none());
        assert_eq!(root["startTimeUnixNano"], "1793700005000000000");
        assert_eq!(root["endTimeUnixNano"], "1793700400000000000");
        assert_eq!(attrs(&root["attributes"])["cicd.pipeline.result"], "success");
        for job in &spans[1..] {
            assert_eq!(job["parentSpanId"], root["spanId"]);
            assert_eq!(job["traceId"], root["traceId"]);
        }
        let lint = attrs(&spans[2]["attributes"]);
        assert_eq!(lint["cicd.pipeline.task.name"], "lint");
        assert_eq!(lint["cicd.pipeline.task.type"], "test");
        assert_eq!(lint["cicd.pipeline.task.run.result"], "failure");
        assert_eq!(lint["ci.stage.name"], "test");
        assert_eq!(lint["ci.job.allow_failure"], true);
        assert_eq!(lint["error.type"], "script_failure");
        assert_eq!(lint["cicd.worker.name"], "shared-runner-linux-1");
        assert!(!lint.contains_key("deployment.environment.name"));
        let deploy = attrs(&spans[3]["attributes"]);
        assert_eq!(deploy["cicd.pipeline.task.type"], "deploy");
        assert_eq!(deploy["deployment.environment.name"], "production");
        assert_eq!(deploy["deployment.service.name"], "storefront-api");
        assert_eq!(attrs(&spans[1]["attributes"])["cicd.pipeline.task.type"], "build");
    }

    #[test]
    fn ids_are_deterministic_and_distinct() {
        assert_eq!(trace_id(4211, 55_001), trace_id(4211, 55_001));
        assert_eq!(trace_id(4211, 55_001).len(), 32);
        assert_ne!(trace_id(4211, 55_001), trace_id(4211, 55_002));
        assert_ne!(trace_id(4211, 55_001), trace_id(4212, 55_001));
        assert_eq!(span_id(4211, 55_001, None).len(), 16);
        assert_ne!(span_id(4211, 55_001, Some(1)), span_id(4211, 55_001, Some(2)));
        assert_ne!(span_id(4211, 55_001, None), span_id(4211, 55_001, Some(1)));
        let (a, b) = (trace(), trace());
        assert_eq!(a, b);
        assert_eq!(a["scopeSpans"][0]["spans"][0]["traceId"], trace_id(4211, 55_001));
    }

    #[test]
    fn statuses_map_to_results_the_pages_read() {
        assert_eq!(result("success"), "success");
        assert_eq!(result("failed"), "failure");
        assert_eq!(result("canceled"), "cancellation");
        assert_eq!(result("skipped"), "skip");
        for s in ["success", "failed", "canceled", "skipped"] {
            assert!(is_finished(s), "{s}");
        }
        for s in ["created", "waiting_for_resource", "preparing", "pending", "running", "manual", "scheduled"] {
            assert!(!is_finished(s), "{s}");
        }
        let failed = pipeline_trace(&fixtures::project(), &fixtures::pipeline(1, "failed", "2026-11-03T10:06:41.000Z"), &[], &Value::Null, &HashMap::new());
        let root = &failed["scopeSpans"][0]["spans"][0];
        assert_eq!(attrs(&root["attributes"])["cicd.pipeline.result"], "failure");
        assert_eq!(root["status"]["code"], 2);
        // Without a commit there is no committed_at (DORA skips the lead time).
        assert!(!attrs(&failed["resource"]["attributes"]).contains_key("vcs.revision.committed_at"));
    }

    #[test]
    fn a_named_pipeline_keeps_its_name() {
        let mut p = fixtures::pipeline(2, "success", "2026-11-03T10:06:41.000Z");
        p["name"] = json!("nightly");
        let t = pipeline_trace(&fixtures::project(), &p, &[], &Value::Null, &HashMap::new());
        assert_eq!(attrs(&t["resource"]["attributes"])["cicd.pipeline.name"], "nightly");
        assert_eq!(t["scopeSpans"][0]["spans"][0]["name"], "nightly");
    }
}
