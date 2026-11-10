//! A package's monitors, Cloud SIEM rules and log-based metrics (`assets.monitors`, `assets.security_rules`,
//! `assets.log_metrics`, in Datadog's JSON), created for the org when a connection is saved, through the Datadog API
//! handlers that create them from the same documents (`datadog_api::monitors`, `::security_monitoring`,
//! `::logs::metrics`), as the admin saving it. Postgres `connector_assets` (migration 0103) records each one:
//!
//! - Idempotent: an asset with a row isn't created again, so saving twice, a second connection or an asset the org
//!   deleted on purpose create nothing. Monitors and rules are tagged `connector:<id>`. A log-based metric whose
//!   name the org already has is the org's: left alone, no row.
//! - An asset that can't be created (a query this install doesn't support) is skipped and logged; the save goes on.
//! - Removing the org's last connection removes what the rows name and the package's log pipeline, as uninstalling
//!   an integration removes its pipelines (`integration_installs::uninstall`); its dashboards stay.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use serde_json::{json, Value};

use super::PackageConnector;
use crate::auth::AuthContext;
use crate::routes::datadog_api::{logs, monitors, security_monitoring, DdAuth};
use crate::state::AppState;

/// The tag on the monitors and rules a package created.
pub fn tag(connector: &str) -> String {
    format!("connector:{connector}")
}

fn with_tag(mut doc: Value, tag: &str) -> Value {
    let tags = doc.as_object_mut().map(|o| o.entry("tags").or_insert_with(|| json!([])));
    if let Some(Value::Array(list)) = tags {
        if !list.iter().any(|t| t.as_str() == Some(tag)) {
            list.push(json!(tag));
        }
    }
    doc
}

/// A monitor document: integrations-core's `{definition: {…}}` or the API's own.
fn monitor_doc(doc: &Value) -> Value {
    doc.get("definition").filter(|d| d.is_object()).cloned().unwrap_or_else(|| doc.clone())
}

/// A log-based metric document as the API takes it (`{data: {id, type, attributes}}`), from it or its attributes.
fn log_metric_doc(doc: &Value) -> Option<Value> {
    if doc.get("data").is_some() {
        return Some(doc.clone());
    }
    let id = doc.get("id").or_else(|| doc.get("name")).and_then(Value::as_str)?;
    let mut attributes = doc.clone();
    if let Some(o) = attributes.as_object_mut() {
        o.remove("id");
        o.remove("name");
    }
    Some(json!({ "data": { "id": id, "type": "logs_metrics", "attributes": attributes } }))
}

async fn recorded(st: &AppState, org: &str, connector: &str, kind: &str, asset: &str) -> Result<bool, sqlx::Error> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM connector_assets WHERE org_id = $1 AND connector_id = $2 AND kind = $3 AND asset = $4")
        .bind(org)
        .bind(connector)
        .bind(kind)
        .bind(asset)
        .fetch_one(&st.postgres)
        .await?;
    Ok(n > 0)
}

async fn record(st: &AppState, org: &str, connector: &str, kind: &str, asset: &str, reference: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO connector_assets (org_id, connector_id, kind, asset, ref) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING")
        .bind(org)
        .bind(connector)
        .bind(kind)
        .bind(asset)
        .bind(reference)
        .execute(&st.postgres)
        .await?;
    Ok(())
}

/// Creates the package's monitors, rules and log-based metrics the org hasn't had yet. Answers what was created
/// (`kind:asset`) and the problems of those skipped.
pub async fn install(st: &AppState, ctx: &AuthContext, p: &PackageConnector) -> Result<(Vec<String>, Vec<String>), sqlx::Error> {
    let m = &p.package.manifest;
    let (org, id) = (ctx.org_id.as_str(), m.id.as_str());
    let tag = tag(id);
    let mut created = Vec::new();
    let mut problems = Vec::new();
    let json_of = |path: &str| p.package.file(path).and_then(|t| serde_json::from_str::<Value>(t).ok());
    for (kind, paths) in [("monitor", &m.assets.monitors), ("security_rule", &m.assets.security_rules), ("log_metric", &m.assets.log_metrics)] {
        for path in paths {
            if recorded(st, org, id, kind, path).await? {
                continue;
            }
            let Some(doc) = json_of(path) else { continue };
            let auth = DdAuth(ctx.clone());
            let result = match kind {
                "monitor" => monitors::create(State(st.clone()), auth, Bytes::from(with_tag(monitor_doc(&doc), &tag).to_string())).await.map(|j| j.0["id"].to_string()),
                "security_rule" => {
                    security_monitoring::create_rule(State(st.clone()), auth, Bytes::from(with_tag(doc, &tag).to_string())).await.map(|(_, j)| j.0["id"].as_str().unwrap_or_default().to_string())
                }
                _ => match log_metric_doc(&doc) {
                    None => Err(crate::routes::datadog_api::DdError { status: axum::http::StatusCode::BAD_REQUEST, errors: vec!["a log-based metric needs an id".into()] }),
                    Some(d) => match logs::metrics::create(State(st.clone()), auth, Bytes::from(d.to_string())).await {
                        Ok((_, j)) => Ok(j.0["data"]["id"].as_str().unwrap_or_default().to_string()),
                        // The org's own metric of that name: not ours to manage.
                        Err(e) if e.status == axum::http::StatusCode::CONFLICT => continue,
                        Err(e) => Err(e),
                    },
                },
            };
            match result {
                Ok(reference) if !reference.is_empty() && reference != "null" => {
                    record(st, org, id, kind, path, &reference).await?;
                    created.push(format!("{kind}:{path}"));
                }
                Ok(_) => problems.push(format!("{path}: created without an id")),
                Err(e) => {
                    tracing::warn!(connector = id, asset = %path, errors = %e.errors.join("; "), "connector asset not created");
                    problems.push(format!("{path}: {}", e.errors.join("; ")));
                }
            }
        }
    }
    Ok((created, problems))
}

