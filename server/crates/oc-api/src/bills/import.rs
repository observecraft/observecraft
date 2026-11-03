//! The bills' import: a job per active config (`cost_bill_configs`, on the cloud crawlers' jobs), every
//! few hours. Each billing month of the last `months` whose bill AWS (or Azure) rewrote since the last
//! run is read again and replaces that month's rows of `otel.cloud_costs` (`FileId`
//! `bill:<provider>:<account>:<YYYY-MM>`); the run read is kept in the config's `imported`.

use std::sync::Arc;

use chrono::{Datelike, Months, NaiveDate};
use serde_json::Value;

use super::aws::{filter_accounts, load, month_manifests, parse_manifest};
use super::BillLine;
use crate::aws::api::{AwsError, Sdk, Target};
use crate::cloud::jobs::{Backoff, Crawler, Issue, JobKey, Outcome};
use crate::routes::cloud_cost::{self, CostLine};
use crate::state::AppState;

pub const AWS_PROVIDER: &str = "aws_cur";
/// How often a config's bills are looked at.
pub const INTERVAL_SECS: i64 = 4 * 3600;

/// Where AWS bills are read from (S3; tests use a fake).
#[async_trait::async_trait]
pub trait BillStore: Send + Sync {
    async fn list(&self, target: &Target, bucket: &str, prefix: &str) -> Result<Vec<String>, AwsError>;
    async fn get(&self, target: &Target, bucket: &str, key: &str) -> Result<Vec<u8>, AwsError>;
}

#[async_trait::async_trait]
impl BillStore for Sdk {
    async fn list(&self, target: &Target, bucket: &str, prefix: &str) -> Result<Vec<String>, AwsError> {
        self.list_objects(target, bucket, prefix).await
    }
    async fn get(&self, target: &Target, bucket: &str, key: &str) -> Result<Vec<u8>, AwsError> {
        self.get_object(target, bucket, key).await
    }
}

/// A config with what its run needs.
#[derive(sqlx::FromRow)]
struct ConfigRow {
    id: i64,
    account_id: String,
    config: Value,
    months: i32,
    imported: Value,
}

async fn config(st: &AppState, key: &JobKey) -> Result<Option<ConfigRow>, sqlx::Error> {
    sqlx::query_as("SELECT id, account_id, config, months, imported FROM cost_bill_configs WHERE org_id = $1 AND uid = $2::uuid AND status = 'active'")
        .bind(&key.org_id)
        .bind(&key.account_id)
        .fetch_optional(&st.postgres)
        .await
}

/// The first billing month (`YYYY-MM`) of the last `months`, the current one included.
fn first_month(now_ms: i64, months: i32) -> String {
    let today = chrono::DateTime::from_timestamp_millis(now_ms).unwrap_or_default().date_naive();
    let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).and_then(|d| d.checked_sub_months(Months::new(months.max(1) as u32 - 1))).unwrap_or(today);
    format!("{:04}-{:02}", first.year(), first.month())
}

/// A month's lines as `otel.cloud_costs` rows of the file `file_id`.
pub fn cost_lines(org: &str, file_id: &str, provider: &str, lines: &[BillLine], version: u64) -> Vec<CostLine> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| CostLine {
            org: org.to_string(),
            file_id: file_id.to_string(),
            line_no: i as u32,
            date: l.date.clone(),
            provider: provider.to_string(),
            service: l.service.clone(),
            sub_account: l.account_name.clone(),
            region: l.region.clone(),
            zone: l.zone.clone(),
            category: l.category.clone(),
            description: l.description.clone(),
            resource_id: l.resource_id.clone(),
            pricing_unit: l.pricing_unit.clone(),
            quantity: l.quantity,
            billed: l.billed,
            effective: l.effective,
            list: l.list,
            pricing_category: l.pricing_category.clone(),
            commitment_id: l.commitment_id.clone(),
            commitment_type: l.commitment_type.clone(),
            commitment_status: l.commitment_status.clone(),
            currency: if l.currency.is_empty() { "USD".into() } else { l.currency.clone() },
            tags: l.tags.clone(),
            source: "bill".into(),
            version,
            deleted: 0,
        })
        .collect()
}

