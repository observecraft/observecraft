# Managing ObserveCraft with Datadog's Terraform provider and API clients

ObserveCraft's server also answers a subset of Datadog's management API, so configuration you manage as code with
Datadog's tools can point at ObserveCraft by changing only the URL and the keys:

- the Terraform provider [`DataDog/datadog`](https://registry.terraform.io/providers/DataDog/datadog) (with Terraform
  or OpenTofu);
- the `datadog-api-client-*` SDKs.

Support grows resource by resource ([ADR-0003](adr/0003-datadog-compatible-management-api.md)); the table below lists
what works today; other endpoints are not available yet.

## Keys

| Datadog | ObserveCraft | Where to create it |
|---|---|---|
| API key (`DD-API-KEY`, provider `api_key`) | an ingest key, `ocik_…` | Organization Settings → API Keys |
| Application key (`DD-APPLICATION-KEY`, provider `app_key`) | an application key, `ocak_…` | Personal Settings → Application keys |
| Bearer token (`Authorization: Bearer`, provider `bearer_token`) | the same application key, `ocak_…` | Personal Settings → Application keys |

Requests act as the user who owns the application key, with that user's role: a Read Only user's key can read but
not change configuration. The organization always comes from the key. When an API key is sent along with an
application key, both must belong to the same organization.

An API key on its own is only good for `GET /api/v1/validate`, as with Datadog. Keys are read from the headers only.

## Terraform provider

Point `api_url` at the server (the address of the web app and API, not the intake on `:4318`):

```hcl
terraform {
  required_providers {
    datadog = {
      source = "DataDog/datadog"
    }
  }
}

provider "datadog" {
  api_url = "https://observecraft.example.com"
  api_key = var.observecraft_ingest_key      # ocik_…
  app_key = var.observecraft_application_key # ocak_…
}
```

Or authenticate with the application key alone:

```hcl
provider "datadog" {
  api_url      = "https://observecraft.example.com"
  bearer_token = var.observecraft_application_key # ocak_…
}
```

`validate` (default `true`) makes the provider check its credentials when it starts: `api_key` + `app_key` call
`GET /api/v1/validate`, which checks the API key; `bearer_token` calls `GET /api/v2/validate_keys`, which checks the
application key. A wrong application key with a valid API key is therefore only reported by the first request that
uses it. `validate = false` skips the check.

The environment variables the provider reads (`DD_API_KEY`, `DD_APP_KEY`, `DD_BEARER_TOKEN`, `DD_HOST`) work the
same way.

## API clients

Datadog's SDKs select the server through their server variables (the custom-URL server, index 1) and take the keys
as usual. For example with `datadog-api-client-go`:

```go
ctx := context.WithValue(context.Background(), datadog.ContextServerIndex, 1)
ctx = context.WithValue(ctx, datadog.ContextServerVariables, map[string]string{"protocol": "https", "name": "observecraft.example.com"})
ctx = context.WithValue(ctx, datadog.ContextAPIKeys, map[string]datadog.APIKey{
	"apiKeyAuth": {Key: os.Getenv("DD_API_KEY")}, // ocik_…
	"appKeyAuth": {Key: os.Getenv("DD_APP_KEY")}, // ocak_…
})
```

Errors come back in Datadog's shape, `{"errors": ["…"]}`, with Datadog's status codes (400, 403, 404, 429, …).

## Supported resources

