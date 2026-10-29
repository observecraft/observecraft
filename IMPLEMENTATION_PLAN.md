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

Status: Not Started

## Stage 5: Service level objectives

Goal: `datadog_service_level_objective`, `datadog_slo_correction`.

Success criteria:
- `/api/v1/slo` (metric and monitor SLOs, thresholds, timeframes), `/api/v1/slo/correction`.
- Cassettes pass; tf-compat passes with no diff.

Status: Not Started

## Stage 6: Synthetics, logs configuration, metric metadata, span metrics, notification integrations

Status: Not Started