/// A billing month of a config's bill, and the run of it that was read.
pub struct MonthRun<'a> {
    /// `aws`, `azure`
    pub provider: &'a str,
    pub account: &'a str,
    /// `YYYY-MM`
    pub month: &'a str,
    pub run: &'a str,
}

impl MonthRun<'_> {
    /// The month's rows in `otel.cloud_costs`.
    pub fn file_id(&self) -> String {
        format!("bill:{}:{}:{}", self.provider, self.account, self.month)
    }
}

/// Replaces a month's rows with `lines` and records the run read.
pub async fn replace_month(st: &AppState, org: &str, config_id: i64, m: &MonthRun<'_>, lines: &[BillLine]) -> Result<(), String> {
    let (file_id, month, run) = (m.file_id(), m.month, m.run);
    cloud_cost::delete_file(st, org, &file_id).await.map_err(|e| format!("removing {month}'s costs: {e}"))?;
    // Newer than the deletion just written.
    let version = chrono::Utc::now().timestamp_millis() as u64 + 1;
    cloud_cost::insert(st, &cost_lines(org, &file_id, m.provider, lines, version)).await.map_err(|e| format!("writing {month}'s costs: {e}"))?;
    sqlx::query("UPDATE cost_bill_configs SET imported = jsonb_set(imported, ARRAY[$3], to_jsonb($4::text)) WHERE org_id = $1 AND id = $2")
        .bind(org)
        .bind(config_id)
        .bind(month)
        .bind(run)
        .execute(&st.postgres)
        .await
        .map_err(|e| format!("recording {month}'s import: {e}"))?;
    Ok(())
}

fn other(message: String) -> Outcome {
    Outcome::Failed(Issue { kind: "other", message, missing_action: None, backoff: Backoff::Exponential })
}

/// AWS Cost and Usage Reports, read with the credentials of the account's AWS integration.
pub struct AwsBills {
    pub store: Arc<dyn BillStore>,
}

