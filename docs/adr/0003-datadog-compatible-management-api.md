# ADR-0003: Datadog-compatible management API

**Status:** Proposed
**Date:** 2026-10-29
**Deciders:** maintainers

## Context

The intake already speaks Datadog's agent and API formats for telemetry, so users keep Datadog's agents and libraries
and only change the URL and key. Configuration is different: ObserveCraft has its own REST API (`/api/v1/monitors`,
`/api/v1/dashboards`, …, documented with OpenAPI) and application keys (`ocak_…`, `Authorization: Bearer`). Teams
that manage Datadog as code can't point their tools at us:

- the Terraform provider `DataDog/datadog` (MPL-2.0) and Pulumi's Datadog provider;
- `datadog-api-client-*` SDKs (Apache-2.0) used in scripts;
- `datadog-sync-cli`, used to copy configuration between orgs, which would be the natural migration path.

All of them let the base URL be changed (`api_url`, `DD_SITE` / server variables). What they need is Datadog's management
API: its paths (`/api/v1/monitor`, `/api/v1/dashboard`, `/api/v2/downtime`, …), JSON documents and authentication.

How the provider authenticates (provider source, `datadog/provider.go`):

- `api_key` + `app_key` → `DD-API-KEY` and `DD-APPLICATION-KEY` headers; startup validation calls `GET /api/v1/validate`.
- `bearer_token` → `Authorization: Bearer <token>`; startup validation calls `GET /api/v2/validate_keys` with the token
  in `DD-APPLICATION-KEY`.
- `validate = false` skips both.

Constraints (CLAUDE.md): Datadog's API clients and OpenAPI spec are Apache-2.0 and may be used; Datadog's documentation
text may not be copied; tenancy comes only from keys/sessions; OpenTelemetry stays native.

## Decision

Add a **Datadog-compatible management API** to `observecraft-server`, resource by resource, translated onto our
model, so that Datadog's Terraform provider, Pulumi provider, SDKs and `datadog-sync-cli` work against ObserveCraft by
changing only the URL and keys.

