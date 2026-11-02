//! Every integration's dashboards from integrations-core (`seeds/integrations/core/dashboards.ndjson.gz`, written by
//! `scripts/import-integration-dashboards.mjs --all`) and ours of each cloud service (`seeds/integrations/cloud/services.ndjson.gz`). They aren't copied into every org at startup (hundreds of
//! dashboards per org): an org gets an integration's dashboards once that integration sends metrics, as Datadog lists
//! an integration's dashboards once it is installed. Installed ones are refreshed with the bundle at startup.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::Read;
use std::sync::OnceLock;

/// integrations-core's dashboards, and ours of each AWS / Azure / Google Cloud service (`scripts/cloud-service-dashboards.mjs`).
const BUNDLES: [&[u8]; 2] = [include_bytes!("../../../../seeds/integrations/core/dashboards.ndjson.gz"), include_bytes!("../../../../seeds/integrations/cloud/services.ndjson.gz")];

/// One integration of the bundle: its catalog id (the dashboards' `logo`), metric prefix and raw line.
struct Entry {
    logo: String,
    prefix: String,
    line: String,
}

#[derive(Deserialize)]
struct Head {
    logo: String,
    prefix: String,
}

fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let mut entries = Vec::new();
        for bundle in BUNDLES {
            let mut text = String::new();
            if let Err(e) = flate2::read::GzDecoder::new(bundle).read_to_string(&mut text) {
                tracing::error!(error = %e, "an integration dashboards bundle is unreadable");
                continue;
            }
            entries.extend(text.lines().filter_map(|line| {
                let head: Head = serde_json::from_str(line).ok()?;
                Some(Entry { logo: head.logo, prefix: head.prefix, line: line.to_string() })
            }));
        }
        entries
    })
}

/// Whether metrics seen (by their first two name segments, `postgresql.connections` → `postgresql.connections`,
/// `datadog.cluster_agent.x` → `datadog.cluster_agent`) belong to an integration with metric `prefix`.
pub fn prefix_matches(prefix: &str, seen: &str) -> bool {
    let prefix = prefix.trim_end_matches('.');
    !prefix.is_empty() && (seen == prefix || seen.starts_with(&format!("{prefix}.")) || prefix.starts_with(&format!("{seen}.")))
}

/// The integrations (catalog ids) whose metrics are among `seen` (two-segment metric name prefixes).
pub fn enabled_logos(seen: &BTreeSet<String>) -> Vec<&'static str> {
    entries().iter().filter(|e| seen.iter().any(|s| prefix_matches(&e.prefix, s))).map(|e| e.logo.as_str()).collect()
}

/// The dashboards of one integration, as seed documents (`id`, document).
pub fn dashboards_of(logo: &str) -> Vec<(String, serde_json::Value)> {
    #[derive(Deserialize)]
    struct Line {
        dashboards: Vec<serde_json::Value>,
    }
    let Some(entry) = entries().iter().find(|e| e.logo == logo) else {
        return Vec::new();
    };
    let Ok(line) = serde_json::from_str::<Line>(&entry.line) else {
        return Vec::new();
    };
    line.dashboards.into_iter().filter_map(|d| Some((d["id"].as_str()?.to_string(), d))).collect()
}

/// Installs the dashboards of the `logos` the org doesn't have yet (on first data of an integration).
pub async fn install_missing(pg: &sqlx::PgPool, org: &str, logos: &[&str]) -> anyhow::Result<()> {
    if logos.is_empty() {
        return Ok(());
    }
    let wanted: Vec<String> = logos.iter().map(|l| l.to_string()).collect();
    let present: Vec<String> = sqlx::query_scalar("SELECT DISTINCT logo FROM dashboards WHERE org_id = $1 AND kind = 'integration' AND logo = ANY($2)").bind(org).bind(&wanted).fetch_all(pg).await?;
    for logo in logos.iter().filter(|l| !present.iter().any(|p| p == *l)) {
        for (id, doc) in dashboards_of(logo) {
            super::dashboards::upsert_seed(pg, org, None, &id, &doc).await?;
        }
    }
    Ok(())
}

/// Refreshes the org's installed integration dashboards from the bundle (a new integrations-core revision).
pub async fn refresh_installed(pg: &sqlx::PgPool, org: &str) -> anyhow::Result<()> {
    let installed: Vec<String> = sqlx::query_scalar("SELECT DISTINCT logo FROM dashboards WHERE org_id = $1 AND kind = 'integration' AND logo IS NOT NULL").bind(org).fetch_all(pg).await?;
    for logo in installed.iter().filter(|l| entries().iter().any(|e| &e.logo == *l)) {
        for (id, doc) in dashboards_of(logo) {
            super::dashboards::upsert_seed(pg, org, None, &id, &doc).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_prefixes_match_on_whole_segments() {
        assert!(prefix_matches("postgresql.", "postgresql.connections"));
        assert!(prefix_matches("postgresql.", "postgresql"));
        assert!(prefix_matches("datadog.cluster_agent.", "datadog.cluster_agent"));
        assert!(prefix_matches("aws.ec2.instance.", "aws.ec2"), "a deeper prefix matches its two first segments");
        assert!(!prefix_matches("postgresql.", "postgres.connections"));
        assert!(!prefix_matches("redis.", "redisdb.x"));
        assert!(!prefix_matches("", "anything"));
    }

    #[test]
    fn the_bundle_carries_postgres_with_its_metric_prefix() {
        let titles: BTreeSet<String> = dashboards_of("postgres").iter().map(|(_, d)| d["title"].as_str().unwrap().to_string()).collect();
        assert_eq!(titles, BTreeSet::from(["Postgres - Metrics".to_string(), "Postgres - Overview".to_string()]));
        for (id, doc) in dashboards_of("postgres") {
            assert_eq!(doc["logo"], "postgres", "{id}");
            assert_eq!(doc["kind"], "integration", "{id}");
        }
        assert!(entries().len() > 250);
    }

    #[test]
    fn cloud_services_get_their_dashboard_when_their_metrics_arrive() {
        let enabled = enabled_logos(&BTreeSet::from(["aws.lambda".to_string()]));
        assert!(enabled.contains(&"tile:aws-lambda"), "{enabled:?}");
        let titles: Vec<String> = dashboards_of("tile:aws-lambda").iter().map(|(_, d)| d["title"].as_str().unwrap().to_string()).collect();
        assert_eq!(titles, ["AWS Lambda"]);
        let enabled = enabled_logos(&BTreeSet::from(["azure.vm".to_string(), "gcp.gce".to_string()]));
        assert!(enabled.contains(&"tile:azure-compute-virtualmachines") && enabled.contains(&"tile:gcp-gce"), "{enabled:?}");
    }

    #[test]
    fn integrations_are_enabled_by_the_metrics_seen() {
        let seen = BTreeSet::from(["postgresql.connections".to_string(), "system.cpu".to_string()]);
        let enabled = enabled_logos(&seen);
        assert!(enabled.contains(&"postgres"), "{enabled:?}");
        assert!(!enabled.contains(&"mysql"));
        assert!(enabled_logos(&BTreeSet::new()).is_empty());
    }
}
