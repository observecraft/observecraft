pub mod apm;
pub mod archives;
pub mod auth;
pub mod ci;
pub mod dashboards;
pub mod downtimes;
pub mod events;
pub mod health;
pub mod hosts;
pub mod identity;
pub mod incident_views;
pub mod incidents;
pub mod ingest_keys;
pub mod instance;
pub mod kubernetes;
pub mod live;
pub mod log_indexes;
pub mod logs;
pub mod metrics;
pub mod monitor_engine;
pub mod monitors;
pub mod network;
pub mod notifications;
pub mod profiles;
pub mod spans;
pub mod sso;

use axum::routing::{delete, get, patch, post};
use axum::Router;
use utoipa::OpenApi;

use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    info(title = "ObserveCraft API", description = "API consumed by the ObserveCraft web app."),
    paths(
        health::healthz,
        health::readyz,
        logs::search,
        logs::histogram,
        logs::facets,
        logs::top,
        logs::patterns,
        logs::suggest,
        logs::insights,
        live::live,
        apm::services,
        apm::resources,
        apm::service,
        apm::deployments,
        apm::dependencies,
        apm::databases,
        apm::errors,
        apm::search_traces,
        apm::trace,
        apm::recommendations,
        apm::anomalies,
        apm::envs,
        metrics::timeseries,
        metrics::dashboard::dashboard_query,
        metrics::dashboard::dashboard_live,
        metrics::scalar,
        metrics::grouped,
        metrics::distribution,
        metrics::catalog,
        metrics::tag_values,
        dashboards::search,
        dashboards::recent,
        dashboards::get_one,
        dashboards::create,
        dashboards::update,
        dashboards::clone,
        dashboards::delete,
        dashboards::restore,
        dashboards::lists,
        dashboards::create_list,
        dashboards::add_to_list,
        dashboards::monitor_summary,
        events::search,
        network::flows,
        network::dns,
        profiles::search,
        profiles::overview,
        profiles::flamegraph,
        profiles::insights,
        profiles::set_insight_state,
        hosts::search,
        hosts::detail,
        hosts::metrics,
        hosts::processes,
        hosts::pods,
        hosts::containers,
        kubernetes::search,
        kubernetes::detail,
        identity::me,
        identity::update_me,
        identity::search_users,
        identity::invite,
        identity::update_user,
        identity::roles,
        identity::search_teams,
        identity::create_team,
        identity::mappings,
        identity::create_mapping,
        identity::delete_mapping,
        incidents::search,
        incidents::get_one,
        incidents::declare,
        incidents::update,
        incidents::timeline,
        incidents::add_note,
        incidents::add_impact,
        incidents::add_followup,
        incidents::update_followup,
        incident_views::set_important,
        incident_views::responders,
        incident_views::add_responder,
        incident_views::remove_responder,
        incident_views::pages,
        incident_views::create_page,
        incident_views::update_page,
        incident_views::notifications,
        incident_views::create_notification,
        incident_views::documents,
        incident_views::add_document,
        incident_views::remove_document,
        incident_views::postmortem,
        incident_views::generate_postmortem,
        incident_views::update_postmortem,
        identity::login_link,
        identity::audit_trail,
        auth::config,
        auth::login,
        auth::mfa_verify,
        auth::logout,
        auth::link_info,
        auth::accept_link,
        auth::logout_all,
        auth::change_password,
        auth::sessions,
        auth::revoke_session,
        auth::mfa_setup,
        auth::mfa_enable,
        auth::mfa_disable,
        auth::my_orgs,
        auth::switch_org,
        notifications::list,
        notifications::create,
        notifications::delete,
        notifications::test,
        notifications::deliveries,
        ingest_keys::list,
        archives::list,
        archives::test,
        archives::create,
        archives::update,
        archives::delete,
        archives::runs,
        archives::run_now,
        archives::create_search,
        archives::list_searches,
        archives::get_search_detail,
        archives::delete_search,
        archives::create_rehydration,
        archives::list_rehydrations,
        archives::expire_rehydration,
        ingest_keys::create,
        ingest_keys::revoke,
        instance::list_orgs,
        instance::create_org,
        instance::update_org,
        instance::delete_org,
        instance::update_limits,
        log_indexes::list,
        log_indexes::create,
        log_indexes::update,
        log_indexes::delete,
        log_indexes::reorder,
        log_indexes::get_settings,
        log_indexes::update_settings,
        instance::audit,
        auth::app_keys,
        auth::create_app_key,
        auth::revoke_app_key,
        auth::security,
        sso::discover_sso,
        sso::saml_metadata,
        sso::saml_login,
        sso::saml_acs,
        sso::oidc_login,
        sso::oidc_callback,
        sso::get_settings,
        sso::update_settings,
        monitors::search,
        monitors::triggered,
        monitors::get_one,
        monitors::create,
        monitors::update,
        monitors::clone,
        monitors::bulk,
        monitors::history,
        monitors::preview,
        monitors::quality,
        downtimes::search,
        downtimes::create,
        downtimes::cancel,
        ci::executions,
        ci::facets,
        ci::charts,
        ci::pipeline,
        ci::aggregations,
        ci::overview,
        ci::flaky,
        ci::set_flaky_state,
        ci::dora,
        ci::repositories,
        spans::search,
        spans::facets,
        spans::analytics,
        spans::summary,
        spans::percentile,
    ),
    // Sent inside the SSE stream rather than as a response body, so it must be listed explicitly.
    components(schemas(live::LiveTailBatch)),
    tags(
        (name = "logs", description = "Log search, facets, histogram and live tail"),
        (name = "apm", description = "Services, endpoints, dependencies and traces computed from spans"),
        (name = "metrics", description = "Metric queries for dashboards (OTel metrics and span-derived trace.* metrics)"),
        (name = "dashboards", description = "Dashboards, dashboard lists and the monitor summary widget"),
        (name = "infrastructure", description = "Hosts, built from host metrics, spans and logs"),
        (name = "events", description = "Events from the Datadog Agent and the events API"),
        (name = "network", description = "Network flows from the Datadog Agent's system-probe"),
        (name = "profiles", description = "Continuous profiles from Datadog's profilers: explorer, flame graphs and insights"),
        (name = "orchestration", description = "Kubernetes and ECS Explorers: objects, containers, utilization and events sent by the collectors"),
        (name = "identity", description = "Profile, users, roles, teams and SAML group mappings"),
        (name = "incidents", description = "Incident management"),
        (name = "ci", description = "Software Delivery: CI pipeline executions, flaky tests and DORA metrics from CI/CD traces"),
        (name = "monitors", description = "Monitors, their evaluation history and downtimes"),
        (name = "auth", description = "Sign-in (password, MFA, SAML, OIDC), sessions, application keys and login methods"),
        (name = "health")
    )
)]
pub struct ApiDoc;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route(
            "/api/openapi.json",
            get(|| async { axum::Json(ApiDoc::openapi()) }),
        )
        .route("/api/v1/logs/search", post(logs::search))
        .route("/api/v1/logs/histogram", post(logs::histogram))
        .route("/api/v1/logs/facets", post(logs::facets))
        .route("/api/v1/logs/top", post(logs::top))
        .route("/api/v1/logs/patterns", post(logs::patterns))
        .route("/api/v1/logs/suggest", post(logs::suggest))
        .route("/api/v1/logs/insights", post(logs::insights))
        .route("/api/v1/logs/live", get(live::live))
        .route("/api/v1/apm/services", post(apm::services))
        .route("/api/v1/apm/resources", post(apm::resources))
        .route("/api/v1/apm/service", post(apm::service))
        .route("/api/v1/apm/deployments", post(apm::deployments))
        .route("/api/v1/apm/dependencies", post(apm::dependencies))
        .route("/api/v1/apm/databases", post(apm::databases))
        .route("/api/v1/apm/errors", post(apm::errors))
        .route("/api/v1/apm/traces/search", post(apm::search_traces))
        .route("/api/v1/apm/traces/:trace_id", get(apm::trace))
        .route("/api/v1/apm/recommendations", post(apm::recommendations))
        .route("/api/v1/apm/anomalies", post(apm::anomalies))
        .route("/api/v1/apm/envs", post(apm::envs))
        .route("/api/v1/query/timeseries", post(metrics::timeseries))
        .route(
            "/api/v1/query/dashboard",
            post(metrics::dashboard::dashboard_query),
        )
        .route(
            "/api/v1/query/dashboard/live",
            post(metrics::dashboard::dashboard_live),
        )
        .route("/api/v1/query/scalar", post(metrics::scalar))
        .route("/api/v1/query/grouped", post(metrics::grouped))
        .route("/api/v1/query/distribution", post(metrics::distribution))
        .route("/api/v1/metrics", get(metrics::catalog))
        .route("/api/v1/tags/:key/values", get(metrics::tag_values))
        .route("/api/v1/dashboards", post(dashboards::create))
        .route("/api/v1/dashboards/search", post(dashboards::search))
        .route("/api/v1/dashboards/recent", get(dashboards::recent))
        .route("/api/v1/dashboards/delete", post(dashboards::delete))
        .route(
            "/api/v1/dashboards/:id",
            get(dashboards::get_one).patch(dashboards::update),
        )
        .route("/api/v1/dashboards/:id/clone", post(dashboards::clone))
        .route("/api/v1/dashboards/:id/restore", post(dashboards::restore))
        .route(
            "/api/v1/dashboard-lists",
            get(dashboards::lists).post(dashboards::create_list),
        )
        .route(
            "/api/v1/dashboard-lists/:id/items",
            post(dashboards::add_to_list),
        )
        .route(
            "/api/v1/monitors/summary",
            post(dashboards::monitor_summary),
        )
        .route("/api/v1/monitors", post(monitors::create))
        .route("/api/v1/monitors/search", post(monitors::search))
        .route("/api/v1/monitors/triggered", post(monitors::triggered))
        .route("/api/v1/monitors/bulk", post(monitors::bulk))
        .route("/api/v1/monitors/preview", post(monitors::preview))
        .route("/api/v1/monitors/quality", post(monitors::quality))
        .route(
            "/api/v1/monitors/:id",
            get(monitors::get_one).put(monitors::update),
        )
        .route("/api/v1/monitors/:id/clone", post(monitors::clone))
        .route("/api/v1/monitors/:id/history", post(monitors::history))
        .route("/api/v1/downtimes", post(downtimes::create))
        .route("/api/v1/downtimes/search", post(downtimes::search))
        .route("/api/v1/downtimes/:id", delete(downtimes::cancel))
        .route("/api/v1/apm/spans/search", post(spans::search))
        .route("/api/v1/apm/spans/facets", post(spans::facets))
        .route("/api/v1/apm/spans/analytics", post(spans::analytics))
        .route("/api/v1/apm/spans/summary", post(spans::summary))
        .route("/api/v1/apm/spans/percentile", post(spans::percentile))
        .route("/api/v1/ci/executions", post(ci::executions))
        .route("/api/v1/ci/executions/facets", post(ci::facets))
        .route("/api/v1/ci/executions/charts", post(ci::charts))
        .route("/api/v1/ci/pipelines/:trace_id", get(ci::pipeline))
        .route("/api/v1/ci/aggregations", post(ci::aggregations))
        .route("/api/v1/ci/overview", post(ci::overview))
        .route("/api/v1/ci/flaky", post(ci::flaky))
        .route(
            "/api/v1/ci/flaky/state",
            axum::routing::put(ci::set_flaky_state),
        )
        .route("/api/v1/ci/dora", post(ci::dora))
        .route("/api/v1/ci/repositories", get(ci::repositories))
        .route("/api/v1/events/search", post(events::search))
        .route("/api/v1/network/flows", post(network::flows))
        .route("/api/v1/network/dns", post(network::dns))
        .route("/api/v1/profiles/search", post(profiles::search))
        .route("/api/v1/profiles/flamegraph", post(profiles::flamegraph))
        .route("/api/v1/profiles/overview", post(profiles::overview))
        .route("/api/v1/profiles/insights", post(profiles::insights))
        .route(
            "/api/v1/profiles/insights/state",
            post(profiles::set_insight_state),
        )
        .route("/api/v1/hosts/search", post(hosts::search))
        .route("/api/v1/hosts/:name", post(hosts::detail))
        .route("/api/v1/hosts/:name/metrics", post(hosts::metrics))
        .route("/api/v1/hosts/:name/processes", post(hosts::processes))
        .route("/api/v1/hosts/:name/pods", post(hosts::pods))
        .route("/api/v1/hosts/:name/containers", post(hosts::containers))
        .route("/api/v1/orchestration/search", post(kubernetes::search))
        .route(
            "/api/v1/orchestration/resources/*uid",
            get(kubernetes::detail),
        )
        .route("/api/v1/me", get(identity::me).patch(identity::update_me))
        .route("/api/v1/users/search", post(identity::search_users))
        .route("/api/v1/users/invite", post(identity::invite))
        .route("/api/v1/users/:id", patch(identity::update_user))
        .route("/api/v1/roles", get(identity::roles))
        .route("/api/v1/teams", post(identity::create_team))
        .route("/api/v1/teams/search", post(identity::search_teams))
        .route(
            "/api/v1/saml/mappings",
            get(identity::mappings).post(identity::create_mapping),
        )
        .route(
            "/api/v1/saml/mappings/:id",
            delete(identity::delete_mapping),
        )
        .route("/api/v1/users/:id/login-link", post(identity::login_link))
        .route("/api/v1/audit", get(identity::audit_trail))
        .route("/api/v1/auth/config", get(auth::config))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/mfa/verify", post(auth::mfa_verify))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route(
            "/api/v1/auth/link/:token",
            get(auth::link_info).post(auth::accept_link),
        )
        .route("/api/v1/auth/sso/discover", post(sso::discover_sso))
        .route("/api/v1/auth/saml/:org/metadata", get(sso::saml_metadata))
        .route("/api/v1/auth/saml/:org/login", get(sso::saml_login))
        .route("/api/v1/auth/saml/:org/acs", post(sso::saml_acs))
        .route("/api/v1/auth/oidc/:org/login", get(sso::oidc_login))
        .route("/api/v1/auth/oidc/:org/callback", get(sso::oidc_callback))
        .route("/api/v1/me/logout-all", post(auth::logout_all))
        .route("/api/v1/me/password", post(auth::change_password))
        .route("/api/v1/me/security", get(auth::security))
        .route("/api/v1/me/sessions", get(auth::sessions))
        .route("/api/v1/me/sessions/:id", delete(auth::revoke_session))
        .route("/api/v1/me/mfa/setup", post(auth::mfa_setup))
        .route("/api/v1/me/mfa/enable", post(auth::mfa_enable))
        .route("/api/v1/me/mfa/disable", post(auth::mfa_disable))
        .route("/api/v1/me/orgs", get(auth::my_orgs))
        .route("/api/v1/me/switch-org", post(auth::switch_org))
        .route(
            "/api/v1/instance/orgs",
            get(instance::list_orgs).post(instance::create_org),
        )
        .route(
            "/api/v1/instance/orgs/:id",
            patch(instance::update_org).delete(instance::delete_org),
        )
        .route("/api/v1/instance/audit", get(instance::audit))
        .route(
            "/api/v1/instance/orgs/:id/limits",
            patch(instance::update_limits),
        )
        .route(
            "/api/v1/logs/indexes",
            get(log_indexes::list).post(log_indexes::create),
        )
        .route("/api/v1/logs/indexes/reorder", post(log_indexes::reorder))
        .route(
            "/api/v1/logs/indexes/:id",
            axum::routing::put(log_indexes::update).delete(log_indexes::delete),
        )
        .route(
            "/api/v1/org/data-settings",
            get(log_indexes::get_settings).patch(log_indexes::update_settings),
        )
        .route(
            "/api/v1/ingest-keys",
            get(ingest_keys::list).post(ingest_keys::create),
        )
        .route("/api/v1/ingest-keys/:id", delete(ingest_keys::revoke))
        .route(
            "/api/v1/notification-channels",
            get(notifications::list).post(notifications::create),
        )
        .route(
            "/api/v1/notification-channels/:id",
            delete(notifications::delete),
        )
        .route(
            "/api/v1/notification-deliveries",
            get(notifications::deliveries),
        )
        .route(
            "/api/v1/monitors/test-notification",
            post(notifications::test),
        )
        .route(
            "/api/v1/logs/archives",
            get(archives::list).post(archives::create),
        )
        .route("/api/v1/logs/archives/test", post(archives::test))
        .route(
            "/api/v1/logs/archives/:id",
            patch(archives::update).delete(archives::delete),
        )
        .route("/api/v1/logs/archives/:id/runs", get(archives::runs))
        .route("/api/v1/logs/archives/:id/run", post(archives::run_now))
        .route(
            "/api/v1/logs/archive-searches",
            get(archives::list_searches).post(archives::create_search),
        )
        .route(
            "/api/v1/logs/archive-searches/:id",
            get(archives::get_search_detail).delete(archives::delete_search),
        )
        .route(
            "/api/v1/logs/rehydrations",
            get(archives::list_rehydrations).post(archives::create_rehydration),
        )
        .route(
            "/api/v1/logs/rehydrations/:id",
            delete(archives::expire_rehydration),
        )
        .route(
            "/api/v1/me/app-keys",
            get(auth::app_keys).post(auth::create_app_key),
        )
        .route("/api/v1/me/app-keys/:id", delete(auth::revoke_app_key))
        .route(
            "/api/v1/org/login-methods",
            get(sso::get_settings).put(sso::update_settings),
        )
        .route("/api/v1/incidents", post(incidents::declare))
        .route("/api/v1/incidents/search", post(incidents::search))
        .route(
            "/api/v1/incidents/:id",
            get(incidents::get_one).patch(incidents::update),
        )
        .route(
            "/api/v1/incidents/:id/timeline",
            get(incidents::timeline).post(incidents::add_note),
        )
        .route("/api/v1/incidents/:id/impacts", post(incidents::add_impact))
        .route(
            "/api/v1/incidents/:id/followups",
            post(incidents::add_followup),
        )
        .route(
            "/api/v1/incidents/:id/followups/:followup_id",
            patch(incidents::update_followup),
        )
        .route(
            "/api/v1/incidents/:id/timeline/:event_id",
            patch(incident_views::set_important),
        )
        .route(
            "/api/v1/incidents/:id/responders",
            get(incident_views::responders).post(incident_views::add_responder),
        )
        .route(
            "/api/v1/incidents/:id/responders/:user_id",
            delete(incident_views::remove_responder),
        )
        .route(
            "/api/v1/incidents/:id/pages",
            get(incident_views::pages).post(incident_views::create_page),
        )
        .route(
            "/api/v1/incidents/:id/pages/:page_id",
            patch(incident_views::update_page),
        )
        .route(
            "/api/v1/incidents/:id/notifications",
            get(incident_views::notifications).post(incident_views::create_notification),
        )
        .route(
            "/api/v1/incidents/:id/documents",
            get(incident_views::documents).post(incident_views::add_document),
        )
        .route(
            "/api/v1/incidents/:id/documents/:doc_id",
            delete(incident_views::remove_document),
        )
        .route(
            "/api/v1/incidents/:id/postmortem",
            get(incident_views::postmortem)
                .post(incident_views::generate_postmortem)
                .put(incident_views::update_postmortem),
        )
}