| Terraform resource / data source | Endpoints | Status |
|---|---|---|
| Provider authentication (`validate`) | `GET /api/v1/validate`, `GET /api/v2/validate_keys` | ✅ |
| `datadog_current_user` (data source) | `GET /api/v2/current_user` | ✅ |
| `datadog_monitor` | `/api/v1/monitor` | Planned |
| `datadog_downtime_schedule`, `datadog_downtime` | `/api/v2/downtime`, `/api/v1/downtime` | Planned |
| `datadog_dashboard`, `datadog_dashboard_json` | `/api/v1/dashboard` | Planned |
| `datadog_dashboard_list` | `/api/v1/dashboard/lists/manual`, `/api/v2/dashboard/lists/manual/{id}/dashboards` | Planned |
| `datadog_service_level_objective`, data sources `datadog_service_level_objective(s)` | `/api/v1/slo` (create, get, update, delete, list), `/api/v1/slo/search` | ✅ [details](#service-level-objectives) |
| `datadog_slo_correction` | `/api/v1/slo/correction`, `/api/v1/slo/{id}/corrections` | ✅ one-off corrections ([details](#slo-corrections)) |

## Service level objectives

SLOs created through `/api/v1/slo` are ObserveCraft SLOs (Monitoring → Service Level Objectives) and the other way
round. The document a client sends is kept and read back as sent, with the fields the server owns added (`id`,
`type_id`, `created_at`, `modified_at`, `creator`). An SLO's id is its ObserveCraft id written as 32 hex characters.
When an SLO is changed in the UI, reads show the change, so `terraform plan` reports it as drift.

| Datadog field | ObserveCraft |
|---|---|
| `type`: `metric`, `monitor`, `time_slice` | the SLO type; it can't change after creation |
| `query.numerator` / `query.denominator` | good and total events |
| `sli_specification.count` (`queries`, `good_events_formula`, `total_events_formula` or `bad_events_formula`) | good and total events, the named queries put into the formulas; with a bad-events formula the total is good + bad |
| `monitor_ids` | the monitors (of the same organization) whose alerts count as downtime |
| `sli_specification.time_slice` (`query`, `comparator`, `threshold`, `query_interval_seconds` 60 or 300) | time-slice SLO with 1- or 5-minute slices |
| `thresholds` (`7d`, `30d`, `90d`), `timeframe`, `target_threshold`, `warning_threshold` | targets per window and the primary window (the first threshold's when `timeframe` is absent) |
| `name`, `description`, `tags` | the same |

Metric queries keep their `.as_count()`, which is how ObserveCraft's metric SLOs count events anyway; a query without
an aggregator is averaged. Rejected with 400 and a message naming the field: `custom` timeframes, unknown types,
data sources other than `metrics`, `.as_rate()`, a `target_threshold` or `warning_threshold` that doesn't match the
primary threshold. Stored and returned but not acted on: `groups` of monitor SLOs (the SLO counts every group of its
monitors). SLOs made in the UI with a 1-day target show only their 7d/30d/90d thresholds, since Datadog has no
1-day timeframe. `force` on delete is accepted; nothing blocks deleting an SLO. Not available: SLO history,
`can_delete` and bulk timeframe deletion (no Terraform resource uses them).

The provider checks the monitors of a monitor SLO with `GET /api/v1/monitor/{id}`, which comes with
`datadog_monitor`; until then set `validate = false` on monitor SLOs.

The `datadog_service_level_objective(s)` data sources filter by `id`/`ids`, `name_query`, `tags_query` and
`metrics_query` (`GET /api/v1/slo`) or by `query` (`GET /api/v1/slo/search`, words matching the name, description or
tags, `key:value` matching a tag).

## SLO corrections

Corrections created through `/api/v1/slo/correction` are ObserveCraft SLO corrections: `slo_id`, `category`
(`Scheduled Maintenance`, `Outside Business Hours`, `Deployment`, `Other`), `description`, and `start` / `end` in
epoch seconds (at most 90 days). `timezone` is stored and returned. Updates (`PATCH`) replace the attributes sent;
the SLO of a correction can't change. Rejected with 400: recurring corrections (`rrule`, `duration`) and corrections
of the SLOs matching a query (`slo_query`), which ObserveCraft corrections can't express yet.

## How compatibility is tested

- **Contract fixtures.** `server/crates/oc-api/tests/datadog_api/` holds the request/response recordings that
  Datadog's Go API client is tested with (Apache-2.0). Each supported resource replays its recordings against the
  server and checks status codes and response shapes (`cargo test -p oc-api datadog_api`, with the live test
  databases `OC_TEST_POSTGRES_URL` and `OC_TEST_CLICKHOUSE_URL`).
- **The real provider.** `make tf-compat` runs Datadog's Terraform provider (pinned version) with OpenTofu against a
  server, for every fixture in `scripts/tf-compat/fixtures/` and both ways of authenticating (`api_key` + `app_key`,
  `bearer_token`): `apply`, a `plan` that must show no changes, an optional update (`update/` overlay) with another
  no-change `plan`, and `destroy`. It also checks that a wrong application key is refused.

  ```bash
  # against a running server
  OC_TF_API_URL=http://localhost:8080 OC_TF_APP_KEY=ocak_… OC_TF_API_KEY=ocik_… make tf-compat
  # or let it start one (needs Postgres, ClickHouse and Valkey; keys are created with psql)
  OC_POSTGRES_URL=… OC_CLICKHOUSE_URL=… OC_CACHE_URL=… make tf-compat TF_COMPAT_ARGS=--local
  ```

  `scripts/tf-compat/run.sh --help` lists every setting. OpenTofu is downloaded once (checksum verified) into
  `~/.cache/observecraft/tf-compat`, the provider from the OpenTofu registry. CI runs both checks when the
  compatibility code or fixtures change.