/// Removes what the package created for the org and its log pipeline (its last connection is gone).
pub async fn remove(st: &AppState, ctx: &AuthContext, connector: &str) -> Result<(), sqlx::Error> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, ref FROM connector_assets WHERE org_id = $1 AND connector_id = $2").bind(&ctx.org_id).bind(connector).fetch_all(&st.postgres).await?;
    for (kind, reference) in rows {
        let auth = DdAuth(ctx.clone());
        // Gone already (the org deleted it) is fine.
        let _ = match kind.as_str() {
            "monitor" => monitors::delete(State(st.clone()), auth, Path(reference.clone()), Query([("force".to_string(), "true".to_string())].into_iter().collect())).await.map(|_| ()),
            "security_rule" => security_monitoring::delete_rule(State(st.clone()), auth, Path(reference.clone())).await.map(|_| ()),
            _ => logs::metrics::delete(State(st.clone()), auth, Path(reference.clone())).await.map(|_| ()),
        };
    }
    sqlx::query("DELETE FROM connector_assets WHERE org_id = $1 AND connector_id = $2").bind(&ctx.org_id).bind(connector).execute(&st.postgres).await?;
    sqlx::query("DELETE FROM log_pipelines WHERE org_id = $1 AND source = $2").bind(&ctx.org_id).bind(format!("connector:{connector}")).execute(&st.postgres).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_take_the_apis_shape() {
        let wrapped = json!({ "version": 2, "definition": { "name": "Errors", "type": "log alert", "tags": ["team:billing"] } });
        assert_eq!(with_tag(monitor_doc(&wrapped), "connector:x")["tags"], json!(["team:billing", "connector:x"]));
        assert_eq!(with_tag(with_tag(json!({ "name": "R" }), "connector:x"), "connector:x")["tags"], json!(["connector:x"]));
        let flat = json!({ "id": "example.logins", "compute": { "aggregation_type": "count" }, "filter": { "query": "source:example" } });
        assert_eq!(
            log_metric_doc(&flat).unwrap(),
            json!({ "data": { "id": "example.logins", "type": "logs_metrics", "attributes": { "compute": { "aggregation_type": "count" }, "filter": { "query": "source:example" } } } })
        );
        assert_eq!(log_metric_doc(&json!({ "data": { "id": "a" } })).unwrap()["data"]["id"], "a");
        assert!(log_metric_doc(&json!({ "compute": {} })).is_none());
    }

    const PACKAGE: &str = "schema_version: 1\nid: example-audit\nname: Example Audit\npublisher: Example Corp\nvendor: Example Corp\ndescription: Audit events.\n\
        streams:\n  - id: events\n    name: Events\n    description: Events.\n    request: { url: 'https://audit.example.com/api/events' }\n    output:\n      logs: { message: '{{ record.name }}' }\n\
        test:\n  request: { url: 'https://audit.example.com/api/events' }\n  message: ok\n\
        assets:\n  monitors: [assets/monitors/failures.json]\n  security_rules: [assets/rules/brute.json]\n  log_metrics: [assets/metrics/logins.json, assets/metrics/taken.json]\n";

    fn package() -> std::sync::Arc<PackageConnector> {
        let dir = std::env::temp_dir().join(format!("oc-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("example-audit");
        for d in ["assets/monitors", "assets/rules", "assets/metrics"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("connector.yaml"), PACKAGE).unwrap();
        let monitor = json!({ "name": "Example Audit: many failed logins", "type": "log alert", "query": "logs(\"source:example-audit @evt.outcome:failure\").index(\"*\").rollup(\"count\").last(\"5m\") > 20", "message": "Many failed logins.", "tags": ["source:example-audit"], "options": { "thresholds": { "critical": 20 } } });
        std::fs::write(root.join("assets/monitors/failures.json"), monitor.to_string()).unwrap();
        let rule = json!({ "name": "Example Audit: brute force", "type": "log_detection", "isEnabled": true, "message": "Brute force on {{@usr.id}}",
            "queries": [{ "name": "failures", "query": "source:example-audit @evt.outcome:failure", "aggregation": "count", "groupByFields": ["@usr.id"] }],
            "cases": [{ "name": "many", "status": "medium", "condition": "failures > 10" }],
            "options": { "evaluationWindow": 300, "keepAlive": 3600, "maxSignalDuration": 86400, "detectionMethod": "threshold" }, "tags": [] });
        std::fs::write(root.join("assets/rules/brute.json"), rule.to_string()).unwrap();
        std::fs::write(
            root.join("assets/metrics/logins.json"),
            json!({ "id": "example_audit.logins", "compute": { "aggregation_type": "count" }, "filter": { "query": "source:example-audit" } }).to_string(),
        )
        .unwrap();
        std::fs::write(
            root.join("assets/metrics/taken.json"),
            json!({ "id": "orgs.own.metric", "compute": { "aggregation_type": "count" }, "filter": { "query": "source:example-audit" } }).to_string(),
        )
        .unwrap();
        let r = super::super::load(Some(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(r.invalid.is_empty(), "{:?}", r.invalid);
        r.package("example-audit").unwrap().clone()
    }

    async fn admin(st: &AppState, prefix: &str) -> AuthContext {
        let mut ctx = crate::test_support::admin_of_new_org(st, prefix).await;
        let (user,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Assets Admin') RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(format!("{}@example.com", ctx.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        ctx.user_id = user;
        ctx
    }

    async fn count(st: &AppState, sql: &str, org: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(org).fetch_one(&st.postgres).await.unwrap()
    }

    /// Created once, tagged, the org's own metric left alone; another org's untouched; removed with the last
    /// connection, an asset deleted by the org not created again.
    #[tokio::test]
    async fn package_assets_are_created_once_and_removed_with_the_last_connection() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let p = package();
        let a = admin(st, "assets-a").await;
        let b = admin(st, "assets-b").await;
        sqlx::query("INSERT INTO log_metrics (org_id, name, query, aggregation, path, group_by, include_percentiles) VALUES ($1, 'orgs.own.metric', 'service:mine', 'count', NULL, '[]', false)")
            .bind(&a.org_id)
            .execute(&st.postgres)
            .await
            .unwrap();
        let (created, problems) = install(st, &a, &p).await.unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(created, ["monitor:assets/monitors/failures.json", "security_rule:assets/rules/brute.json", "log_metric:assets/metrics/logins.json"]);
        let monitors = "SELECT count(*) FROM monitors WHERE org_id = $1 AND 'connector:example-audit' = ANY(tags)";
        let rules = "SELECT count(*) FROM security_rules WHERE org_id = $1 AND 'connector:example-audit' = ANY(tags)";
        let metric = "SELECT count(*) FROM log_metrics WHERE org_id = $1 AND name = 'example_audit.logins'";
        let own = "SELECT count(*) FROM log_metrics WHERE org_id = $1 AND name = 'orgs.own.metric' AND query = 'service:mine'";
        assert_eq!((count(st, monitors, &a.org_id).await, count(st, rules, &a.org_id).await, count(st, metric, &a.org_id).await, count(st, own, &a.org_id).await), (1, 1, 1, 1));
        assert_eq!(install(st, &a, &p).await.unwrap().0, Vec::<String>::new(), "idempotent");
        assert_eq!((count(st, monitors, &a.org_id).await, count(st, rules, &a.org_id).await), (1, 1));
        assert_eq!(count(st, monitors, &b.org_id).await, 0);
        // The org deletes the monitor: a later save doesn't bring it back.
        sqlx::query("DELETE FROM monitors WHERE org_id = $1 AND 'connector:example-audit' = ANY(tags)").bind(&a.org_id).execute(&st.postgres).await.unwrap();
        assert!(install(st, &a, &p).await.unwrap().0.is_empty());
        assert_eq!(count(st, monitors, &a.org_id).await, 0);
        install(st, &b, &p).await.unwrap();
        remove(st, &a, "example-audit").await.unwrap();
        assert_eq!((count(st, rules, &a.org_id).await, count(st, metric, &a.org_id).await, count(st, own, &a.org_id).await), (0, 0, 1), "the org's own metric stays");
        assert_eq!((count(st, monitors, &b.org_id).await, count(st, rules, &b.org_id).await), (1, 1), "B's are B's");
        // Installed again after removal (a new first connection).
        assert_eq!(install(st, &a, &p).await.unwrap().0.len(), 3);
    }
}
