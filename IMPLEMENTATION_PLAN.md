# Implementation plan: Datadog-compatible management API

Design: [docs/adr/0003-datadog-compatible-management-api.md](docs/adr/0003-datadog-compatible-management-api.md).
The previous plan (ADR-0002, storage at SaaS scale) is complete; see git history. Open items from it: marking OTLP
points as known series in the intake, span error groups precomputed, the live-dashboard cache key, the chart's
`cache-password` with `secrets.existingSecret`.

Every stage: `cargo test` green (with live ClickHouse/Postgres/Valkey where tests need them), contract fixtures from
`datadog-api-client-go` replayed for the resources it touches, and `make tf-compat` (OpenTofu + the `DataDog/datadog`
provider against a local server: apply → plan shows no changes → update → destroy) passing for its resources.

## Stage 1: Harness and authentication

Goal: the compat module, the test harness and key handling.

Success criteria:
- `routes/datadog_api/` with Datadog's error shape and auth: `DD-APPLICATION-KEY` / `Authorization: Bearer` with an
  `ocak_` key act as its user; `DD-API-KEY` alone only passes `/api/v1/validate`.
- `GET /api/v1/validate` and `GET /api/v2/validate_keys` on the server, as the provider calls them.
- Vendored cassettes (subset, with LICENSE/NOTICE) and a replay test helper.
- `make tf-compat`: OpenTofu + `DataDog/datadog` pinned, local server, a per-resource fixture directory, no-diff
  check after apply.

Status: Complete. `routes/datadog_api/` (errors, `DdAuth`, `validate.rs`, `current_user.rs`, the `replay.rs` test
helper), compat routes merged outside the session middleware (`routes::app`); fixtures from `datadog-api-client-go`
in `server/crates/oc-api/tests/datadog_api/` (Wave 1 features and cassettes; authentication replayed);
`scripts/tf-compat/run.sh` (OpenTofu 1.13.2, provider 4.27.0) with the `provider` fixture, which needs
`GET /api/v2/current_user` (`datadog_current_user`) because OpenTofu configures a provider only when a resource or
data source uses it; CI job `tf-compat`; docs in `docs/datadog-api.md`.

## Scope

Only the endpoints Datadog's Terraform provider, Pulumi provider and API clients call for the resources below get
Datadog's paths and documents. The UI and our own API stay as they are; compat routes translate onto the same tables,
so a resource created through Terraform shows in the UI and the other way round.

## Stage 2: Monitors and downtimes

Goal: `datadog_monitor`, `datadog_downtime_schedule`, `datadog_downtime`.

Success criteria:
- `/api/v1/monitor` (create, get, update, delete, list, search, `/validate`, `can_delete`), stored Datadog document
  returned on read; mapping onto our monitor types and options; unsupported types rejected with a clear message;
  accepted-but-not-enforced options listed.
- `/api/v2/downtime` and `/api/v1/downtime` onto our downtimes.
- Cassettes for the supported scenarios pass; tf-compat for both resources passes with no diff.

Status: Not Started

## Stage 3: Users, roles, teams and keys

Goal: `datadog_user`, `datadog_role` (+ permissions data source), `datadog_team`, `datadog_team_membership`,
`datadog_team_link` where meaningful, `datadog_api_key`, `datadog_application_key`, and the data sources the
provider uses with them (`datadog_user`, `datadog_role`, `datadog_permissions`, `datadog_team`).

Success criteria:
- `/api/v2/users` (+ invitations as the provider sends them), `/api/v2/roles` (+ `/permissions`, role users),
  `/api/v2/team` (+ memberships), `/api/v2/api_keys`, `/api/v2/application_keys` / current-user keys, onto our
  users, roles, teams, ingest keys and application keys; our permission names mapped to Datadog's where they exist.
- Tenancy and roles enforced exactly as in our API (only admins manage users/roles/keys).
- Cassettes pass; tf-compat passes with no diff.

Status: Not Started

## Stage 4: Dashboards and dashboard lists

Goal: `datadog_dashboard`, `datadog_dashboard_json`, `datadog_dashboard_list`.

Success criteria:
- `/api/v1/dashboard` CRUD and list; definitions stored verbatim and rendered through the integration-dashboard path;
  unsupported widget types rejected with the widget path.
- `/api/v1/dashboard/lists/manual` and `/api/v2/dashboard/lists/manual/{id}/dashboards`.
- Cassettes pass; tf-compat with a multi-widget dashboard (timeseries, query value, toplist, group, note, template
  variables) passes with no diff.

Status: Complete. `routes/datadog_api/dashboards.rs` (`/api/v1/dashboard` CRUD, list with `filter[deleted]`,
`filter[shared]`, `count`/`start`, bulk delete and restore, and `POST /api/unstable/dashboard/widgets/validate`,
which the provider calls during `plan`), `dashboard_widgets.rs` (which widgets we render, Datadog → our document as
the integration-dashboard importer does, and our widgets back to Datadog's), `dashboard_lists.rs` (v1 lists, v2
items). Migration `0050_datadog_dashboards.sql`: `dashboards.datadog_document` (the document as written; UI edits
are folded back in on read) and integer ids, author and timestamps for `dashboard_lists`. Cassettes: 42 dashboard
scenarios replayed as recorded, 15 about the dashboard itself replayed with their fixture widget (a
`profile_metrics_query` timeseries or a `hostmap`) replaced, 62 refused with the widget's path as expected; all 14
dashboard list scenarios. tf-compat fixtures `dashboard`, `dashboard_json`, `dashboard_list` (each with an update).

## Stage 5: Service level objectives

Goal: `datadog_service_level_objective`, `datadog_slo_correction`.

Success criteria:
- `/api/v1/slo` (metric and monitor SLOs, thresholds, timeframes), `/api/v1/slo/correction`.
- Cassettes pass; tf-compat passes with no diff.

Status: Complete. `routes/datadog_api/slos.rs` (create, get, update, delete, list with the data sources' filters,
search) and `slo_corrections.rs` (CRUD, list, an SLO's corrections) onto the `slos` and `slo_corrections` tables;
documents stored as received in `datadog_slo_documents` / `datadog_slo_correction_documents` (Postgres 0051) and
reconciled with UI edits on read. Metric (query or `sli_specification.count`), monitor and time-slice SLOs; `custom`
timeframes, recurring (`rrule`) and query-based (`slo_query`) corrections rejected with 400. Cassettes: 14 SLO
scenarios replayed (history left out), 5 correction scenarios replayed, the rrule and slo_query ones checked to be
rejected. tf-compat fixtures `service_level_objective` (metric 7d+30d, count, monitor, time-slice SLOs and both data
sources; the monitor comes from a fixture `setup.sh` through our API, `validate = false` until `datadog_monitor`)
and `slo_correction`, both with update overlays.

## Stage 6: Synthetics, logs configuration, metric metadata, span metrics, notification integrations

Status: Not Started
