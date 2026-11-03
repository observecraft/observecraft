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
use crate::azure::api::{AzureError, Credentials, Rest};
use crate::routes::datadog_api::azure_integration as azure_accounts;
use crate::routes::datadog_api::cost_bills::DATASETS;
use crate::cloud::jobs::{Backoff, Crawler, Issue, JobKey, Outcome};
use crate::routes::cloud_cost::{self, CostLine};
use crate::state::AppState;

pub const AWS_PROVIDER: &str = "aws_cur";
pub const AZURE_PROVIDER: &str = "azure_uc";
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

/// Where Azure cost exports are read from (Blob Storage; tests use a fake).
#[async_trait::async_trait]
pub trait BlobStore: Send + Sync {
    /// The blobs under `prefix`, with their last modification (epoch ms).
    async fn list(&self, c: &Credentials, account: &str, container: &str, prefix: &str) -> Result<Vec<(String, i64)>, AzureError>;
    async fn get(&self, c: &Credentials, account: &str, container: &str, name: &str) -> Result<Vec<u8>, AzureError>;
}

#[async_trait::async_trait]
impl BlobStore for Rest {
    async fn list(&self, c: &Credentials, account: &str, container: &str, prefix: &str) -> Result<Vec<(String, i64)>, AzureError> {
        self.list_blobs(c, account, container, prefix).await
    }
    async fn get(&self, c: &Credentials, account: &str, container: &str, name: &str) -> Result<Vec<u8>, AzureError> {
        self.get_blob(c, account, container, name).await
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

/// A bill's downloaded files, in a directory removed when it's dropped. Each file keeps its name: its
/// extension (`.parquet`, `.csv.gz`) tells DuckDB how to read it.
struct BillDir {
    dir: std::path::PathBuf,
    files: Vec<std::path::PathBuf>,
}

impl BillDir {
    fn new(name: String) -> Result<BillDir, String> {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("a temporary directory for the bill: {e}"))?;
        Ok(BillDir { dir, files: vec![] })
    }

    fn add(&mut self, key: &str, bytes: Vec<u8>) -> Result<(), String> {
        let path = self.dir.join(format!("{}-{}", self.files.len(), key.rsplit('/').next().unwrap_or("part")));
        std::fs::write(&path, bytes).map_err(|e| format!("writing the bill to disk: {e}"))?;
        self.files.push(path);
        Ok(())
    }

    /// The files read by `load`, off the async runtime.
    async fn load(&self, load: impl FnOnce(&[std::path::PathBuf]) -> Result<Vec<BillLine>, String> + Send + 'static) -> Result<Vec<BillLine>, String> {
        if self.files.is_empty() {
            return Ok(vec![]);
        }
        let files = self.files.clone();
        tokio::task::spawn_blocking(move || load(&files)).await.unwrap_or_else(|e| Err(e.to_string()))
    }
}

impl Drop for BillDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
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
            let mut dir = match BillDir::new(format!("oc-bill-{}-{month}-{}", key.account_id, std::process::id())) {
                Ok(d) => d,
                Err(e) => return Ok(other(e)),
            };
            for file in &manifest.files {
                let bytes = match self.store.get(&job.target, &bucket, file).await {
                    Ok(b) => b,
                    Err(e) => return Ok(Outcome::Failed(crate::aws::crawler::issue(&e))),
                };
                if let Err(e) = dir.add(file, bytes) {
                    return Ok(other(e));
                }
            }
            let lines = match dir.load(load).await {
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

/// Azure's actual and amortized cost exports, read with the app registration of the config's client id.
pub struct AzureBills {
    pub store: Arc<dyn BlobStore>,
}

#[async_trait::async_trait]
impl Crawler for AzureBills {
    fn provider(&self) -> &'static str {
        AZURE_PROVIDER
    }

    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        sqlx::query_as("SELECT c.org_id, c.uid::text, '' FROM cost_bill_configs c JOIN orgs o ON o.id = c.org_id AND o.status = 'active' WHERE c.provider = 'azure' AND c.status = 'active'")
            .fetch_all(&st.postgres)
            .await
    }

