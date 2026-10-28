//! Data Jobs Monitoring: Airflow DAGs and Spark applications, from the metrics their integrations
//! already send through the Datadog Agent (names from `integrations-core/airflow/metadata.csv` and
//! `integrations-core/spark/metadata.csv`, BSD-3-Clause, Copyright (c) Datadog, Inc.):
//! - Airflow (StatsD, tagged `dag_id`): `airflow.dagrun.duration.success` / `.failed` (one point
//!   per finished run, its duration in ms) and `airflow.ti_successes` / `airflow.ti_failures`;
//! - Spark (tagged `app_name`, and `status` on job counts): `spark.job.count`, `spark.job.num_failed_tasks`,
//!   reported every check run as the application's current totals.
//!
//! A job is one DAG or Spark application; its runs are counted over the whole range.

use std::collections::{BTreeMap, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::{matches, parse_query, Fields, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use super::metric_inventory::{series_values, SeriesValue};
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum JobFramework {
    Airflow,
    Spark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum JobField {
    /// A finished run's duration (ms): one point per run.
    SucceededRun,
    FailedRun,
    /// Task (instance) counts: Airflow sends deltas, totalled over the range.
    TaskSuccesses,
    TaskFailures,
    /// Spark's current totals per application: the largest value in the range.
    SucceededJobs,
    FailedJobs,
    RunningJobs,
    FailedTasks,
}

type JobMetric = (&'static str, JobFramework, JobField, Option<(&'static str, &'static str)>);

const JOB_METRICS: &[JobMetric] = {
    use JobField::*;
    use JobFramework::*;
    &[
        ("airflow.dagrun.duration.success", Airflow, SucceededRun, None),
        ("airflow.dagrun.duration.failed", Airflow, FailedRun, None),
        ("airflow.ti_successes", Airflow, TaskSuccesses, None),
        ("airflow.ti_failures", Airflow, TaskFailures, None),
        ("spark.job.count", Spark, SucceededJobs, Some(("status", "succeeded"))),
        ("spark.job.count", Spark, FailedJobs, Some(("status", "failed"))),
        ("spark.job.count", Spark, RunningJobs, Some(("status", "running"))),
        ("spark.job.num_failed_tasks", Spark, FailedTasks, None),
    ]
};

const DAG_KEYS: &[&str] = &["dag_id", "dag"];
const APP_KEYS: &[&str] = &["app_name", "spark_app_name", "application"];

fn metric_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = JOB_METRICS.iter().map(|m| m.0).collect();
    names.dedup();
    names
}

fn classify(v: &SeriesValue) -> Option<(JobFramework, JobField)> {
    JOB_METRICS.iter().find(|(name, _, _, filter)| *name == v.metric && filter.is_none_or(|(k, want)| v.attrs.get(k).is_some_and(|x| x == want))).map(|(_, f, field, _)| (*f, *field))
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Failures in the range.
    Failed,
    Running,
    Succeeded,
    /// Only task-level metrics in the range.
    Unknown,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DataJob {
    /// `framework:name`.
    pub id: String,
    pub framework: JobFramework,
    /// The DAG id or Spark application name.
    pub name: String,
    pub env: Option<String>,
    pub cluster: Option<String>,
    pub host: Option<String>,
    /// Successful runs (Airflow DAG runs, Spark jobs) in the range.
    pub succeeded: u64,
    pub failed: u64,
    /// Spark jobs running at the last check.
    pub running: u64,
    /// Average duration of the runs in the range (Airflow).
    pub avg_duration_ms: Option<f64>,
    pub last_duration_ms: Option<f64>,
    pub failed_tasks: u64,
    pub succeeded_tasks: u64,
    pub status: JobStatus,
    pub last_seen: i64,
}

#[derive(Default)]
struct Acc {
    framework: Option<JobFramework>,
    name: String,
    env: Option<String>,
    cluster: Option<String>,
    host: Option<String>,
    counts: HashMap<&'static str, f64>,
    durations: Vec<(f64, u64)>,
    last_duration: Option<(i64, f64)>,
    last: i64,
}

/// The jobs behind `values` (whole-range values over a window of `window_ms`), failed first.
pub(crate) fn aggregate_jobs(values: &[SeriesValue], window_ms: i64) -> Vec<DataJob> {
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    for v in values {
        let Some((framework, field)) = classify(v) else { continue };
        let (fw, keys) = match framework {
            JobFramework::Airflow => ("airflow", DAG_KEYS),
            JobFramework::Spark => ("spark", APP_KEYS),
        };
        let name = v.tag(keys).unwrap_or_else(|| "unnamed".into());
        let a = accs.entry(format!("{fw}:{name}")).or_default();
        a.framework = Some(framework);
        a.name = name;
        let fill = |slot: &mut Option<String>, keys: &[&str]| {
            if slot.is_none() {
                *slot = v.tag(keys);
            }
        };
        fill(&mut a.env, &["deployment.environment"]);
        fill(&mut a.cluster, &["k8s.cluster.name", "cluster_name", "databricks_cluster_name"]);
        fill(&mut a.host, &["host.name"]);
        a.last = a.last.max(v.last);
        let total = v.value * window_ms as f64 / 1000.0;
        let add = |counts: &mut HashMap<&'static str, f64>, k: &'static str, x: f64| *counts.entry(k).or_default() += x;
        let max = |counts: &mut HashMap<&'static str, f64>, k: &'static str, x: f64| {
            let e = counts.entry(k).or_default();
            *e = e.max(x);
        };
        match field {
            JobField::SucceededRun | JobField::FailedRun => {
                add(&mut a.counts, if field == JobField::SucceededRun { "succeeded" } else { "failed" }, v.count as f64);
                a.durations.push((v.value, v.count));
                if a.last_duration.is_none_or(|(t, _)| v.last > t) {
                    a.last_duration = Some((v.last, v.value));
                }
            }
            JobField::TaskSuccesses => add(&mut a.counts, "succeeded_tasks", total),
            JobField::TaskFailures => add(&mut a.counts, "failed_tasks", total),
            JobField::SucceededJobs => max(&mut a.counts, "succeeded", v.max),
            JobField::FailedJobs => max(&mut a.counts, "failed", v.max),
            JobField::RunningJobs => max(&mut a.counts, "running", v.value.round()),
            JobField::FailedTasks => max(&mut a.counts, "failed_tasks", v.max),
        }
    }
    let mut jobs: Vec<DataJob> = accs
        .into_iter()
        .filter_map(|(id, a)| {
            let framework = a.framework?;
            let n = |k: &str| a.counts.get(k).copied().unwrap_or(0.0).max(0.0).round() as u64;
            let runs: u64 = a.durations.iter().map(|(_, c)| c).sum();
            let avg = (runs > 0).then(|| (a.durations.iter().map(|(d, c)| d * *c as f64).sum::<f64>() / runs as f64).round());
            let (succeeded, failed, running) = (n("succeeded"), n("failed"), n("running"));
            let status = if failed > 0 || n("failed_tasks") > 0 && framework == JobFramework::Airflow {
                JobStatus::Failed
            } else if running > 0 {
                JobStatus::Running
            } else if succeeded > 0 {
                JobStatus::Succeeded
            } else {
                JobStatus::Unknown
            };
            Some(DataJob {
                id,
                framework,
                name: a.name,
                env: a.env,
                cluster: a.cluster,
                host: a.host,
                succeeded,
                failed,
                running,
                avg_duration_ms: avg,
                last_duration_ms: a.last_duration.map(|(_, d)| d.round()),
                failed_tasks: n("failed_tasks"),
                succeeded_tasks: n("succeeded_tasks"),
                status,
                last_seen: a.last,
            })
        })
        .collect();
    let rank = |s: JobStatus| match s {
        JobStatus::Failed => 0,
        JobStatus::Running => 1,
        JobStatus::Succeeded => 2,
        JobStatus::Unknown => 3,
    };
    jobs.sort_by(|a, b| rank(a.status).cmp(&rank(b.status)).then(a.name.cmp(&b.name)));
    jobs
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Facet values of a job: `framework`, `status`, `env`, `cluster`, `host`.
fn job_tags(j: &DataJob) -> HashMap<String, String> {
    let mut tags = HashMap::from([("framework".to_string(), enum_str(&j.framework)), ("status".to_string(), enum_str(&j.status))]);
    for (k, v) in [("env", &j.env), ("cluster", &j.cluster), ("host", &j.host)] {
        if let Some(v) = v {
            tags.insert(k.to_string(), v.clone());
        }
    }
    tags
}

fn job_matches(j: &DataJob, terms: &[Term]) -> bool {
    let tags = job_tags(j);
    let empty = HashMap::new();
    matches(terms, &Fields { service: "", status: &enum_str(&j.status), host: j.host.as_deref().unwrap_or(""), source: "", message: &j.name, tags: &tags, attr: &empty, index: None })
}

const FACET_KEYS: &[&str] = &["framework", "status", "env", "cluster", "host"];

#[derive(Serialize, ToSchema)]
pub struct JobFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
pub struct JobFacet {
    pub key: String,
    pub values: Vec<JobFacetValue>,
}

fn facets(jobs: &[DataJob]) -> Vec<JobFacet> {
    FACET_KEYS
        .iter()
        .map(|k| {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for j in jobs {
                if let Some(v) = job_tags(j).remove(*k) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            let mut values: Vec<JobFacetValue> = counts.into_iter().map(|(value, count)| JobFacetValue { value, count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            JobFacet { key: k.to_string(), values }
        })
        .filter(|f| !f.values.is_empty())
        .collect()
}

#[derive(Deserialize, ToSchema)]
pub struct JobSearchRequest {
    pub range: TimeRange,
    /// `framework:airflow status:failed env:prod cluster:x host:y` + free text on the job name.
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema)]
pub struct JobSearchPage {
    /// False when no Airflow or Spark metrics arrived in the range (show the setup guide).
    pub enabled: bool,
    pub jobs: Vec<DataJob>,
    pub facets: Vec<JobFacet>,
}

/// Airflow DAGs and Spark applications that reported in the range, with their runs.
#[utoipa::path(post, path = "/api/v1/data-jobs/search", operation_id = "search_data_jobs", request_body = JobSearchRequest, responses((status = 200, body = JobSearchPage)), tag = "databases")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<JobSearchRequest>) -> ApiResult<JobSearchPage> {
    req.range.validate()?;
    let values = series_values(&st, &org, &metric_names(), req.range.from, req.range.to, None).await?;
    let all = aggregate_jobs(&values, req.range.to - req.range.from);
    let terms = parse_query(&req.query);
    let jobs: Vec<DataJob> = all.iter().filter(|j| job_matches(j, &terms)).cloned().collect();
    Ok(Json(JobSearchPage { enabled: !all.is_empty(), facets: facets(&all), jobs }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(metric: &str, value: f64, count: u64, last: i64, attrs: &[(&str, &str)]) -> SeriesValue {
        let attrs = attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let res = HashMap::from([("host.name".to_string(), "i-0a1b2c3d4e5f60020".to_string()), ("deployment.environment".to_string(), "prod".to_string())]);
        SeriesValue { metric: metric.into(), res, attrs, t: 0, value, max: value, count, last }
    }

    #[test]
    fn airflow_runs_from_duration_points() {
        let dag = [("dag_id", "nightly_orders_export")];
        let jobs = aggregate_jobs(
            &[
                sv("airflow.dagrun.duration.success", 120_000.0, 3, 10, &dag),
                sv("airflow.dagrun.duration.failed", 40_000.0, 1, 20, &dag),
                // 0.01 task failures per second over an hour: 36.
                sv("airflow.ti_failures", 0.01, 4, 20, &[("dag_id", "nightly_orders_export"), ("task_id", "load")]),
            ],
            3_600_000,
        );
        assert_eq!(jobs.len(), 1);
        let j = &jobs[0];
        assert_eq!(j.id, "airflow:nightly_orders_export");
        assert_eq!((j.succeeded, j.failed, j.failed_tasks), (3, 1, 36));
        assert_eq!(j.avg_duration_ms, Some(100_000.0), "weighted by runs");
        assert_eq!(j.last_duration_ms, Some(40_000.0));
        assert_eq!(j.status, JobStatus::Failed);
        assert_eq!(j.env.as_deref(), Some("prod"));
        assert_eq!(j.last_seen, 20);
    }

    #[test]
    fn spark_totals_are_the_latest_counts() {
        let app = |status: &'static str| [("app_name", "catalog-reindex"), ("status", status)];
        let mut succeeded = sv("spark.job.count", 4.0, 10, 5, &app("succeeded"));
        succeeded.max = 12.0;
        let jobs = aggregate_jobs(&[succeeded, sv("spark.job.count", 1.0, 10, 5, &app("running")), sv("spark.job.count", 0.0, 10, 5, &app("failed"))], 3_600_000);
        let j = &jobs[0];
        assert_eq!(j.framework, JobFramework::Spark);
        assert_eq!((j.succeeded, j.failed, j.running), (12, 0, 1));
        assert_eq!(j.status, JobStatus::Running);
    }

    #[test]
    fn failed_first_then_facets_and_query() {
        let jobs = aggregate_jobs(
            &[
                sv("airflow.dagrun.duration.success", 1000.0, 1, 1, &[("dag_id", "a_ok")]),
                sv("airflow.dagrun.duration.failed", 1000.0, 1, 1, &[("dag_id", "z_broken")]),
                sv("spark.job.count", 2.0, 1, 1, &[("app_name", "m"), ("status", "succeeded")]),
            ],
            60_000,
        );
        assert_eq!(jobs[0].name, "z_broken");
        let f = facets(&jobs);
        assert_eq!(f.iter().find(|f| f.key == "framework").unwrap().values.len(), 2);
        assert_eq!(jobs.iter().filter(|j| job_matches(j, &parse_query("framework:airflow status:succeeded"))).count(), 1);
        assert_eq!(jobs.iter().filter(|j| job_matches(j, &parse_query("broken"))).count(), 1);
    }

    #[test]
    fn unrelated_metrics_are_ignored() {
        assert!(aggregate_jobs(&[sv("spark.driver.memory_used", 1.0, 1, 1, &[])], 60_000).is_empty());
        assert_eq!(metric_names().len(), 6);
    }
}
