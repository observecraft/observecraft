//! Connectors (docs/adr/0007-connectors.md, docs/connectors.md): the integrations that call a vendor's API. Each is
//! a [`Connector`]: a package run by the engine (`oc-connectors`, bundled in `integrations/connectors/` or the
//! operator's in `OC_CONNECTORS_DIR`), or a native one with its own code and page (authentik, FleetDM, GitLab Audit
//! Events, Google Workspace, Entra ID). What is generic about them — the catalog entry, whether an org is connected,
//! its connections and their jobs' status, syncing now — goes through the trait, so the catalog, installed state and
//! dashboard gating don't list them one by one.
//!
//! - [`registry`]: every connector, loaded once.
//! - [`assets`]: packages' monitors, Cloud SIEM rules and log-based metrics, created with the first connection.
//! - [`store`]: packages' connections, stream state and request quotas in Postgres.
//! - [`runner`]: packages' streams on the cloud crawlers' jobs, published to Kafka.
//! - [`net`]: the SSRF-guarded transport packages' requests go through.
//! - [`push`]: push packages' definitions and signing secrets for the intake.
//! - [`sources`]: native sources (`native:` packages: CrowdStrike FDR's SQS and S3), their sessions on the same jobs.

pub mod assets;
pub mod native;
pub mod net;
pub mod push;
pub mod runner;
pub mod sources;
pub mod store;

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

pub use oc_connectors::catalog::{entry_of, ConnectorEntry, ConnectorKind};
use oc_connectors::package::{Origin, Package};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

mod bundled {
    include!(concat!(env!("OUT_DIR"), "/bundled_connectors.rs"));
}

/// One of an org's connections, as the generic pages list it.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionRef {
    pub id: String,
    pub name: String,
}

/// A connector as the catalog shows it.
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    fn entry(&self) -> &ConnectorEntry;
    /// The jobs' provider on `cloud_crawl_jobs` (`account_id` is a connection's id).
    fn provider(&self) -> &str;
    /// The org's connections.
    async fn connections(&self, st: &AppState, org: &str) -> Result<Vec<ConnectionRef>, sqlx::Error>;
}

/// A connector run by the engine from a package.
pub struct PackageConnector {
    pub package: Arc<Package>,
    entry: ConnectorEntry,
}

#[async_trait::async_trait]
impl Connector for PackageConnector {
    fn entry(&self) -> &ConnectorEntry {
        &self.entry
    }

    fn provider(&self) -> &str {
        &self.package.manifest.id
    }

    async fn connections(&self, st: &AppState, org: &str) -> Result<Vec<ConnectionRef>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT id::text, name FROM connector_connections WHERE org_id = $1 AND connector_id = $2 ORDER BY name")
            .bind(org)
            .bind(&self.package.manifest.id)
            .fetch_all(&st.postgres)
            .await?;
        Ok(rows.into_iter().map(|(id, name)| ConnectionRef { id, name }).collect())
    }
}

/// Every connector, and the packages that couldn't be loaded.
pub struct Registry {
    pub connectors: Vec<Arc<dyn Connector>>,
    pub packages: Vec<Arc<PackageConnector>>,
    /// (directory, problems) of every invalid package.
    pub invalid: Vec<(String, Vec<String>)>,
}

impl Registry {
    pub fn get(&self, id: &str) -> Option<&Arc<dyn Connector>> {
        self.connectors.iter().find(|c| c.entry().id == id)
    }

    pub fn package(&self, id: &str) -> Option<&Arc<PackageConnector>> {
        self.packages.iter().find(|p| p.package.manifest.id == id)
    }

    /// The logos of connectors' dashboards: listed once the org has a connection.
    pub fn gated_logos(&self) -> BTreeSet<String> {
        self.connectors.iter().map(|c| c.entry().logo.clone()).collect()
    }
}

/// The bundled packages' files, by package directory.
pub fn bundled_files() -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> {
    let mut out: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> = Default::default();
    for (dir, path, text) in bundled::FILES {
        out.entry(dir.to_string()).or_default().insert(path.to_string(), text.to_string());
    }
    out
}