#[cfg(test)]
mod tests {
    use super::ApiDoc;
    use utoipa::OpenApi;

    /// openapi-typescript keys operations by id, so two handlers with the same function name in
    /// different modules would silently merge their types on the web side.
    #[test]
    fn operation_ids_are_unique() {
        let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
        let mut seen = std::collections::HashSet::new();
        for item in doc["paths"].as_object().unwrap().values() {
            for op in item.as_object().unwrap().values() {
                if let Some(id) = op["operationId"].as_str() {
                    assert!(
                        seen.insert(id.to_string()),
                        "duplicate operationId {id}; set operation_id on one handler"
                    );
                }
            }
        }
    }

    /// utoipa keys schemas by type name, so two `pub struct Foo` in different route modules would
    /// silently replace each other in the spec.
    #[test]
    fn schema_type_names_are_unique() {
        let sources = [
            include_str!("apm.rs"),
            include_str!("archives.rs"),
            include_str!("auth.rs"),
            include_str!("ci.rs"),
            include_str!("sso.rs"),
            include_str!("spans.rs"),
            include_str!("dashboards.rs"),
            include_str!("downtimes.rs"),
            include_str!("monitor_engine.rs"),
            include_str!("monitors.rs"),
            include_str!("notifications.rs"),
            include_str!("health.rs"),
            include_str!("hosts.rs"),
            include_str!("events.rs"),
            include_str!("network.rs"),
            include_str!("profiles.rs"),
            include_str!("identity.rs"),
            include_str!("incidents.rs"),
            include_str!("incident_views.rs"),
            include_str!("ingest_keys.rs"),
            include_str!("instance.rs"),
            include_str!("kubernetes.rs"),
            include_str!("live.rs"),
            include_str!("log_indexes.rs"),
            include_str!("logs.rs"),
            include_str!("metrics.rs"),
            include_str!("metrics/dashboard.rs"),
        ];
        let mut seen = std::collections::HashSet::new();
        for src in sources {
            for line in src.lines() {
                let Some(rest) = line
                    .strip_prefix("pub struct ")
                    .or_else(|| line.strip_prefix("pub enum "))
                else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                assert!(
                    seen.insert(name.clone()),
                    "type name {name} is defined in two route modules"
                );
            }
        }
    }

