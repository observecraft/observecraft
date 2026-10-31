//! Logs → Configuration, Datadog-compatible: the endpoints of the Terraform provider's
//! `datadog_logs_index` / `_index_order`, `datadog_logs_archive` / `_archive_order`,
//! `datadog_logs_custom_pipeline`, `datadog_logs_integration_pipeline`, `datadog_logs_pipeline_order`,
//! `datadog_logs_metric`, `datadog_logs_restriction_query` and `datadog_logs_custom_destination`.
//!
//! They read and write the records the UI edits (`log_indexes`, `log_archives`, `log_pipelines`,
//! `log_metrics`, `log_restriction_queries`, `log_custom_destinations`); see each module for how
//! Datadog's documents map onto them.

pub mod archives;
pub mod destinations;
pub mod indexes;
pub mod metrics;
pub mod pipelines;
pub mod restrictions;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/logs/config/indexes", get(indexes::list).post(indexes::create))
        .route("/api/v1/logs/config/indexes/:name", get(indexes::get).put(indexes::update).delete(indexes::delete))
        .route("/api/v1/logs/config/index-order", get(indexes::get_order).put(indexes::update_order))
        .route("/api/v2/logs/config/archives", get(archives::list).post(archives::create))
        .route("/api/v2/logs/config/archives/:id", get(archives::get).put(archives::update).delete(archives::delete))
        .route("/api/v2/logs/config/archive-order", get(archives::get_order).put(archives::update_order))
        .route("/api/v1/logs/config/pipelines", get(pipelines::list).post(pipelines::create))
        .route("/api/v1/logs/config/pipelines/:id", get(pipelines::get).put(pipelines::update).delete(pipelines::delete))
        .route("/api/v1/logs/config/pipeline-order", get(pipelines::get_order).put(pipelines::update_order))
        .route("/api/v2/logs/config/metrics", get(metrics::list).post(metrics::create))
        .route("/api/v2/logs/config/metrics/:id", get(metrics::get).patch(metrics::update).delete(metrics::delete))
        .route("/api/v2/logs/config/restriction_queries", get(restrictions::list).post(restrictions::create))
        .route("/api/v2/logs/config/restriction_queries/user/:user_id", get(restrictions::for_user))
        .route("/api/v2/logs/config/restriction_queries/role/:role_id", get(restrictions::for_role))
        .route(
            "/api/v2/logs/config/restriction_queries/:id",
            get(restrictions::get).patch(restrictions::update).put(restrictions::replace).delete(restrictions::delete),
        )
        .route("/api/v2/logs/config/restriction_queries/:id/roles", get(restrictions::list_roles).post(restrictions::grant_role).delete(restrictions::revoke_role))
        .route("/api/v2/logs/config/custom-destinations", get(destinations::list).post(destinations::create))
        .route("/api/v2/logs/config/custom-destinations/:id", get(destinations::get).patch(destinations::update).delete(destinations::delete))
}

/// A search query as Datadog writes it: `*` for every log, where we store ''.
pub(crate) fn dd_query(q: &str) -> String {
    if q.trim().is_empty() {
        "*".into()
    } else {
        q.to_string()
    }
}

/// Our stored query: '' for every log.
pub(crate) fn our_query(q: &str) -> String {
    match q.trim() {
        "*" => String::new(),
        q => q.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::replay::{Feature, TestServer};

    /// Replays a feature's recorded scenarios; returns the problems found.
    async fn replay(feature: &str, server: &str, at_least: usize) {
        let Some(f) = Feature::load(feature) else { return };
        let Some(s) = TestServer::start(server).await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() || !f.replayed(scenario) {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= at_least, "{feature}: {replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[tokio::test]
    async fn replays_datadogs_logs_pipelines_scenarios() {
        replay("v1/logs_pipelines", "dd-logs-pipelines-replay", 15).await;
    }

    #[tokio::test]
    async fn replays_datadogs_logs_metrics_scenarios() {
        replay("v2/logs_metrics", "dd-logs-metrics-replay", 5).await;
    }

    #[tokio::test]
    async fn replays_datadogs_logs_restriction_queries_scenarios() {
        replay("v2/logs_restriction_queries", "dd-logs-restrictions-replay", 15).await;
    }

    #[tokio::test]
    async fn replays_datadogs_logs_custom_destinations_scenarios() {
        replay("v2/logs_custom_destinations", "dd-logs-destinations-replay", 20).await;
    }
}
