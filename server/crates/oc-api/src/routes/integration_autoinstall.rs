//! Integrations installed by their logs, as Datadog does: when logs of a source arrive for the first time, the server
//! installs that integration's log pipeline from the library (`pipelines::library`) and, for integrations that send
//! only logs (Supply Chain Firewall, `source:scfw`), their dashboards (`integration_dashboards::log_only_logos`).
//! Each is installed once per org (Postgres `integration_autoinstalls`), so a pipeline or dashboard the org removes
//! stays removed. Runs every few minutes over the sources seen in the last hour (`oc.log_facets_1m`, cheap).

use std::collections::BTreeSet;
use std::time::Duration;

use crate::pipelines::library;
use crate::routes::log_pipelines::{insert, LogPipelineInput};
use crate::state::AppState;

const EVERY: Duration = Duration::from_secs(300);
const LOCK: i64 = 7330;

pub async fn run(st: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let mut conn = match st.postgres.acquire().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "integration auto-install: postgres unavailable");
                continue;
            }
        };
        if !sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)").bind(LOCK).fetch_one(&mut *conn).await.unwrap_or(false) {
            continue;
        }
        match sqlx::query_scalar::<_, String>("SELECT id FROM orgs WHERE status = 'active'").fetch_all(&st.postgres).await {
            Ok(orgs) => {
                for org in orgs {
                    if let Err(e) = install_for_org(&st, &org).await {
                        tracing::warn!(error = %e, org, "integration auto-install failed");
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "integration auto-install: listing orgs failed"),
        }
        let _ = sqlx::query("SELECT pg_advisory_unlock($1)").bind(LOCK).execute(&mut *conn).await;
    }
}

/// The log sources an org sent in the last hour.
async fn recent_sources(st: &AppState, org: &str) -> anyhow::Result<BTreeSet<String>> {
    let sources = st
        .ch(org)
        .query("SELECT DISTINCT toString(source) FROM oc.log_facets_1m_read WHERE org_id = ? AND minute >= now() - INTERVAL 1 HOUR AND source != ''")
        .bind(org)
        .fetch_all::<String>()
        .await?;
    Ok(sources.into_iter().collect())
}

/// The log sources a library pipeline's filter selects: `source:scfw`, `source:(a OR b)`; none for other filters.
pub fn filter_sources(filter: &str) -> Vec<String> {
    let Some(rest) = filter.trim().strip_prefix("source:") else {
        return Vec::new();
    };
    let rest = rest.trim();
    if let Some(inner) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(" OR ").map(str::trim).collect();
        if parts.iter().all(|p| !p.is_empty() && !p.contains(char::is_whitespace)) {
            return parts.into_iter().map(str::to_string).collect();
        }
        return Vec::new();
    }
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return Vec::new();
    }
    vec![rest.to_string()]
}

/// What a set of sources installs: library pipeline ids and log-only integrations (catalog ids).
pub fn wanted(sources: &BTreeSet<String>) -> (Vec<&'static str>, Vec<&'static str>) {
    let pipelines = library::all().iter().filter(|p| filter_sources(&p.filter).iter().any(|s| sources.contains(s))).map(|p| p.id.as_str()).collect();
    (pipelines, super::integration_dashboards::log_only_logos(sources))
}

/// Records an item as installed for the org; false when it already was (installed before, maybe removed since).
async fn claim(st: &AppState, org: &str, item: &str) -> anyhow::Result<bool> {
    let done = sqlx::query("INSERT INTO integration_autoinstalls (org_id, item) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(org).bind(item).execute(&st.postgres).await?;
    Ok(done.rows_affected() == 1)
}

pub async fn install_for_org(st: &AppState, org: &str) -> anyhow::Result<()> {
    let sources = recent_sources(st, org).await?;
    if sources.is_empty() {
        return Ok(());
    }
    let (pipelines, logos) = wanted(&sources);
    for id in pipelines {
        if !claim(st, org, &format!("pipeline:{id}")).await? {
            continue;
        }
        let Some(p) = library::get(id) else { continue };
        let input = LogPipelineInput { name: p.name.clone(), filter: p.filter.clone(), enabled: true, processors: p.processors.clone() };
        match insert(st, org, &input, &format!("integration:{}", p.id)).await {
            Ok(Some(_)) => tracing::info!(org, pipeline = p.id, "installed an integration pipeline on its first logs"),
            // Installed by hand before: nothing to do.
            Ok(None) => {}
            Err(e) => tracing::warn!(org, pipeline = p.id, error = %e, "installing an integration pipeline failed"),
        }
    }
    let mut fresh = Vec::new();
    for logo in logos {
        if claim(st, org, &format!("dashboards:{logo}")).await? {
            fresh.push(logo);
        }
    }
    if !fresh.is_empty() {
        super::integration_dashboards::install_missing(&st.postgres, org, &fresh).await?;
        super::dashboards::forget_enabled(org);
        tracing::info!(org, integrations = ?fresh, "installed integration dashboards on their first logs");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_filters_name_their_sources() {
        assert_eq!(filter_sources("source:scfw"), ["scfw"]);
        assert_eq!(filter_sources(" source:(nginx OR nginx-ingress) "), ["nginx", "nginx-ingress"]);
        assert!(filter_sources("source:nginx service:web").is_empty());
        assert!(filter_sources("service:web").is_empty());
        assert!(filter_sources("source:").is_empty());
    }

    #[test]
    fn supply_chain_firewall_logs_install_its_pipeline_and_dashboards() {
        let (pipelines, logos) = wanted(&BTreeSet::from(["scfw".to_string()]));
        assert_eq!(pipelines, ["supply_chain_firewall"]);
        assert_eq!(logos, ["supply-chain-firewall"]);
        let (pipelines, logos) = wanted(&BTreeSet::from(["nginx".to_string()]));
        assert!(pipelines.contains(&"nginx"), "{pipelines:?}");
        assert!(logos.is_empty(), "nginx sends metrics: {logos:?}");
        assert_eq!(wanted(&BTreeSet::new()), (vec![], vec![]));
    }
}