/// Loads the bundled packages and the operator's (`dir`): an invalid package is left out with its problems; an
/// operator's package can't replace a bundled or native connector.
pub fn load(dir: Option<&str>) -> Registry {
    let mut packages: Vec<Package> = Vec::new();
    let mut invalid = Vec::new();
    for (name, files) in bundled_files() {
        match Package::from_files(files, Origin::Bundled) {
            Ok(p) => packages.push(p),
            Err(problems) => invalid.push((format!("integrations/connectors/{name}"), problems)),
        }
    }
    // A package naming a native source this server doesn't have can't run.
    let unknown_native = |p: &Package| p.manifest.native.as_deref().filter(|n| !sources::NAMES.contains(n)).map(|n| vec![format!("native: {n} is not a native source of this server")]);
    let (packages, unrunnable): (Vec<Package>, Vec<Package>) = packages.into_iter().partition(|p| unknown_native(p).is_none());
    invalid.extend(unrunnable.iter().map(|p| (format!("integrations/connectors/{}", p.manifest.id), unknown_native(p).unwrap_or_default())));
    let mut packages = packages;
    let natives = native::all();
    if let Some(dir) = dir.filter(|d| !d.trim().is_empty()) {
        let (theirs, bad) = oc_connectors::package::load_all(std::path::Path::new(dir), |p| Origin::ThirdParty { dir: p.display().to_string() });
        invalid.extend(bad);
        for p in theirs {
            let id = p.manifest.id.clone();
            if packages.iter().any(|b| b.manifest.id == id) || natives.iter().any(|n| n.entry().id == id) {
                invalid.push((format!("{dir}/{id}"), vec![format!("id: {id} is already a connector of ObserveCraft's")]));
            } else if let Some(problems) = unknown_native(&p) {
                invalid.push((format!("{dir}/{id}"), problems));
            } else {
                packages.push(p);
            }
        }
    }
    let packages: Vec<Arc<PackageConnector>> = packages.into_iter().map(|p| Arc::new(PackageConnector { entry: entry_of(&p), package: Arc::new(p) })).collect();
    let mut connectors: Vec<Arc<dyn Connector>> = natives;
    connectors.extend(packages.iter().map(|p| p.clone() as Arc<dyn Connector>));
    connectors.sort_by_key(|c| c.entry().name.to_lowercase());
    Registry { connectors, packages, invalid }
}

/// The registry of this server, loaded on first use (`OC_CONNECTORS_DIR` for the operator's packages); invalid
/// packages are logged once.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let dir = std::env::var("OC_CONNECTORS_DIR").ok();
        let r = load(dir.as_deref());
        for (dir, problems) in &r.invalid {
            tracing::error!(package = %dir, problems = %problems.join("; "), "connector package not loaded");
        }
        tracing::info!(connectors = r.connectors.len(), packages = r.packages.len(), "connectors loaded");
        r
    })
}

/// The unit a package declares for a metric it writes (`unit` of a metric with a fixed name).
pub fn unit(metric: &str) -> Option<String> {
    static UNITS: OnceLock<std::collections::HashMap<String, String>> = OnceLock::new();
    UNITS
        .get_or_init(|| {
            registry()
                .packages
                .iter()
                .flat_map(|p| p.package.manifest.streams.iter().flat_map(|s| s.output.metrics.iter()))
                .filter(|m| !m.name.contains("{{"))
                .filter_map(|m| Some((m.name.clone(), m.unit.clone().filter(|u| !u.contains("{{"))?)))
                .collect()
        })
        .get(metric)
        .cloned()
}

