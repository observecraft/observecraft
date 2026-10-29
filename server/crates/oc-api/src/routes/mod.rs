pub mod apm;
pub mod apps;
pub mod archives;
pub mod auth;
pub mod catalog;
pub mod changes;
pub mod ci;
pub mod cloud_map;
pub mod dashboard_reports;
pub mod dashboards;
pub mod derived_metrics;
pub mod deployment_gates;
pub mod data_jobs;
pub mod data_streams;
pub mod databases;
pub mod downtimes;
pub mod error_tracking;
pub mod event_correlation;
pub mod event_settings;
pub mod events;
pub mod feature_flags;
pub mod fleet;
pub mod gpu;
pub mod serverless;
pub mod sql_editor;
pub mod health;
pub mod hosts;
pub mod identity;
pub mod incident_views;
pub mod incidents;
pub mod ingest_keys;
pub mod instance;
pub mod kubernetes;
pub mod live;
pub mod llm;
pub mod llm_attrs;
pub mod llm_checks;
pub mod log_indexes;
pub mod log_metrics;
pub mod log_pipelines;
pub mod log_restrictions;
pub mod log_aggregates;
pub mod logs;
pub mod metric_inventory;
pub mod metric_rules;
pub mod metrics;
pub mod monitor_engine;
pub mod monitor_formula;
pub mod monitors;
pub mod kafka_console;
pub mod network;
pub mod notebooks;
pub mod notifications;
pub mod on_call;
pub mod on_call_escalation;
pub mod on_call_schedule;
pub mod portal_home;
pub mod pr_gates;
pub mod portal_sources;
pub mod portal_action_steps;
pub mod portal_actions;
pub mod portal_surveys;
pub mod portal_catalog;
pub mod portal_model;
pub mod portal_reports;
pub mod workflow_cron;
pub mod workflow_engine;
pub mod workflow_expr;
pub mod workflows;
pub mod profiles;
pub mod reference_tables;
pub mod resource_policies;
pub mod resource_policy_rules;
pub mod scorecard_rules;
pub mod scorecards;
pub mod sheets;
pub mod service_checks;
pub mod spans;
pub mod standard_attributes;
pub mod slo_math;
pub mod slos;
pub mod status_pages;
pub mod sso;
pub mod synthetics;
pub mod watchdog;
pub mod synthetics_exec;
pub mod synthetics_net;
pub mod synthetics_settings;
pub mod synthetics_suites;
pub mod synthetics_explorer;

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
        error_tracking::search_issues,
        error_tracking::set_issue_state,
        error_tracking::issue_detail,
        error_tracking::issue_samples,
        error_tracking::issue_facets,
        logs::patterns,
        logs::fields,
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
        metrics::volume,
        metrics::tag_values,
        derived_metrics::list,
        derived_metrics::create,
        derived_metrics::update,
        derived_metrics::delete,
        deployment_gates::list,
        deployment_gates::get,
        deployment_gates::create,
        deployment_gates::update,
        deployment_gates::delete,
        deployment_gates::evaluate,
        deployment_gates::evaluations,
        deployment_gates::evaluation,
        pr_gates::list,
        pr_gates::create,
        pr_gates::update,
        pr_gates::delete,
        pr_gates::evaluate,
        metric_rules::list,
        metric_rules::create,
        metric_rules::update,
        metric_rules::delete,
        metric_rules::overview,
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
        notebooks::search,
        notebooks::get_one,
        notebooks::create,
        notebooks::update,
        notebooks::clone,
        notebooks::delete,
        notebooks::restore,
        notebooks::set_favorite,
        dashboard_reports::list,
        dashboard_reports::options,
        dashboard_reports::get_one,
        dashboard_reports::create,
        dashboard_reports::update,
        dashboard_reports::delete,
        dashboard_reports::subscribe,
        dashboard_reports::preview,
        dashboard_reports::send_now,
        sheets::search,
        sheets::get_one,
        sheets::create,
        sheets::update,
        sheets::delete,
        events::search,
        events::fields,
        events::work_items,
        event_settings::list_correlators,
        event_settings::create_correlator,
        event_settings::update_correlator,
        event_settings::delete_correlator,
        event_settings::list_windows,
        event_settings::create_window,
        event_settings::delete_window,
        event_settings::list_attributes,
        event_settings::create_attribute,
        event_settings::update_attribute,
        event_settings::delete_attribute,
        event_settings::set_value,
        service_checks::list,
        service_checks::detail,
        watchdog::alerts,
        network::flows,
        network::dns,
        profiles::search,
        profiles::overview,
        profiles::flamegraph,
        profiles::insights,
        profiles::set_insight_state,
        fleet::agents,
        fleet::agent,
        fleet::configurations,
        hosts::search,
        hosts::detail,
        hosts::metrics,
        hosts::processes,
        hosts::pods,
        hosts::containers,
        catalog::search,
        changes::search,
        resource_policies::search,
        resource_policies::attributes,
        resource_policies::preview,
        resource_policies::get,
        resource_policies::create,
        resource_policies::update,
        resource_policies::delete,
        cloud_map::cloud_map,
        gpu::search,
        gpu::timeseries,
        serverless::search,
        serverless::timeseries,
        serverless::invocations,
        sql_editor::schema,
        sql_editor::templates,
        sql_editor::run,
        sql_editor::list_queries,
        sql_editor::create_query,
        sql_editor::update_query,
        sql_editor::delete_query,
        databases::search,
        databases::instance,
        data_jobs::search,
        data_streams::map,
        data_streams::measure,
        data_streams::node,
        kafka_console::console,
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
        on_call::list_schedules,
        on_call::get_schedule,
        on_call::create_schedule,
        on_call::update_schedule,
        on_call::delete_schedule,
        on_call::schedule_shifts,
        on_call::preview_schedule,
        on_call::create_override,
        on_call::delete_override,
        on_call::my_shifts,
        on_call::list_policies,
        on_call::get_policy,
        on_call::create_policy,
        on_call::update_policy,
        on_call::delete_policy,
        on_call::list_teams,
        on_call::set_routing,
        on_call::search_pages,
        on_call::get_page,
        on_call::create_page,
        on_call::acknowledge_page,
        on_call::resolve_page,
        on_call::reassign_page,
        on_call::get_profile,
        on_call::update_profile,
        on_call::test_profile,
        on_call::analytics,
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
        log_pipelines::list,
        log_pipelines::create,
        log_pipelines::update,
        log_pipelines::set_enabled,
        log_pipelines::delete,
        log_pipelines::reorder,
        log_pipelines::test,
        log_pipelines::grok,
        log_pipelines::library_list,
        log_pipelines::install,
        log_metrics::list,
        log_metrics::create,
        log_metrics::update,
        log_metrics::delete,
        log_restrictions::list,
        log_restrictions::create,
        log_restrictions::update,
        log_restrictions::delete,
        log_restrictions::assign_role,
        log_restrictions::unassign_role,
        log_restrictions::assign_team,
        log_restrictions::unassign_team,
        log_restrictions::preview,
        standard_attributes::list,
        standard_attributes::create,
        standard_attributes::update,
        standard_attributes::delete,
        standard_attributes::reset,
        reference_tables::list,
        reference_tables::create,
        reference_tables::get_one,
        reference_tables::replace,
        reference_tables::delete,
        status_pages::list,
        status_pages::create,
        status_pages::get_one,
        status_pages::update,
        status_pages::delete,
        status_pages::create_notice,
        status_pages::update_notice,
        status_pages::delete_notice,
        status_pages::get_public,
        synthetics::locations,
        synthetics::search,
        synthetics::get_one,
        synthetics::create,
        synthetics::update,
        synthetics::delete,
        synthetics::run_now,
        synthetics::try_request,
        synthetics::results,
        synthetics::result,
        synthetics_suites::search_suites,
        synthetics_suites::get_suite,
        synthetics_suites::create_suite,
        synthetics_suites::update_suite,
        synthetics_suites::delete_suite,
        synthetics_explorer::search_results,
        synthetics_explorer::coverage,
        synthetics_settings::get_settings,
        synthetics_settings::update_settings,
        synthetics_settings::list_variables,
        synthetics_settings::create_variable,
        synthetics_settings::update_variable,
        synthetics_settings::delete_variable,
        scorecards::get_overview,
        scorecards::setup,
        scorecards::evaluate_now,
        scorecards::create_rule,
        scorecards::update_rule,
        scorecards::delete_rule,
        scorecards::set_outcomes,
        scorecards::update_metadata,
        feature_flags::list,
        feature_flags::get_one,
        feature_flags::create,
        feature_flags::update,
        feature_flags::delete,
        feature_flags::update_environment,
        feature_flags::history,
        feature_flags::flag_history,
        feature_flags::list_environments,
        feature_flags::create_environment,
        feature_flags::update_environment_settings,
        feature_flags::delete_environment,
        portal_home::get_connectors,
        portal_home::put_connector,
        portal_home::delete_connector,
        portal_home::test_connector,
        portal_home::get_identities,
        portal_home::put_identities,
        portal_home::pull_requests,
        portal_home::tickets,
        portal_home::entities,
        portal_home::add_favorite,
        portal_home::remove_favorite,
        portal_home::get_apps,
        portal_home::create_app,
        portal_home::update_app,
        portal_home::delete_app,
        portal_home::get_sources,
        portal_surveys::list_surveys,
        portal_surveys::get_survey,
        portal_surveys::create_survey,
        portal_surveys::update_survey,
        portal_surveys::delete_survey,
        portal_surveys::survey_form,
        portal_surveys::submit_response,
        portal_surveys::survey_results,
        portal_surveys::list_responses,
        portal_actions::list_actions,
        portal_actions::get_action,
        portal_actions::create_action,
        portal_actions::update_action,
        portal_actions::delete_action,
        portal_actions::run_action,
        portal_actions::list_runs,
        portal_catalog::catalog,
        portal_catalog::list_entities,
        portal_catalog::get_entity,
        portal_catalog::upsert_entity,
        portal_catalog::update_entity,
        portal_catalog::delete_entity,
        portal_catalog::import_entities,
        portal_reports::teams,
        portal_reports::scorecards,
        portal_reports::list_dashboards,
        portal_reports::set_dashboards,
        workflows::list_workflows,
        workflows::get_workflow,
        workflows::create_workflow,
        workflows::update_workflow,
        workflows::delete_workflow,
        workflows::publish_workflow,
        workflows::unpublish_workflow,
        workflows::create_token,
        workflows::run_workflow,
        workflows::trigger_workflow,
        workflows::list_runs,
        workflows::get_run,
        workflows::cancel_run,
        apps::list_apps,
        apps::get_app,
        apps::create_app,
        apps::update_app,
        apps::delete_app,
        apps::run_app,
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
        slos::search,
        slos::get_one,
        slos::create,
        slos::update,
        slos::delete,
        slos::list_corrections,
        slos::create_correction,
        slos::update_correction,
        slos::delete_correction,
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
        llm::overview,
        llm::facets,
        llm::search_traces,
        llm::trace,
        llm::evaluations,
    ),
    // Sent inside the SSE stream rather than as a response body, so it must be listed explicitly
    // (and the kind of the recursive WorkflowStep, which `no_recursion` leaves out).
    components(schemas(live::LiveTailBatch, workflow_engine::WorkflowStepKind)),
    tags(
        (name = "logs", description = "Log search, facets, histogram and live tail"),
        (name = "apm", description = "Services, endpoints, dependencies and traces computed from spans"),
        (name = "metrics", description = "Metric queries for dashboards (OTel metrics and span-derived trace.* metrics)"),
        (name = "dashboards", description = "Dashboards, dashboard lists and the monitor summary widget"),
        (name = "notebooks", description = "Notebooks: documents of Markdown and live query cells"),
        (name = "infrastructure", description = "Hosts, built from host metrics, spans and logs"),
        (name = "fleet", description = "Fleet Automation: the Datadog Agents and OpenTelemetry Collectors sending data, their versions, checks and reported configuration (read-only)"),
        (name = "databases", description = "Database Monitoring and Data Jobs, built from database, Spark and Airflow integration metrics"),
        (name = "events", description = "Events from the Datadog Agent and the events API"),
        (name = "data-streams", description = "Data Streams Monitoring: messaging pathways from spans and the Kafka console"),
        (name = "network", description = "Network flows from the Datadog Agent's system-probe"),
        (name = "profiles", description = "Continuous profiles from Datadog's profilers: explorer, flame graphs and insights"),
        (name = "orchestration", description = "Kubernetes and ECS Explorers: objects, containers, utilization and events sent by the collectors"),
        (name = "identity", description = "Profile, users, roles, teams and SAML group mappings"),
        (name = "incidents", description = "Incident management and status pages"),
        (name = "llm", description = "LLM Observability: LLM spans (OpenTelemetry GenAI, OpenLLMetry, dd-trace LLM Observability tags), estimated cost and built-in evaluations"),
        (name = "ci", description = "Software Delivery: CI pipeline executions, flaky tests and DORA metrics from CI/CD traces"),
        (name = "monitors", description = "Monitors, their evaluation history and downtimes"),
        (name = "slos", description = "Service level objectives, their status and error budget, and corrections"),
        (name = "on-call", description = "On-Call: schedules, escalation policies, team routing, pages and notification preferences"),
        (name = "synthetics", description = "Synthetic API tests (HTTP, SSL, DNS, TCP) run by the server, their results and uptime"),
        (name = "scorecards", description = "Developer Portal scorecards: rules evaluated per service, their levels and service metadata"),
        (name = "feature_flags", description = "Feature flags, their targeting per environment and history; SDKs evaluate them over OFREP at the intake"),
        (name = "portal", description = "Developer Portal: home (connectors, identities, pull requests, tickets, entities, apps), the Software Catalog (telemetry and entity definitions v3), reports, settings, surveys and self-service actions"),
        (name = "automation", description = "Workflow Automation (workflows, triggers, runs) and App Builder"),
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
        .route("/api/v1/error-tracking/issues/search", post(error_tracking::search_issues))
        .route("/api/v1/error-tracking/issues/:id/state", post(error_tracking::set_issue_state))
        .route("/api/v1/error-tracking/issues/facets", post(error_tracking::issue_facets))
        .route("/api/v1/error-tracking/issues/:id", post(error_tracking::issue_detail))
        .route("/api/v1/error-tracking/issues/:id/samples", post(error_tracking::issue_samples))
        .route("/api/v1/logs/patterns", post(logs::patterns))
        .route("/api/v1/logs/fields", post(logs::fields))
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
        .route("/api/v1/metrics/volume", post(metrics::volume))
        .route("/api/v1/metrics/derived", get(derived_metrics::list).post(derived_metrics::create))
        .route("/api/v1/metrics/derived/:id", axum::routing::put(derived_metrics::update).delete(derived_metrics::delete))
        .route("/api/v1/deployment-gates", get(deployment_gates::list).post(deployment_gates::create))
        .route("/api/v1/deployment-gates/evaluate", post(deployment_gates::evaluate))
        .route("/api/v1/deployment-gates/evaluations", get(deployment_gates::evaluations))
        .route("/api/v1/deployment-gates/evaluations/:id", get(deployment_gates::evaluation))
        .route("/api/v1/deployment-gates/:id", get(deployment_gates::get).put(deployment_gates::update).delete(deployment_gates::delete))
        .route("/api/v1/pr-gates/rules", get(pr_gates::list).post(pr_gates::create))
        .route("/api/v1/pr-gates/rules/:id", axum::routing::put(pr_gates::update).delete(pr_gates::delete))
        .route("/api/v1/pr-gates/evaluate", post(pr_gates::evaluate))
        .route("/api/v1/metrics/rules", get(metric_rules::list).post(metric_rules::create))
        .route("/api/v1/metrics/rules/overview", post(metric_rules::overview))
        .route("/api/v1/metrics/rules/:id", axum::routing::put(metric_rules::update).delete(metric_rules::delete))
        .route("/api/v1/tags/:key/values", get(metrics::tag_values))
        .route("/api/v1/dashboards", post(dashboards::create))
        .route("/api/v1/dashboards/search", post(dashboards::search))
        .route("/api/v1/notebooks", post(notebooks::create))
        .route("/api/v1/notebooks/search", post(notebooks::search))
        .route(
            "/api/v1/notebooks/:id",
            get(notebooks::get_one).patch(notebooks::update).delete(notebooks::delete),
        )
        .route("/api/v1/notebooks/:id/clone", post(notebooks::clone))
        .route("/api/v1/notebooks/:id/restore", post(notebooks::restore))
        .route("/api/v1/notebooks/:id/favorite", axum::routing::put(notebooks::set_favorite))
        .route("/api/v1/dashboard-reports", get(dashboard_reports::list).post(dashboard_reports::create))
        .route("/api/v1/dashboard-reports/options", get(dashboard_reports::options))
        .route(
            "/api/v1/dashboard-reports/:id",
            get(dashboard_reports::get_one).put(dashboard_reports::update).delete(dashboard_reports::delete),
        )
        .route("/api/v1/dashboard-reports/:id/subscription", axum::routing::put(dashboard_reports::subscribe))
        .route("/api/v1/dashboard-reports/:id/preview", get(dashboard_reports::preview))
        .route("/api/v1/dashboard-reports/:id/send", post(dashboard_reports::send_now))
        .route("/api/v1/spreadsheets", post(sheets::create))
        .route("/api/v1/spreadsheets/search", post(sheets::search))
        .route("/api/v1/spreadsheets/:id", get(sheets::get_one).patch(sheets::update).delete(sheets::delete))
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
        .route("/api/v1/slos", post(slos::create))
        .route("/api/v1/slos/search", post(slos::search))
        .route(
            "/api/v1/slos/:id",
            get(slos::get_one).put(slos::update).delete(slos::delete),
        )
        .route(
            "/api/v1/slo-corrections",
            get(slos::list_corrections).post(slos::create_correction),
        )
        .route(
            "/api/v1/slo-corrections/:id",
            axum::routing::put(slos::update_correction).delete(slos::delete_correction),
        )
        .route("/api/v1/apm/spans/search", post(spans::search))
        .route("/api/v1/apm/spans/facets", post(spans::facets))
        .route("/api/v1/apm/spans/analytics", post(spans::analytics))
        .route("/api/v1/apm/spans/summary", post(spans::summary))
        .route("/api/v1/apm/spans/percentile", post(spans::percentile))
        .route("/api/v1/llm/overview", post(llm::overview))
        .route("/api/v1/llm/facets", post(llm::facets))
        .route("/api/v1/llm/traces/search", post(llm::search_traces))
        .route("/api/v1/llm/traces/:trace_id", get(llm::trace))
        .route("/api/v1/llm/evaluations", post(llm::evaluations))
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
        .route("/api/v1/events/fields", post(events::fields))
        .route("/api/v1/events/work-items", post(events::work_items))
        .route("/api/v1/events/work-items/:id/attributes", axum::routing::put(event_settings::set_value))
        .route("/api/v1/events/correlators", get(event_settings::list_correlators).post(event_settings::create_correlator))
        .route("/api/v1/events/correlators/:id", axum::routing::put(event_settings::update_correlator).delete(event_settings::delete_correlator))
        .route("/api/v1/events/maintenance-windows", get(event_settings::list_windows).post(event_settings::create_window))
        .route("/api/v1/events/maintenance-windows/:id", delete(event_settings::delete_window))
        .route("/api/v1/events/work-item-attributes", get(event_settings::list_attributes).post(event_settings::create_attribute))
        .route("/api/v1/events/work-item-attributes/:id", axum::routing::put(event_settings::update_attribute).delete(event_settings::delete_attribute))
        .route("/api/v1/service-checks", get(service_checks::list))
        .route("/api/v1/service-checks/detail", post(service_checks::detail))
        .route("/api/v1/watchdog/alerts", get(watchdog::alerts))
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
        .route("/api/v1/fleet/agents", post(fleet::agents))
        .route("/api/v1/fleet/agent", post(fleet::agent))
        .route("/api/v1/fleet/configurations", post(fleet::configurations))
        .route("/api/v1/hosts/search", post(hosts::search))
        .route("/api/v1/hosts/:name", post(hosts::detail))
        .route("/api/v1/hosts/:name/metrics", post(hosts::metrics))
        .route("/api/v1/hosts/:name/processes", post(hosts::processes))
        .route("/api/v1/hosts/:name/pods", post(hosts::pods))
        .route("/api/v1/hosts/:name/containers", post(hosts::containers))
        .route("/api/v1/infrastructure/catalog", post(catalog::search))
        .route("/api/v1/infrastructure/changes", post(changes::search))
        .route("/api/v1/infrastructure/cloud-map", post(cloud_map::cloud_map))
        .route("/api/v1/infrastructure/policies", post(resource_policies::create))
        .route("/api/v1/infrastructure/policies/search", post(resource_policies::search))
        .route("/api/v1/infrastructure/policies/attributes", get(resource_policies::attributes))
        .route("/api/v1/infrastructure/policies/preview", post(resource_policies::preview))
        .route(
            "/api/v1/infrastructure/policies/:id",
            get(resource_policies::get).put(resource_policies::update).delete(resource_policies::delete),
        )
        .route("/api/v1/gpu/search", post(gpu::search))
        .route("/api/v1/gpu/timeseries", post(gpu::timeseries))
        .route("/api/v1/serverless/functions", post(serverless::search))
        .route("/api/v1/serverless/timeseries", post(serverless::timeseries))
        .route("/api/v1/serverless/invocations", post(serverless::invocations))
        .route("/api/v1/sql/schema", get(sql_editor::schema))
        .route("/api/v1/sql/templates", get(sql_editor::templates))
        .route("/api/v1/sql/run", post(sql_editor::run))
        .route("/api/v1/sql/queries", get(sql_editor::list_queries).post(sql_editor::create_query))
        .route("/api/v1/sql/queries/:id", axum::routing::put(sql_editor::update_query).delete(sql_editor::delete_query))
        .route("/api/v1/databases/search", post(databases::search))
        .route("/api/v1/databases/instance", post(databases::instance))
        .route("/api/v1/data-jobs/search", post(data_jobs::search))
        .route("/api/v1/data-streams/map", post(data_streams::map))
        .route("/api/v1/data-streams/measure", post(data_streams::measure))
        .route("/api/v1/data-streams/node", post(data_streams::node))
        .route("/api/v1/data-streams/kafka", post(kafka_console::console))
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
            "/api/v1/logs/pipelines",
            get(log_pipelines::list).post(log_pipelines::create),
        )
        .route("/api/v1/logs/pipelines/reorder", post(log_pipelines::reorder))
        .route("/api/v1/logs/pipelines/test", post(log_pipelines::test))
        .route("/api/v1/logs/pipelines/grok", post(log_pipelines::grok))
        .route(
            "/api/v1/logs/pipelines/:id",
            axum::routing::put(log_pipelines::update)
                .patch(log_pipelines::set_enabled)
                .delete(log_pipelines::delete),
        )
        .route(
            "/api/v1/logs/pipeline-library",
            get(log_pipelines::library_list),
        )
        .route(
            "/api/v1/logs/pipeline-library/:id/install",
            post(log_pipelines::install),
        )
        .route(
            "/api/v1/logs/metrics",
            get(log_metrics::list).post(log_metrics::create),
        )
        .route(
            "/api/v1/logs/metrics/:id",
            axum::routing::put(log_metrics::update).delete(log_metrics::delete),
        )
        .route(
            "/api/v1/logs/restriction-queries",
            get(log_restrictions::list).post(log_restrictions::create),
        )
        .route(
            "/api/v1/logs/restriction-queries/preview",
            post(log_restrictions::preview),
        )
        .route(
            "/api/v1/logs/restriction-queries/:id",
            axum::routing::put(log_restrictions::update).delete(log_restrictions::delete),
        )
        .route(
            "/api/v1/logs/restriction-queries/:id/roles",
            post(log_restrictions::assign_role),
        )
        .route(
            "/api/v1/logs/restriction-queries/:id/roles/:role_id",
            delete(log_restrictions::unassign_role),
        )
        .route(
            "/api/v1/logs/restriction-queries/:id/teams",
            post(log_restrictions::assign_team),
        )
        .route(
            "/api/v1/logs/restriction-queries/:id/teams/:handle",
            delete(log_restrictions::unassign_team),
        )
        .route(
            "/api/v1/logs/standard-attributes",
            get(standard_attributes::list)
                .post(standard_attributes::create)
                .delete(standard_attributes::reset),
        )
        .route(
            "/api/v1/logs/standard-attributes/:path",
            axum::routing::put(standard_attributes::update).delete(standard_attributes::delete),
        )
        .route(
            "/api/v1/reference-tables",
            get(reference_tables::list)
                .post(reference_tables::create)
                .layer(axum::extract::DefaultBodyLimit::max(reference_tables::MAX_BODY_BYTES)),
        )
        .route(
            "/api/v1/reference-tables/:name",
            get(reference_tables::get_one)
                .put(reference_tables::replace)
                .delete(reference_tables::delete)
                .layer(axum::extract::DefaultBodyLimit::max(reference_tables::MAX_BODY_BYTES)),
        )
        .route("/api/v1/status-pages", get(status_pages::list).post(status_pages::create))
        .route(
            "/api/v1/status-pages/:id",
            get(status_pages::get_one).put(status_pages::update).delete(status_pages::delete),
        )
        .route("/api/v1/status-pages/:id/notices", post(status_pages::create_notice))
        .route(
            "/api/v1/status-pages/:id/notices/:notice_id",
            axum::routing::delete(status_pages::delete_notice),
        )
        .route(
            "/api/v1/status-pages/:id/notices/:notice_id/updates",
            post(status_pages::update_notice),
        )
        .route("/api/v1/synthetics/locations", get(synthetics::locations))
        .route("/api/v1/synthetics/suites", post(synthetics_suites::create_suite))
        .route("/api/v1/synthetics/suites/search", post(synthetics_suites::search_suites))
        .route(
            "/api/v1/synthetics/suites/:id",
            get(synthetics_suites::get_suite).put(synthetics_suites::update_suite).delete(synthetics_suites::delete_suite),
        )
        .route("/api/v1/synthetics/results/search", post(synthetics_explorer::search_results))
        .route("/api/v1/synthetics/coverage", post(synthetics_explorer::coverage))
        .route("/api/v1/synthetics/settings", get(synthetics_settings::get_settings).put(synthetics_settings::update_settings))
        .route("/api/v1/synthetics/variables", get(synthetics_settings::list_variables).post(synthetics_settings::create_variable))
        .route(
            "/api/v1/synthetics/variables/:id",
            axum::routing::put(synthetics_settings::update_variable).delete(synthetics_settings::delete_variable),
        )
        .route("/api/v1/synthetics/tests", post(synthetics::create))
        .route("/api/v1/synthetics/tests/search", post(synthetics::search))
        .route(
            "/api/v1/synthetics/tests/:id",
            get(synthetics::get_one).put(synthetics::update).delete(synthetics::delete),
        )
        .route("/api/v1/synthetics/tests/:id/run", post(synthetics::run_now))
        .route("/api/v1/synthetics/tests/:id/results", post(synthetics::results))
        .route("/api/v1/synthetics/tests/:id/results/:result_id", get(synthetics::result))
        .route("/api/v1/synthetics/try", post(synthetics::try_request))
        .route("/api/v1/scorecards", get(scorecards::get_overview))
        .route("/api/v1/scorecards/setup", post(scorecards::setup))
        .route("/api/v1/scorecards/evaluate", post(scorecards::evaluate_now))
        .route("/api/v1/scorecards/rules", post(scorecards::create_rule))
        .route("/api/v1/scorecards/rules/:id", axum::routing::put(scorecards::update_rule).delete(scorecards::delete_rule))
        .route("/api/v1/scorecards/rules/:id/outcomes", post(scorecards::set_outcomes))
        .route("/api/v1/scorecards/services/:service/metadata", axum::routing::put(scorecards::update_metadata))
        .route("/api/v1/feature-flags", get(feature_flags::list).post(feature_flags::create))
        .route("/api/v1/feature-flags/history", get(feature_flags::history))
        .route("/api/v1/feature-flags/environments", get(feature_flags::list_environments).post(feature_flags::create_environment))
        .route(
            "/api/v1/feature-flags/environments/:id",
            axum::routing::put(feature_flags::update_environment_settings).delete(feature_flags::delete_environment),
        )
        .route(
            "/api/v1/feature-flags/:id",
            get(feature_flags::get_one).put(feature_flags::update).delete(feature_flags::delete),
        )
        .route("/api/v1/feature-flags/:id/history", get(feature_flags::flag_history))
        .route("/api/v1/feature-flags/:id/environments/:environment_id", axum::routing::put(feature_flags::update_environment))
        .route("/api/v1/portal/connectors", get(portal_home::get_connectors))
        .route("/api/v1/portal/connectors/:provider", axum::routing::put(portal_home::put_connector).delete(portal_home::delete_connector))
        .route("/api/v1/portal/connectors/:provider/test", post(portal_home::test_connector))
        .route("/api/v1/portal/identities", get(portal_home::get_identities).put(portal_home::put_identities))
        .route("/api/v1/portal/home/prs", get(portal_home::pull_requests))
        .route("/api/v1/portal/home/tickets", get(portal_home::tickets))
        .route("/api/v1/portal/home/entities", get(portal_home::entities))
        .route("/api/v1/portal/favorites/:entity", axum::routing::put(portal_home::add_favorite).delete(portal_home::remove_favorite))
        .route("/api/v1/portal/apps", get(portal_home::get_apps).post(portal_home::create_app))
        .route("/api/v1/portal/apps/:id", axum::routing::put(portal_home::update_app).delete(portal_home::delete_app))
        .route("/api/v1/portal/sources", get(portal_home::get_sources))
        .route("/api/v1/portal/surveys", get(portal_surveys::list_surveys).post(portal_surveys::create_survey))
        .route(
            "/api/v1/portal/surveys/:id",
            get(portal_surveys::get_survey).put(portal_surveys::update_survey).delete(portal_surveys::delete_survey),
        )
        .route("/api/v1/portal/surveys/:id/form", get(portal_surveys::survey_form))
        .route("/api/v1/portal/surveys/:id/responses", get(portal_surveys::list_responses).post(portal_surveys::submit_response))
        .route("/api/v1/portal/surveys/:id/results", get(portal_surveys::survey_results))
        .route("/api/v1/portal/actions", get(portal_actions::list_actions).post(portal_actions::create_action))
        .route(
            "/api/v1/portal/actions/:id",
            get(portal_actions::get_action).put(portal_actions::update_action).delete(portal_actions::delete_action),
        )
        .route("/api/v1/portal/actions/:id/runs", get(portal_actions::list_runs).post(portal_actions::run_action))
        .route("/api/v1/workflows", get(workflows::list_workflows).post(workflows::create_workflow))
        .route(
            "/api/v1/workflows/:id",
            get(workflows::get_workflow).put(workflows::update_workflow).delete(workflows::delete_workflow),
        )
        .route("/api/v1/workflows/:id/publish", post(workflows::publish_workflow))
        .route("/api/v1/workflows/:id/unpublish", post(workflows::unpublish_workflow))
        .route("/api/v1/workflows/:id/token", post(workflows::create_token))
        .route("/api/v1/workflows/:id/trigger", post(workflows::trigger_workflow))
        .route("/api/v1/workflows/:id/runs", get(workflows::list_runs).post(workflows::run_workflow))
        .route("/api/v1/workflows/:id/runs/:run", get(workflows::get_run))
        .route("/api/v1/workflows/:id/runs/:run/cancel", post(workflows::cancel_run))
        .route("/api/v1/apps", get(apps::list_apps).post(apps::create_app))
        .route("/api/v1/apps/:id", get(apps::get_app).put(apps::update_app).delete(apps::delete_app))
        .route("/api/v1/apps/:id/run", post(apps::run_app))
        .route("/api/v1/portal/catalog", post(portal_catalog::catalog))
        .route("/api/v1/portal/entities", get(portal_catalog::list_entities).post(portal_catalog::upsert_entity))
        .route("/api/v1/portal/entities/import", post(portal_catalog::import_entities))
        .route(
            "/api/v1/portal/entities/:id",
            get(portal_catalog::get_entity).put(portal_catalog::update_entity).delete(portal_catalog::delete_entity),
        )
        .route("/api/v1/portal/reports/teams", post(portal_reports::teams))
        .route("/api/v1/portal/reports/scorecards", post(portal_reports::scorecards))
        .route(
            "/api/v1/portal/reports/dashboards",
            get(portal_reports::list_dashboards).put(portal_reports::set_dashboards),
        )
        // Needs no sign-in (auth::is_public); answers only for public pages or members of the page's org.
        .route("/api/v1/public/status-pages/:slug", get(status_pages::get_public))
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
        .route("/api/v1/on-call/schedules", get(on_call::list_schedules).post(on_call::create_schedule))
        .route("/api/v1/on-call/schedules/preview", post(on_call::preview_schedule))
        .route(
            "/api/v1/on-call/schedules/:id",
            get(on_call::get_schedule).put(on_call::update_schedule).delete(on_call::delete_schedule),
        )
        .route("/api/v1/on-call/schedules/:id/shifts", post(on_call::schedule_shifts))
        .route("/api/v1/on-call/schedules/:id/overrides", post(on_call::create_override))
        .route("/api/v1/on-call/schedules/:id/overrides/:override_id", delete(on_call::delete_override))
        .route("/api/v1/on-call/my-shifts", post(on_call::my_shifts))
        .route("/api/v1/on-call/escalation-policies", get(on_call::list_policies).post(on_call::create_policy))
        .route(
            "/api/v1/on-call/escalation-policies/:id",
            get(on_call::get_policy).put(on_call::update_policy).delete(on_call::delete_policy),
        )
        .route("/api/v1/on-call/teams", get(on_call::list_teams))
        .route("/api/v1/on-call/teams/:handle/routing", axum::routing::put(on_call::set_routing))
        .route("/api/v1/on-call/pages", post(on_call::create_page))
        .route("/api/v1/on-call/pages/search", post(on_call::search_pages))
        .route("/api/v1/on-call/pages/:id", get(on_call::get_page))
        .route("/api/v1/on-call/pages/:id/acknowledge", post(on_call::acknowledge_page))
        .route("/api/v1/on-call/pages/:id/resolve", post(on_call::resolve_page))
        .route("/api/v1/on-call/pages/:id/reassign", post(on_call::reassign_page))
        .route("/api/v1/on-call/profile", get(on_call::get_profile).put(on_call::update_profile))
        .route("/api/v1/on-call/profile/test", post(on_call::test_profile))
        .route("/api/v1/on-call/analytics", post(on_call::analytics))
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
            include_str!("catalog.rs"),
            include_str!("changes.rs"),
            include_str!("ci.rs"),
            include_str!("sso.rs"),
            include_str!("spans.rs"),
            include_str!("dashboards.rs"),
            include_str!("dashboard_reports.rs"),
            include_str!("sheets.rs"),
            include_str!("downtimes.rs"),
            include_str!("slos.rs"),
            include_str!("slo_math.rs"),
            include_str!("monitor_engine.rs"),
            include_str!("monitors.rs"),
            include_str!("notifications.rs"),
            include_str!("on_call.rs"),
            include_str!("on_call_escalation.rs"),
            include_str!("on_call_schedule.rs"),
            include_str!("portal_action_steps.rs"),
            include_str!("portal_actions.rs"),
            include_str!("portal_surveys.rs"),
            include_str!("workflow_engine.rs"),
            include_str!("workflows.rs"),
            include_str!("apps.rs"),
            include_str!("health.rs"),
            include_str!("hosts.rs"),
            include_str!("gpu.rs"),
            include_str!("serverless.rs"),
            include_str!("sql_editor.rs"),
            include_str!("databases.rs"),
            include_str!("data_jobs.rs"),
            include_str!("data_streams.rs"),
            include_str!("kafka_console.rs"),
            include_str!("metric_inventory.rs"),
            include_str!("metric_rules.rs"),
            include_str!("derived_metrics.rs"),
            include_str!("deployment_gates.rs"),
            include_str!("pr_gates.rs"),
            include_str!("event_correlation.rs"),
            include_str!("event_settings.rs"),
            include_str!("service_checks.rs"),
            include_str!("watchdog.rs"),
            include_str!("events.rs"),
            include_str!("feature_flags.rs"),
            include_str!("fleet.rs"),
            include_str!("network.rs"),
            include_str!("notebooks.rs"),
            include_str!("profiles.rs"),
            include_str!("reference_tables.rs"),
            include_str!("resource_policies.rs"),
            include_str!("cloud_map.rs"),
            include_str!("resource_policy_rules.rs"),
            include_str!("status_pages.rs"),
            include_str!("synthetics.rs"),
            include_str!("synthetics_exec.rs"),
            include_str!("synthetics_net.rs"),
            include_str!("synthetics_suites.rs"),
            include_str!("synthetics_explorer.rs"),
            include_str!("synthetics_settings.rs"),
            include_str!("scorecards.rs"),
            include_str!("portal_home.rs"),
            include_str!("portal_sources.rs"),
            include_str!("scorecard_rules.rs"),
            include_str!("portal_catalog.rs"),
            include_str!("portal_model.rs"),
            include_str!("portal_reports.rs"),
            include_str!("identity.rs"),
            include_str!("incidents.rs"),
            include_str!("incident_views.rs"),
            include_str!("ingest_keys.rs"),
            include_str!("instance.rs"),
            include_str!("kubernetes.rs"),
            include_str!("live.rs"),
            include_str!("llm.rs"),
            include_str!("llm_attrs.rs"),
            include_str!("llm_checks.rs"),
            include_str!("log_indexes.rs"),
            include_str!("log_pipelines.rs"),
            include_str!("log_metrics.rs"),
            include_str!("log_restrictions.rs"),
            include_str!("standard_attributes.rs"),
            include_str!("../standard_attributes.rs"),
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
            ("catalog.rs", include_str!("catalog.rs")),
            ("changes.rs", include_str!("changes.rs")),
            ("cloud_map.rs", include_str!("cloud_map.rs")),
            ("ci.rs", include_str!("ci.rs")),
            ("deployment_gates.rs", include_str!("deployment_gates.rs")),
            ("pr_gates.rs", include_str!("pr_gates.rs")),
            ("dashboards.rs", include_str!("dashboards.rs")),
            ("dashboard_reports.rs", include_str!("dashboard_reports.rs")),
            ("sheets.rs", include_str!("sheets.rs")),
            ("error_tracking.rs", include_str!("error_tracking.rs")),
            ("hosts.rs", include_str!("hosts.rs")),
            ("gpu.rs", include_str!("gpu.rs")),
            ("serverless.rs", include_str!("serverless.rs")),
            ("sql_editor.rs", include_str!("sql_editor.rs")),
            ("databases.rs", include_str!("databases.rs")),
            ("data_jobs.rs", include_str!("data_jobs.rs")),
            ("data_streams.rs", include_str!("data_streams.rs")),
            ("kafka_console.rs", include_str!("kafka_console.rs")),
            ("metric_inventory.rs", include_str!("metric_inventory.rs")),
            ("metric_rules.rs", include_str!("metric_rules.rs")),
            ("service_checks.rs", include_str!("service_checks.rs")),
            ("watchdog.rs", include_str!("watchdog.rs")),
            ("events.rs", include_str!("events.rs")),
            ("feature_flags.rs", include_str!("feature_flags.rs")),
            ("fleet.rs", include_str!("fleet.rs")),
            ("network.rs", include_str!("network.rs")),
            ("profiles.rs", include_str!("profiles.rs")),
            ("reference_tables.rs", include_str!("reference_tables.rs")),
            ("status_pages.rs", include_str!("status_pages.rs")),
            ("synthetics.rs", include_str!("synthetics.rs")),
            ("synthetics_exec.rs", include_str!("synthetics_exec.rs")),
            ("synthetics_net.rs", include_str!("synthetics_net.rs")),
            ("synthetics_suites.rs", include_str!("synthetics_suites.rs")),
            ("synthetics_explorer.rs", include_str!("synthetics_explorer.rs")),
            ("synthetics_settings.rs", include_str!("synthetics_settings.rs")),
            ("scorecards.rs", include_str!("scorecards.rs")),
            ("portal_home.rs", include_str!("portal_home.rs")),
            ("portal_sources.rs", include_str!("portal_sources.rs")),
            ("portal_catalog.rs", include_str!("portal_catalog.rs")),
            ("portal_reports.rs", include_str!("portal_reports.rs")),
            ("kubernetes.rs", include_str!("kubernetes.rs")),
            ("live.rs", include_str!("live.rs")),
            ("llm.rs", include_str!("llm.rs")),
            ("llm_attrs.rs", include_str!("llm_attrs.rs")),
            ("log_indexes.rs", include_str!("log_indexes.rs")),
            ("log_pipelines.rs", include_str!("log_pipelines.rs")),
            ("log_metrics.rs", include_str!("log_metrics.rs")),
            ("log_aggregates.rs", include_str!("log_aggregates.rs")),
            ("logs.rs", include_str!("logs.rs")),
            ("metrics.rs", include_str!("metrics.rs")),
            ("metrics/dashboard.rs", include_str!("metrics/dashboard.rs")),
            ("monitor_engine.rs", include_str!("monitor_engine.rs")),
            ("monitors.rs", include_str!("monitors.rs")),
            ("slos.rs", include_str!("slos.rs")),
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
                let names_table = ["FROM otel.", "FROM oc.logs", "JOIN otel.", "JOIN oc.logs", "FROM oc.error_groups", "FROM oc.log_facets"]
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

    /// Every source file of the server, relative to `src/routes`.
    fn server_sources() -> Vec<(&'static str, &'static str)> {
        vec![
            ("apm.rs", include_str!("apm.rs")),
            ("archives.rs", include_str!("archives.rs")),
            ("auth.rs", include_str!("auth.rs")),
            ("catalog.rs", include_str!("catalog.rs")),
            ("changes.rs", include_str!("changes.rs")),
            ("cloud_map.rs", include_str!("cloud_map.rs")),
            ("ci.rs", include_str!("ci.rs")),
            ("deployment_gates.rs", include_str!("deployment_gates.rs")),
            ("pr_gates.rs", include_str!("pr_gates.rs")),
            ("dashboards.rs", include_str!("dashboards.rs")),
            ("dashboard_reports.rs", include_str!("dashboard_reports.rs")),
            ("sheets.rs", include_str!("sheets.rs")),
            ("downtimes.rs", include_str!("downtimes.rs")),
            ("error_tracking.rs", include_str!("error_tracking.rs")),
            ("events.rs", include_str!("events.rs")),
            ("event_correlation.rs", include_str!("event_correlation.rs")),
            ("event_settings.rs", include_str!("event_settings.rs")),
            ("derived_metrics.rs", include_str!("derived_metrics.rs")),
            ("feature_flags.rs", include_str!("feature_flags.rs")),
            ("fleet.rs", include_str!("fleet.rs")),
            ("health.rs", include_str!("health.rs")),
            ("hosts.rs", include_str!("hosts.rs")),
            ("gpu.rs", include_str!("gpu.rs")),
            ("serverless.rs", include_str!("serverless.rs")),
            ("sql_editor.rs", include_str!("sql_editor.rs")),
            ("databases.rs", include_str!("databases.rs")),
            ("data_jobs.rs", include_str!("data_jobs.rs")),
            ("data_streams.rs", include_str!("data_streams.rs")),
            ("kafka_console.rs", include_str!("kafka_console.rs")),
            ("metric_inventory.rs", include_str!("metric_inventory.rs")),
            ("metric_rules.rs", include_str!("metric_rules.rs")),
            ("service_checks.rs", include_str!("service_checks.rs")),
            ("watchdog.rs", include_str!("watchdog.rs")),
            ("identity.rs", include_str!("identity.rs")),
            ("incident_views.rs", include_str!("incident_views.rs")),
            ("incidents.rs", include_str!("incidents.rs")),
            ("ingest_keys.rs", include_str!("ingest_keys.rs")),
            ("instance.rs", include_str!("instance.rs")),
            ("kubernetes.rs", include_str!("kubernetes.rs")),
            ("live.rs", include_str!("live.rs")),
            ("llm.rs", include_str!("llm.rs")),
            ("llm_attrs.rs", include_str!("llm_attrs.rs")),
            ("llm_checks.rs", include_str!("llm_checks.rs")),
            ("log_indexes.rs", include_str!("log_indexes.rs")),
            ("log_metrics.rs", include_str!("log_metrics.rs")),
            ("log_aggregates.rs", include_str!("log_aggregates.rs")),
            ("log_pipelines.rs", include_str!("log_pipelines.rs")),
            ("log_restrictions.rs", include_str!("log_restrictions.rs")),
            ("logs.rs", include_str!("logs.rs")),
            ("metrics.rs", include_str!("metrics.rs")),
            ("metrics/dashboard.rs", include_str!("metrics/dashboard.rs")),
            ("mod.rs", include_str!("mod.rs")),
            ("monitor_engine.rs", include_str!("monitor_engine.rs")),
            ("monitor_formula.rs", include_str!("monitor_formula.rs")),
            ("monitors.rs", include_str!("monitors.rs")),
            ("network.rs", include_str!("network.rs")),
            ("notebooks.rs", include_str!("notebooks.rs")),
            ("notifications.rs", include_str!("notifications.rs")),
            ("on_call.rs", include_str!("on_call.rs")),
            ("on_call_escalation.rs", include_str!("on_call_escalation.rs")),
            ("on_call_schedule.rs", include_str!("on_call_schedule.rs")),
            ("portal_action_steps.rs", include_str!("portal_action_steps.rs")),
            ("portal_actions.rs", include_str!("portal_actions.rs")),
            ("portal_surveys.rs", include_str!("portal_surveys.rs")),
            ("portal_catalog.rs", include_str!("portal_catalog.rs")),
            ("portal_model.rs", include_str!("portal_model.rs")),
            ("portal_reports.rs", include_str!("portal_reports.rs")),
            ("workflow_cron.rs", include_str!("workflow_cron.rs")),
            ("workflow_engine.rs", include_str!("workflow_engine.rs")),
            ("workflow_expr.rs", include_str!("workflow_expr.rs")),
            ("workflows.rs", include_str!("workflows.rs")),
            ("apps.rs", include_str!("apps.rs")),
            ("profiles.rs", include_str!("profiles.rs")),
            ("reference_tables.rs", include_str!("reference_tables.rs")),
            ("resource_policies.rs", include_str!("resource_policies.rs")),
            ("resource_policy_rules.rs", include_str!("resource_policy_rules.rs")),
            ("scorecard_rules.rs", include_str!("scorecard_rules.rs")),
            ("scorecards.rs", include_str!("scorecards.rs")),
            ("portal_home.rs", include_str!("portal_home.rs")),
            ("portal_sources.rs", include_str!("portal_sources.rs")),
            ("slo_math.rs", include_str!("slo_math.rs")),
            ("slos.rs", include_str!("slos.rs")),
            ("status_pages.rs", include_str!("status_pages.rs")),
            ("synthetics.rs", include_str!("synthetics.rs")),
            ("synthetics_exec.rs", include_str!("synthetics_exec.rs")),
            ("synthetics_net.rs", include_str!("synthetics_net.rs")),
            ("synthetics_suites.rs", include_str!("synthetics_suites.rs")),
            ("synthetics_explorer.rs", include_str!("synthetics_explorer.rs")),
            ("synthetics_settings.rs", include_str!("synthetics_settings.rs")),
            ("spans.rs", include_str!("spans.rs")),
            ("sso.rs", include_str!("sso.rs")),
            ("standard_attributes.rs", include_str!("standard_attributes.rs")),
            ("../auth/crypto.rs", include_str!("../auth/crypto.rs")),
            ("../auth/mod.rs", include_str!("../auth/mod.rs")),
            ("../auth/password.rs", include_str!("../auth/password.rs")),
            ("../auth/totp.rs", include_str!("../auth/totp.rs")),
            ("../config.rs", include_str!("../config.rs")),
            ("../demo.rs", include_str!("../demo.rs")),
            ("../error.rs", include_str!("../error.rs")),
            ("../log_access.rs", include_str!("../log_access.rs")),
            ("../log_metrics.rs", include_str!("../log_metrics.rs")),
            ("../logproc.rs", include_str!("../logproc.rs")),
            ("../main.rs", include_str!("../main.rs")),
            ("../pipelines/dates.rs", include_str!("../pipelines/dates.rs")),
            ("../pipelines/filter.rs", include_str!("../pipelines/filter.rs")),
            ("../pipelines/grok.rs", include_str!("../pipelines/grok.rs")),
            ("../pipelines/library.rs", include_str!("../pipelines/library.rs")),
            ("../pipelines/log.rs", include_str!("../pipelines/log.rs")),
            ("../pipelines/mod.rs", include_str!("../pipelines/mod.rs")),
            ("../pipelines/processors.rs", include_str!("../pipelines/processors.rs")),
            ("../s3.rs", include_str!("../s3.rs")),
            ("../standard_attributes.rs", include_str!("../standard_attributes.rs")),
            ("../state.rs", include_str!("../state.rs")),
        ]
    }

    /// The code of a source file without its tests.
    fn without_tests(src: &str) -> &str {
        src.split("#[cfg(test)]").next().unwrap_or(src)
    }

    /// The string literals of a source file (queries span lines with `\` continuations).
    fn string_literals(src: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = src;
        while let Some(start) = rest.find('"') {
            let body = &rest[start + 1..];
            let bytes = body.as_bytes();
            let mut end = 0;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            out.push(&body[..end.min(body.len())]);
            rest = &body[(end + 1).min(body.len())..];
        }
        out
    }

    /// `a/routes/../b.rs` → `a/b.rs`.
    fn normalize(p: &std::path::Path) -> std::path::PathBuf {
        let mut out = std::path::PathBuf::new();
        for c in p.components() {
            match c {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                c => out.push(c),
            }
        }
        out
    }

    /// The source guards below see every file: a new one must be added to `server_sources`.
    #[test]
    fn source_guards_cover_every_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let listed: std::collections::HashSet<std::path::PathBuf> = server_sources()
            .iter()
            .map(|(f, _)| normalize(&root.join("routes").join(f)))
            .collect();
        let mut stack = vec![root];
        let mut missing = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !listed.contains(&normalize(&path))
                {
                    missing.push(path.display().to_string());
                }
            }
        }
        assert!(
            missing.is_empty(),
            "add these files to server_sources():\n{}",
            missing.join("\n")
        );
    }

    /// Log data access: a query that names a log table itself skips the reader's restriction
    /// queries, which `logs::scope` adds. Only the reads below may do that, and none of them serves
    /// log events to a user; every other read builds its table and WHERE clause with
    /// `logs::scope`, which always carries the restriction.
    #[test]
    fn log_reads_go_through_the_restricted_scope() {
        const ALLOWED: &[(&str, &str, &str)] = &[
            (
                "log_indexes.rs",
                "FROM logs WHERE org_id = ? AND timestamp >= now() - INTERVAL 25 HOUR GROUP BY index_name, hour",
                "per-index volumes on the index settings (counts)",
            ),
            (
                "log_indexes.rs",
                "SELECT count() FROM logs_raw WHERE org_id = ?",
                "today's ingested volume (a count)",
            ),
            (
                "archives.rs",
                "FROM logs_raw WHERE {clause}",
                "the archiver writing the org's logs to its archives (background)",
            ),
            (
                "instance.rs",
                "count() AS n FROM oc.logs WHERE timestamp > now() - INTERVAL 1 DAY GROUP BY org_id",
                "per-org volumes for instance admins (counts)",
            ),
            (
                "../logproc.rs",
                "FROM logs WHERE timestamp >= now() - INTERVAL 25 HOUR GROUP BY org_id, index_name, hour",
                "the indexer's daily quotas (background)",
            ),
        ];
        let mut direct = Vec::new();
        for (file, src) in server_sources() {
            for lit in string_literals(without_tests(src)) {
                let names_log_table = ["FROM logs", "FROM oc.logs", "JOIN logs", "JOIN oc.logs"]
                    .iter()
                    .any(|t| lit.contains(t));
                let allowed = ALLOWED
                    .iter()
                    .any(|(f, fragment, _)| *f == file && lit.contains(fragment));
                if names_log_table && !allowed {
                    let text = lit.split_whitespace().collect::<Vec<_>>().join(" ");
                    direct.push(format!("{file}: {}", text.chars().take(120).collect::<String>()));
                }
            }
        }
        assert!(
            direct.is_empty(),
            "log reads that skip the restriction queries (use logs::scope):\n{}",
            direct.join("\n")
        );
    }

    /// `LogAccess::org_background()` reads every log of the org: only the monitor engine's own
    /// evaluation (and backfill) and the scorecards' per-service log counts may use it, never to
    /// serve log events.
    #[test]
    fn unrestricted_log_access_is_only_for_background_jobs() {
        const ALLOWED: &[(&str, usize)] = &[("monitors.rs", 2), ("scorecards.rs", 1)];
        for (file, src) in server_sources() {
            if file == "../log_access.rs" {
                continue;
            }
            let uses = without_tests(src)
                .matches("LogAccess::org_background()")
                .count();
            let allowed = ALLOWED.iter().find(|(f, _)| *f == file).map_or(0, |(_, n)| *n);
            assert_eq!(
                uses, allowed,
                "{file} reads logs unrestricted (LogAccess::org_background()) {uses} times, {allowed} allowed"
            );
        }
    }
}