#[async_trait::async_trait]
impl Crawler for AwsBills {
    fn provider(&self) -> &'static str {
        AWS_PROVIDER
    }

    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        sqlx::query_as("SELECT c.org_id, c.uid::text, '' FROM cost_bill_configs c JOIN orgs o ON o.id = c.org_id AND o.status = 'active' WHERE c.provider = 'aws' AND c.status = 'active'")
            .fetch_all(&st.postgres)
            .await
    }

    async fn run(&self, st: &AppState, key: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(c) = config(st, key).await? else { return Ok(Outcome::Idle) };
        let integration: Option<String> = sqlx::query_scalar("SELECT id::text FROM aws_accounts WHERE org_id = $1 AND aws_account_id = $2").bind(&key.org_id).bind(&c.account_id).fetch_optional(&st.postgres).await?;
        let text = |k: &str| c.config[k].as_str().unwrap_or_default().to_string();
        let (bucket, region, report, prefix) = (text("bucket_name"), text("bucket_region"), text("report_name"), text("report_prefix"));
        let job = match integration {
            Some(id) => crate::aws::crawler::load_job(st, &JobKey { provider: "aws".into(), org_id: key.org_id.clone(), account_id: id, scope: region, collected_until: None }).await?,
            None => None,
        };
        let Some(job) = job else {
            let message = format!("ObserveCraft reads the Cost and Usage Report with the AWS integration of account {}: add it under Integrations → AWS", c.account_id);
            return Ok(Outcome::Failed(Issue { kind: "auth", message, missing_action: None, backoff: Backoff::Interval }));
        };
        let base = format!("{}{report}/", if prefix.is_empty() { String::new() } else { format!("{prefix}/") });
        let keys = match self.store.list(&job.target, &bucket, &base).await {
            Ok(k) => k,
            Err(e) => return Ok(Outcome::Failed(crate::aws::crawler::issue(&e))),
        };
        let first = first_month(now_ms, c.months);
        let (mut months, mut rows) = (0, 0);
        for (month, manifest_key) in month_manifests(keys, &prefix, &report).into_iter().filter(|(m, _)| *m >= first) {
            let manifest = match self.store.get(&job.target, &bucket, &manifest_key).await {
                Ok(bytes) => match parse_manifest(&bytes) {
                    Ok(m) => m,
                    Err(e) => return Ok(other(format!("s3://{bucket}/{manifest_key}: {e}"))),
                },
                Err(e) => return Ok(Outcome::Failed(crate::aws::crawler::issue(&e))),
            };
            if c.imported[&month].as_str() == Some(manifest.run.as_str()) {
                continue;
            }
            let dir = std::env::temp_dir().join(format!("oc-bill-{}-{month}-{}", key.account_id, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Ok(other(format!("a temporary directory for the bill: {e}")));
            }
            let mut files = vec![];
            for (i, file) in manifest.files.iter().enumerate() {
                let bytes = match self.store.get(&job.target, &bucket, file).await {
                    Ok(b) => b,
                    Err(e) => return Ok(Outcome::Failed(crate::aws::crawler::issue(&e))),
                };
                // The file's own name keeps its extension (.parquet, .csv.gz), which tells DuckDB how to read it.
                let path = dir.join(format!("{i}-{}", file.rsplit('/').next().unwrap_or("part")));
                if let Err(e) = std::fs::write(&path, bytes) {
                    return Ok(other(format!("writing the bill to disk: {e}")));
                }
                files.push(path);
            }
            let loaded = if files.is_empty() { Ok(vec![]) } else { tokio::task::spawn_blocking(move || load(&files)).await.unwrap_or_else(|e| Err(e.to_string())) };
            let _ = std::fs::remove_dir_all(&dir);
            let lines = match loaded {
                Ok(l) => filter_accounts(l, &c.config["account_filters"]),
                Err(e) => return Ok(other(format!("reading {month}'s report: {e}"))),
            };
            let m = MonthRun { provider: "aws", account: &c.account_id, month: &month, run: &manifest.run };
            if let Err(e) = replace_month(st, &key.org_id, c.id, &m, &lines).await {
                return Ok(other(e));
            }
            months += 1;
            rows += lines.len();
        }
        Ok(Outcome::Collected { end_ms: now_ms, metrics: months, points: rows, warning: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    /// A bucket: key → bytes.
    #[derive(Default)]
    struct FakeBucket {
        objects: StdMutex<HashMap<String, Vec<u8>>>,
    }

    #[async_trait::async_trait]
    impl BillStore for FakeBucket {
        async fn list(&self, _t: &Target, bucket: &str, prefix: &str) -> Result<Vec<String>, AwsError> {
            assert_eq!(bucket, "example-org-cur");
            Ok(self.objects.lock().unwrap().keys().filter(|k| k.starts_with(prefix)).cloned().collect())
        }
        async fn get(&self, _t: &Target, _bucket: &str, key: &str) -> Result<Vec<u8>, AwsError> {
            self.objects.lock().unwrap().get(key).cloned().ok_or_else(|| AwsError::Other(format!("NoSuchKey {key}")))
        }
    }

    /// A CUR 2.0 month: its manifest (run `run`) and one Parquet file of usage lines `(day, service, cost)`.
    fn deliver(bucket: &FakeBucket, month: &str, run: &str, lines: &[(&str, &str, f64)]) {
        let dir = std::env::temp_dir().join(format!("oc-bills-deliver-{}-{month}-{run}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("costs-00001.snappy.parquet");
        let values: Vec<String> = lines.iter().map(|(day, service, cost)| format!("(TIMESTAMP '{day} 00:00:00', '123456789012', 'ecommerce-prod', 'Usage', '{service}', 'us-east-1', {cost}, 'USD')")).collect();
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "COPY (SELECT * FROM (VALUES {}) AS t(line_item_usage_start_date, line_item_usage_account_id, line_item_usage_account_name, line_item_line_item_type, line_item_product_code, product_region_code, line_item_unblended_cost, line_item_currency_code)) TO '{}' (FORMAT parquet)",
            values.join(", "),
            file.display()
        ))
        .unwrap();
        let data = format!("cur/costs/data/BILLING_PERIOD={month}/costs-00001.snappy.parquet");
        let mut objects = bucket.objects.lock().unwrap();
        objects.insert(data.clone(), std::fs::read(&file).unwrap());
        objects.insert(format!("cur/costs/metadata/BILLING_PERIOD={month}/costs-Manifest.json"), json!({ "executionId": run, "dataFiles": [format!("s3://example-org-cur/{data}")] }).to_string().into_bytes());
    }

    async fn sums(st: &AppState, org: &str) -> (f64, f64, u64) {
        st.ch(org)
            .query("SELECT toFloat64(sum(BilledCost)), toFloat64(sum(EffectiveCost)), count() FROM otel.cloud_costs FINAL WHERE OrgId = {oc_org:String} AND IsDeleted = 0 AND ProviderName = 'aws'")
            .fetch_one::<(f64, f64, u64)>()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_month_is_read_again_only_when_aws_rewrites_it() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "bills-aws").await;
        let (uid,): (String,) = sqlx::query_as(
            "INSERT INTO cost_bill_configs (org_id, provider, account_id, config) VALUES ($1, 'aws', '123456789012', $2) RETURNING uid::text",
        )
        .bind(&org)
        .bind(json!({ "bucket_name": "example-org-cur", "bucket_region": "us-east-1", "report_name": "costs", "report_prefix": "cur", "account_filters": null }))
        .fetch_one(&st.postgres)
        .await
        .unwrap();
        let bucket = Arc::new(FakeBucket::default());
        let importer = AwsBills { store: bucket.clone() };
        assert!(importer.desired(st).await.unwrap().iter().any(|d| d.0 == org && d.1 == uid && d.2.is_empty()));
        let key = JobKey { provider: AWS_PROVIDER.into(), org_id: org.clone(), account_id: uid.clone(), scope: "".into(), collected_until: None };
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp_millis();

        // Without the AWS integration of the account there are no credentials to read the bucket with.
        let Outcome::Failed(issue) = importer.run(st, &key, now).await.unwrap() else { panic!("no integration") };
        assert!(issue.message.contains("Integrations → AWS"), "{}", issue.message);
        sqlx::query("INSERT INTO aws_accounts (org_id, aws_account_id, role_name, external_id) VALUES ($1, '123456789012', 'ObserveCraftIntegrationRole', 'x')").bind(&org).execute(&st.postgres).await.unwrap();

        deliver(&bucket, "2026-10", "e1", &[("2026-10-01", "AmazonEC2", 10.0), ("2026-10-02", "AmazonS3", 2.5)]);
        deliver(&bucket, "2026-11", "e1", &[("2026-11-01", "AmazonEC2", 4.0)]);
        // Older than the config's 15 months: not read.
        deliver(&bucket, "2025-01", "e1", &[("2025-01-01", "AmazonEC2", 1000.0)]);
        let outcome = importer.run(st, &key, now).await.unwrap();
        assert!(matches!(outcome, Outcome::Collected { metrics: 2, points: 3, warning: None, .. }), "{outcome:?}");
        assert_eq!(sums(st, &org).await, (16.5, 16.5, 3));

        // Nothing rewritten: nothing read.
        assert!(matches!(importer.run(st, &key, now).await.unwrap(), Outcome::Collected { metrics: 0, points: 0, .. }));
        // AWS rewrote October: its rows are replaced, November's stay.
        deliver(&bucket, "2026-10", "e2", &[("2026-10-01", "AmazonEC2", 11.0)]);
        assert!(matches!(importer.run(st, &key, now).await.unwrap(), Outcome::Collected { metrics: 1, points: 1, .. }));
        assert_eq!(sums(st, &org).await, (15.0, 15.0, 2));
        let imported: Value = sqlx::query_scalar("SELECT imported FROM cost_bill_configs WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(imported, json!({ "2026-10": "e2", "2026-11": "e1" }));
    }
}