/// The catalog tiles and logos of the connectors the org has a connection of (installed tiles, listed dashboards).
pub async fn connected(st: &AppState, org: &str) -> Result<BTreeSet<String>, sqlx::Error> {
    let mut out = BTreeSet::new();
    let ids: Vec<String> = sqlx::query_scalar("SELECT DISTINCT connector_id FROM connector_connections WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let r = registry();
    for c in &r.connectors {
        let e = c.entry();
        let on = if e.kind == ConnectorKind::Native { native::connected(st, org, &e.id).await? } else { ids.contains(&e.id) };
        if on {
            out.insert(e.id.clone());
            out.insert(e.logo.clone());
            out.extend(e.tiles.iter().cloned());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bundled package is valid, its tiles are catalog ids, and its recorded answers give what it expects.
    #[tokio::test]
    async fn every_bundled_package_loads_and_replays() {
        let r = load(None);
        assert!(r.invalid.is_empty(), "{:#?}", r.invalid);
        // Catalog tiles: integrations-core's and extras' (generated), and ObserveCraft's own (the mock catalog).
        let catalog: serde_json::Value = serde_json::from_str(include_str!("../../../../../web/src/api/integrations/catalog.generated.json")).unwrap();
        let tiles: BTreeSet<&str> = catalog["integrations"].as_array().unwrap().iter().filter_map(|i| i["id"].as_str()).collect();
        let own = include_str!("../../../../../web/src/api/mock/integrations.ts");
        for p in &r.packages {
            let m = &p.package.manifest;
            for t in &m.tiles {
                assert!(tiles.contains(t.as_str()) || own.contains(&format!("id: '{t}'")) || t == &m.id, "{}: tile {t} is not in the catalog (a package adds its own tile with its id)", m.id);
            }
            let report = oc_connectors::replay::replay(&p.package, false).await;
            assert!(report.problems.is_empty(), "{}: {:#?}", m.id, report.problems);
        }
    }

    /// A record's own `status` reaches the log store as the `status` attribute, apart from the log's status.
    #[test]
    fn a_records_status_stays_an_attribute_after_normalizing() {
        let m: oc_connectors::manifest::Manifest = serde_yaml_ng::from_str(
            "schema_version: 1\nid: example-jobs\nname: Example Jobs\npublisher: Example\nvendor: Example\ndescription: Jobs.\n\
             streams:\n  - id: jobs\n    name: Jobs\n    description: Jobs.\n    request: { url: 'https://jobs.example.com/api/jobs' }\n    output:\n\
             \x20     logs: { message: '{{ record.name }} finished', status: \"{{ record.failed ? 'error' : 'info' }}\", attributes: \"{{ record }}\" }\n",
        )
        .unwrap();
        let mapper = oc_connectors::output::Mapper::compile(&m, &m.streams[0]).unwrap();
        let normalized = |record: serde_json::Value| {
            let mut scope = serde_json::Map::new();
            scope.insert("record".into(), record);
            let mut out = oc_connectors::output::Mapped::default();
            mapper.map(&oc_connectors::output::Item { scope: &scope, time_ms: None, id: "1", instance: "jobs.example.com", now_ms: 1_794_312_000_000 }, &mut out);
            let mut fields = out.logs.remove(0).as_object().unwrap().clone();
            fields.insert("source_type".into(), serde_json::json!("kafka"));
            fields.insert("_oc_kafka_headers".into(), serde_json::json!({ "x-oc-org": "org-1" }));
            crate::log_normalize::normalize(crate::log_normalize::LogEvent { fields, timestamp: None }, chrono::Utc::now()).unwrap()
        };
        let row = normalized(serde_json::json!({ "name": "nightly-export", "failed": true, "status": "DONE", "rows": 12 }));
        assert_eq!((row.message.as_str(), row.status.as_str(), row.service.as_str()), ("nightly-export finished", "error", "example-jobs"));
        assert_eq!((row.attr.get("status").map(String::as_str), row.attr.get("rows").map(String::as_str), row.attr.get("level")), (Some("DONE"), Some("12"), None));
        let row = normalized(serde_json::json!({ "name": "nightly-export", "failed": false }));
        assert_eq!((row.message.as_str(), row.status.as_str(), row.attr.get("status")), ("nightly-export finished", "info", None));
    }

    #[test]
    fn operator_packages_load_beside_the_bundled_ones_and_bad_ones_are_named() {
        let dir = std::env::temp_dir().join(format!("oc-connectors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let good = dir.join("example-status");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("connector.yaml"),
            "schema_version: 1\nid: example-status\nname: Example Status\npublisher: Example Corp\nvendor: Example Corp\ndescription: Incidents of a status page.\n\
             streams:\n  - id: incidents\n    name: Incidents\n    description: Incidents.\n    request: { url: 'https://status.example.com/api/incidents' }\n    output:\n      logs: { message: '{{ record.name }}' }\n\
             test:\n  request: { url: 'https://status.example.com/api/incidents' }\n  message: ok\n",
        )
        .unwrap();
        let clash = dir.join("authentik");
        std::fs::create_dir_all(&clash).unwrap();
        std::fs::write(clash.join("connector.yaml"), std::fs::read_to_string(good.join("connector.yaml")).unwrap().replace("example-status", "authentik")).unwrap();
        let broken = dir.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("connector.yaml"), "schema_version: 1\nid: broken\n").unwrap();
        // A ConfigMap's flat files: <package>__<path>.
        std::fs::write(dir.join("example-flat__connector.yaml"), std::fs::read_to_string(good.join("connector.yaml")).unwrap().replace("example-status", "example-flat")).unwrap();
        let r = load(Some(dir.to_str().unwrap()));
        let theirs = r.package("example-status").expect("loaded");
        assert!(entry_of(&theirs.package).third_party);
        assert!(r.get("example-status").is_some());
        assert!(r.package("example-flat").is_some(), "flat ConfigMap files");
        let problems: Vec<String> = r.invalid.iter().map(|(d, p)| format!("{d}: {}", p.join("; "))).collect();
        assert!(problems.iter().any(|p| p.contains("broken") && p.contains("missing field")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("authentik is already a connector")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