    /// Tenant isolation: every query that names a telemetry table must filter by organization
    /// (`OrgId = {oc_org:String}` or `org_id = ?`). Queries on a table chosen at run time get their
    /// WHERE clause from a helper that adds the org (e.g. `logs::scope`).
    #[test]
    fn telemetry_queries_filter_by_org() {
        let sources = [
            ("apm.rs", include_str!("apm.rs")),
            ("archives.rs", include_str!("archives.rs")),
            ("ci.rs", include_str!("ci.rs")),
            ("dashboards.rs", include_str!("dashboards.rs")),
            ("hosts.rs", include_str!("hosts.rs")),
            ("events.rs", include_str!("events.rs")),
            ("network.rs", include_str!("network.rs")),
            ("profiles.rs", include_str!("profiles.rs")),
            ("kubernetes.rs", include_str!("kubernetes.rs")),
            ("live.rs", include_str!("live.rs")),
            ("log_indexes.rs", include_str!("log_indexes.rs")),
            ("logs.rs", include_str!("logs.rs")),
            ("metrics.rs", include_str!("metrics.rs")),
            ("metrics/dashboard.rs", include_str!("metrics/dashboard.rs")),
            ("monitor_engine.rs", include_str!("monitor_engine.rs")),
            ("monitors.rs", include_str!("monitors.rs")),
            ("spans.rs", include_str!("spans.rs")),
            ("../logproc.rs", include_str!("../logproc.rs")),
        ];
        let mut missing = Vec::new();
        for (file, src) in sources {
            // Every string literal (queries span lines with `\` continuations).
            let mut rest = src;
            while let Some(start) = rest.find('"') {
                let body = &rest[start + 1..];
                let mut end = 0;
                let bytes = body.as_bytes();
                while end < bytes.len() && bytes[end] != b'"' {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                let lit = &body[..end.min(body.len())];
                let names_table = ["FROM otel.", "FROM oc.logs", "JOIN otel.", "JOIN oc.logs"]
                    .iter()
                    .any(|t| lit.contains(t));
                if names_table && !lit.contains("OrgId") && !lit.contains("org_id") {
                    missing.push(format!(
                        "{file}: {}",
                        lit.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(120)
                            .collect::<String>()
                    ));
                }
                rest = &body[(end + 1).min(body.len())..];
            }
        }
        assert!(
            missing.is_empty(),
            "queries without an org filter:\n{}",
            missing.join("\n")
        );
    }

    /// Read queries behind the UI go through `AppState::ch(org)`, which bounds their time and
    /// memory and counts them against the org's quota; the bare client has no limits.
    #[test]
    fn ui_queries_use_the_bounded_client() {
        let sources = [("logs.rs", include_str!("logs.rs"))];
        let unbounded: Vec<String> = sources
            .iter()
            .flat_map(|(file, src)| {
                src.lines()
                    .enumerate()
                    .filter(|(_, l)| l.contains("st.clickhouse."))
                    .map(move |(i, l)| format!("{file}:{}: {}", i + 1, l.trim()))
            })
            .collect();
        assert!(
            unbounded.is_empty(),
            "queries on the unbounded ClickHouse client (use st.ch(&org)):\n{}",
            unbounded.join("\n")
        );
    }
}