1. **Same host, Datadog paths.** The compatibility routes live next to our API on the server (`/api/v1/monitor`,
   `/api/v1/dashboard`, `/api/v1/slo`, `/api/v2/downtime`, `/api/v1/validate`, `/api/v2/validate_keys`, …). Our own
   paths stay unchanged (they are mostly plural, so they don't collide). A route module per resource under
   `routes/datadog_api/`.
2. **Authentication.**
   - `DD-APPLICATION-KEY: ocak_…` (with or without `DD-API-KEY`) and `Authorization: Bearer ocak_…` act as that
     application key's user, with its roles. `DD-API-KEY` alone (an ingest key) only passes `/api/v1/validate`,
     as in Datadog where the API key alone can't change configuration.
   - The org always comes from the key (CLAUDE.md tenancy rule).
3. **Read back what was written.** Terraform compares what it sent with what it reads; any field we drop or rewrite
   shows up as a diff on every `plan`. Each compatible resource stores the Datadog document as received (JSONB in
   Postgres, next to our row), and reads return it with the server-owned fields filled in (`id`, `created`,
   `modified`, `creator`, `overall_state`, …). Our own model is derived from the document on write.
4. **Honest about what we don't do.** Fields that map onto our model drive behaviour. Fields we accept but don't act
   on (e.g. a monitor option we don't evaluate yet) are stored and returned so plans stay clean, and are listed per
   resource in the docs and flagged in the UI ("set through the API, not enforced"). Documents we can't represent at all
   (an unknown monitor `type`, a widget type we can't render) are rejected with Datadog's error shape (`{"errors":
   [...]}`) and a precise message, never silently changed.
5. **Errors, pagination and IDs as Datadog's clients expect**: `{"errors": ["…"]}` with the same status codes, the
   paging parameters each endpoint uses, integer monitor and SLO-correction ids, string dashboard ids
   (`abc-def-ghi`), UUIDs for v2 resources.
6. **Contract fixtures from Datadog's own client.** `datadog-api-client-go` (Apache-2.0) records real request/response
   pairs per feature (`tests/scenarios/cassettes/TestScenarios/v1/Feature_Monitors`, …) and describes the scenarios in
   `tests/scenarios/features`. We vendor the subset we support under `server/crates/oc-api/tests/datadog_api/` with its
   LICENSE and NOTICE, and replay them against our endpoints (status, required fields, shapes).
7. **End-to-end with the real provider.** An opt-in test (`make tf-compat`) runs OpenTofu (MPL-2.0; Terraform itself
   is BSL since 1.6) with the `DataDog/datadog` provider against a local server: `apply` a set of resources, `plan`
   must show no changes, `apply` an update, `destroy`. It runs in CI when the compatibility code changes.

### Resources, in order

| Wave | Resources (Terraform name → endpoints) |
|---|---|
| 1 | auth (`/api/v1/validate`, `/api/v2/validate_keys`); `datadog_monitor` (`/api/v1/monitor`, `/validate`, search); `datadog_downtime_schedule` (`/api/v2/downtime`) and `datadog_downtime` (`/api/v1/downtime`); `datadog_dashboard` / `datadog_dashboard_json` (`/api/v1/dashboard`); `datadog_dashboard_list` (`/api/v2/dashboard/lists/manual`, `/api/v1/dashboard/lists/manual`); `datadog_service_level_objective` (`/api/v1/slo`), `datadog_slo_correction` |
| 2 | `datadog_synthetics_test` (API tests) and `datadog_synthetics_global_variable`; `datadog_logs_custom_pipeline`, `datadog_logs_pipeline_order`, `datadog_logs_index`, `datadog_logs_index_order`, `datadog_logs_metric`; `datadog_metric_metadata`; `datadog_spans_metric` |
| 3 | `datadog_user`, `datadog_role`, `datadog_team` (+ memberships), `datadog_api_key`, `datadog_application_key`; `datadog_webhook` and Slack/PagerDuty integrations as our notification channels; `datadog_on_call_*`; `datadog_status_page*`; `datadog_reference_table`; `datadog_software_catalog` / service definitions |

Dashboards are the largest piece: widget definitions are numerous. We already render Datadog widget definitions for
integration dashboards (`scripts/import-integration-dashboards.mjs` and the `integration` widget type), so the compat
endpoint stores the Datadog definition and renders it through the same path; widget types we can't render yet are
rejected with the widget path in the error.

## Options considered

### A: Datadog-compatible management API (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | High overall, but incremental per resource |
| Cost | A translation layer per resource; contract fixtures keep it honest |
| Scalability | Same server, same tenancy and limits |
| Team familiarity | Medium (we already translate Datadog formats at the intake and for integration dashboards) |

**Pros:** existing Terraform/Pulumi code, SDK scripts and `datadog-sync-cli` work unchanged; migration from Datadog
becomes "change the URL"; consistent with the intake's compatibility.
**Cons:** large surface; Datadog adds fields over time; some options won't be enforced at first.

### B: Our own Terraform provider

Generated from our OpenAPI. Simpler mapping, but users rewrite their Terraform code, and Pulumi/SDKs/sync tools still
don't work. Not needed if A exists.

### C: File/CRD provisioning (GitOps)

Useful for ArgoCD-only teams and cheap to add later, reading the same Datadog documents from a ConfigMap or Git. Not a
substitute for A.

## Consequences

- **Easier:** migrations from Datadog; managing ObserveCraft as code with tools teams already use.
- **Harder:** every change to our monitor/dashboard model must keep the compat translation and fixtures green;
  documents stored verbatim must be kept in sync when the same resource is edited in the UI (the UI edit updates the
  stored document too, so Terraform shows the drift as Datadog would).
- **Revisit:** whether to expose the compat routes on a separate hostname for clarity; personal access tokens
  (`ddpat_`-style) once we have them.

## Action items

See `IMPLEMENTATION_PLAN.md`.