    async fn run(&self, st: &AppState, key: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(c) = config(st, key).await? else { return Ok(Outcome::Idle) };
        let client_id = c.config["client_id"].as_str().unwrap_or_default().to_string();
        let registration: Option<String> =
            sqlx::query_scalar("SELECT id::text FROM azure_accounts WHERE org_id = $1 AND tenant_name = $2 AND client_id = $3").bind(&key.org_id).bind(&c.account_id).bind(&client_id).fetch_optional(&st.postgres).await?;
        let secret = match registration {
            Some(id) => azure_accounts::client_secret(st, &key.org_id, &id).await?,
            None => None,
        };
        let Some(client_secret) = secret else {
            let message = format!("ObserveCraft reads the cost exports with the Azure integration of app registration {client_id} (tenant {}): add it, with its client secret, under Integrations → Azure", c.account_id);
            return Ok(Outcome::Failed(Issue { kind: "auth", message, missing_action: None, backoff: Backoff::Interval }));
        };
        let creds = Credentials { tenant_id: c.account_id.clone(), client_id, client_secret };
        let first = first_month(now_ms, c.months);
        let (mut months, mut rows) = (0, 0);
        for dataset in DATASETS {
            let bill = &c.config[dataset];
            let text = |k: &str| bill[k].as_str().unwrap_or_default().to_string();
            let (account, container, path, export) = (text("storage_account"), text("storage_container"), text("export_path"), text("export_name"));
            let path = path.trim_matches('/');
            let prefix = if path.is_empty() { format!("{export}/") } else { format!("{path}/{export}/") };
            let blobs = match self.store.list(&creds, &account, &container, &prefix).await {
                Ok(b) => b,
                Err(e) => return Ok(Outcome::Failed(crate::azure::crawler::issue(&e))),
            };
            for (month, (manifest, run)) in super::azure::month_runs(blobs, path, &export).into_iter().filter(|(m, _)| *m >= first) {
                let imported_key = format!("{dataset}:{month}");
                if c.imported[&imported_key].as_str() == Some(run.as_str()) {
                    continue;
                }
                let files = if !manifest.ends_with("/manifest.json") {
                    vec![manifest.clone()]
                } else {
                    match self.store.get(&creds, &account, &container, &manifest).await {
                        Ok(bytes) => match super::azure::manifest_blobs(&bytes) {
                            Ok(f) => f,
                            Err(e) => return Ok(other(format!("{account}/{container}/{manifest}: {e}"))),
                        },
                        Err(e) => return Ok(Outcome::Failed(crate::azure::crawler::issue(&e))),
                    }
                };
                let mut dir = match BillDir::new(format!("oc-bill-{}-{dataset}-{month}-{}", key.account_id, std::process::id())) {
                    Ok(d) => d,
                    Err(e) => return Ok(other(e)),
                };
                for file in &files {
                    let bytes = match self.store.get(&creds, &account, &container, file).await {
                        Ok(b) => b,
                        Err(e) => return Ok(Outcome::Failed(crate::azure::crawler::issue(&e))),
                    };
                    if let Err(e) = dir.add(file, bytes) {
                        return Ok(other(e));
                    }
                }
                let lines = match dir.load(move |paths| super::azure::load(paths, dataset)).await {
                    Ok(l) => l,
                    Err(e) => return Ok(other(format!("reading {month}'s {dataset} cost export: {e}"))),
                };
                let config_id = c.id.to_string();
                let m = MonthRun { provider: "azure", account: &config_id, month: &imported_key, run: &run };
                if let Err(e) = replace_month(st, &key.org_id, c.id, &m, &lines).await {
                    return Ok(other(e));
                }
                months += 1;
                rows += lines.len();
            }
        }
        Ok(Outcome::Collected { end_ms: now_ms, metrics: months, points: rows, warning: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::api::{AzureError, Credentials};
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

    /// A provider's billed and amortized costs, and their rows.
    async fn sums(st: &AppState, org: &str, provider: &str) -> (f64, f64, u64) {
        st.ch(org)
            .query("SELECT toFloat64(sum(BilledCost)), toFloat64(sum(EffectiveCost)), count() FROM otel.cloud_costs FINAL WHERE OrgId = {oc_org:String} AND IsDeleted = 0 AND ProviderName = ?")
            .bind(provider)
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
        assert_eq!(sums(st, &org, "aws").await, (16.5, 16.5, 3));

        // Nothing rewritten: nothing read.
        assert!(matches!(importer.run(st, &key, now).await.unwrap(), Outcome::Collected { metrics: 0, points: 0, .. }));
        // AWS rewrote October: its rows are replaced, November's stay.
        deliver(&bucket, "2026-10", "e2", &[("2026-10-01", "AmazonEC2", 11.0)]);
        assert!(matches!(importer.run(st, &key, now).await.unwrap(), Outcome::Collected { metrics: 1, points: 1, .. }));
        assert_eq!(sums(st, &org, "aws").await, (15.0, 15.0, 2));
        let imported: Value = sqlx::query_scalar("SELECT imported FROM cost_bill_configs WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(imported, json!({ "2026-10": "e2", "2026-11": "e1" }));
    }

    /// (storage account, container, blob)
    type BlobKey = (String, String, String);

    /// Storage accounts: blob → (bytes, last modified).
    #[derive(Default)]
    struct FakeContainers {
        blobs: StdMutex<HashMap<BlobKey, (Vec<u8>, i64)>>,
    }

    #[async_trait::async_trait]
    impl BlobStore for FakeContainers {
        async fn list(&self, c: &Credentials, account: &str, container: &str, prefix: &str) -> Result<Vec<(String, i64)>, AzureError> {
            assert_eq!((c.tenant_id.as_str(), c.client_id.as_str(), c.client_secret.as_str()), ("tenant-1", "client-1", "s3cret"));
            Ok(self.blobs.lock().unwrap().iter().filter(|((a, k, n), _)| a == account && k == container && n.starts_with(prefix)).map(|((_, _, n), (_, m))| (n.clone(), *m)).collect())
        }
        async fn get(&self, _c: &Credentials, account: &str, container: &str, name: &str) -> Result<Vec<u8>, AzureError> {
            self.blobs.lock().unwrap().get(&(account.into(), container.into(), name.into())).map(|(b, _)| b.clone()).ok_or_else(|| AzureError::Other(format!("BlobNotFound {name}")))
        }
    }

    /// An export's run of a month: its manifest and one CSV of `(day, meter category, cost)` rows.
    fn export(blobs: &FakeContainers, name: &str, range: &str, run: &str, modified: i64, rows: &[(&str, &str, f64)]) {
        let dir = format!("cost-exports/{name}/{range}/{run}");
        let csv = std::iter::once("date,subscriptionId,subscriptionName,resourceLocation,meterCategory,costInBillingCurrency,billingCurrency,chargeType,pricingModel".to_string())
            .chain(rows.iter().map(|(day, category, cost)| format!("{day},sub-1,storefront-prod,EastUS,{category},{cost},USD,Usage,OnDemand")))
            .collect::<Vec<_>>()
            .join("\n");
        let part = format!("{dir}/part_0_0001.csv");
        let mut b = blobs.blobs.lock().unwrap();
        b.insert(("examplecostexports".into(), "costs".into(), part.clone()), (csv.into_bytes(), modified));
        b.insert(("examplecostexports".into(), "costs".into(), format!("{dir}/manifest.json")), (json!({ "blobs": [{ "blobName": part }], "runInfo": { "runId": run } }).to_string().into_bytes(), modified));
    }

    #[tokio::test]
    async fn azure_exports_fill_the_billed_and_amortized_costs() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "bills-azure").await;
        let bill = |export: &str| json!({ "export_name": export, "export_path": "/cost-exports", "storage_account": "examplecostexports", "storage_container": "costs" });
        let (uid,): (String,) = sqlx::query_as("INSERT INTO cost_bill_configs (org_id, provider, account_id, scope, config) VALUES ($1, 'azure', 'tenant-1', '/subscriptions/sub-1', $2) RETURNING uid::text")
            .bind(&org)
            .bind(json!({ "client_id": "client-1", "actual": bill("observecraft-actual"), "amortized": bill("observecraft-amortized") }))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let blobs = Arc::new(FakeContainers::default());
        let importer = AzureBills { store: blobs.clone() };
        assert!(importer.desired(st).await.unwrap().iter().any(|d| d.0 == org && d.1 == uid));
        let key = JobKey { provider: AZURE_PROVIDER.into(), org_id: org.clone(), account_id: uid.clone(), scope: "".into(), collected_until: None };
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp_millis();

        let Outcome::Failed(issue) = importer.run(st, &key, now).await.unwrap() else { panic!("no app registration") };
        assert!(issue.message.contains("Integrations → Azure"), "{}", issue.message);
        let body = json!({ "tenant_name": "tenant-1", "client_id": "client-1", "client_secret": "s3cret" });
        crate::routes::datadog_api::azure_integration::upsert_row(st, &org, body.as_object().unwrap()).await.unwrap();

        // Two daily runs of October's actual costs: the later one has the month to date.
        export(&blobs, "observecraft-actual", "20261001-20261031", "run-1", 100, &[("10/01/2026", "Virtual Machines", 5.0)]);
        export(&blobs, "observecraft-actual", "20261001-20261031", "run-2", 200, &[("10/01/2026", "Virtual Machines", 5.0), ("10/02/2026", "Storage", 1.0)]);
        export(&blobs, "observecraft-amortized", "20261001-20261031", "run-3", 300, &[("10/01/2026", "Virtual Machines", 4.0), ("10/02/2026", "Storage", 1.0)]);
        let outcome = importer.run(st, &key, now).await.unwrap();
        assert!(matches!(outcome, Outcome::Collected { metrics: 2, points: 4, warning: None, .. }), "{outcome:?}");
        assert_eq!(sums(st, &org, "azure").await, (6.0, 5.0, 4));
        // Only the export that delivered a new run is read again.
        assert!(matches!(importer.run(st, &key, now).await.unwrap(), Outcome::Collected { metrics: 0, .. }));
        export(&blobs, "observecraft-amortized", "20261001-20261031", "run-4", 400, &[("10/01/2026", "Virtual Machines", 4.5), ("10/02/2026", "Storage", 1.0)]);
        assert!(matches!(importer.run(st, &key, now).await.unwrap(), Outcome::Collected { metrics: 1, .. }));
        assert_eq!(sums(st, &org, "azure").await, (6.0, 5.5, 4));
    }
}
