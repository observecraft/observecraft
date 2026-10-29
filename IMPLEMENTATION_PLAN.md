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

Status: Not Started

## Stage 2: Monitors and downtimes

Goal: `datadog_monitor`, `datadog_downtime_schedule`, `datadog_downtime`.

Success criteria:
- `/api/v1/monitor` (create, get, update, delete, list, search, `/validate`, `can_delete`), stored Datadog document
  returned on read; mapping onto our monitor types and options; unsupported types rejected with a clear message;
  accepted-but-not-enforced options listed.
- `/api/v2/downtime` and `/api/v1/downtime` onto our downtimes.
- Cassettes for the supported scenarios pass; tf-compat for both resources passes with no diff.

Status: Not Started

## Stage 3: Dashboards and dashboard lists

Goal: `datadog_dashboard`, `datadog_dashboard_json`, `datadog_dashboard_list`.

Success criteria:
- `/api/v1/dashboard` CRUD and list; definitions stored verbatim and rendered through the integration-dashboard path;
  unsupported widget types rejected with the widget path.
- `/api/v1/dashboard/lists/manual` and `/api/v2/dashboard/lists/manual/{id}/dashboards`.
- Cassettes pass; tf-compat with a multi-widget dashboard (timeseries, query value, toplist, group, note, template
  variables) passes with no diff.

Status: Not Started

## Stage 4: Service level objectives

Goal: `datadog_service_level_objective`, `datadog_slo_correction`.

Success criteria:
- `/api/v1/slo` (metric and monitor SLOs, thresholds, timeframes), `/api/v1/slo/correction`.
- Cassettes pass; tf-compat passes with no diff.

Status: Not Started

## Stage 5: Wave 2 (synthetics, logs configuration, metric metadata, span metrics)

Status: Not Started

## Stage 6: Wave 3 (users, roles, teams, keys, notification integrations, on-call, status pages, reference tables, catalog)

Status: Not Started
