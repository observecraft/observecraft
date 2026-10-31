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
| `datadog_user`, `datadog_user` / `datadog_users` (data sources) | `/api/v2/users`, `/api/v2/user_invitations` | ✅ |
| `datadog_role`, `datadog_user_role`, `datadog_role` / `datadog_roles` / `datadog_permissions` (data sources) | `/api/v2/roles` (+ `/permissions`, `/users`, `/clone`), `/api/v2/permissions` | ✅ |
| `datadog_team`, `datadog_team_membership`, `datadog_team` / `datadog_team_memberships` (data sources) | `/api/v2/team` (+ `/memberships`), `/api/v2/users/{id}/memberships` | ✅ |
| `datadog_team_link` | `/api/v2/team/{id}/links` | Not supported (teams have no links; listed as none) |
| `datadog_api_key` | `/api/v2/api_keys` | ✅ |
| `datadog_application_key` | `/api/v2/current_user/application_keys`, `/api/v2/application_keys` | ✅ creating keys needs `OC_DATADOG_API_ALLOW_APP_KEY_CREATION=true` ([details](#users-roles-teams-and-keys)) |
| `datadog_monitor`, `datadog_monitor_json`, `datadog_monitor` / `datadog_monitors` (data sources) | `/api/v1/monitor` (create, list, get, update, delete), `/api/v1/monitor/validate`, `/api/v1/monitor/{id}/validate`, `/api/v1/monitor/can_delete`, `/api/v1/monitor/search` | ✅ [details](#monitors) |
| `datadog_downtime_schedule` | `/api/v2/downtime` (create, list, get, update, cancel), `/api/v2/monitor/{id}/downtime_matches` | ✅ [details](#downtimes) |
| `datadog_downtime` (deprecated by the provider) | `/api/v1/downtime` (create, list, get, update, cancel), `/api/v1/downtime/cancel/by_scope` | ✅ [details](#downtimes) |
| `datadog_dashboard`, `datadog_dashboard_json`, `datadog_dashboard` (data source) | `/api/v1/dashboard` (create, get, update, delete, list, bulk delete and restore), `POST /api/unstable/dashboard/widgets/validate` | ✅ [details](#dashboards) |
| `datadog_dashboard_list`, `datadog_dashboard_list` (data source) | `/api/v1/dashboard/lists/manual`, `/api/v2/dashboard/lists/manual/{id}/dashboards` | ✅ [details](#dashboard-lists) |
| `datadog_service_level_objective`, data sources `datadog_service_level_objective(s)` | `/api/v1/slo` (create, get, update, delete, list), `/api/v1/slo/search` | ✅ [details](#service-level-objectives) |
| `datadog_slo_correction` | `/api/v1/slo/correction`, `/api/v1/slo/{id}/corrections` | ✅ one-off corrections ([details](#slo-corrections)) |
| `datadog_cost_budget`, `datadog_cost_budget` (data source) | `PUT /api/v2/cost/budget`, `GET`/`DELETE /api/v2/cost/budget/{id}`, `GET /api/v2/cost/budgets`, `POST /api/v2/cost/budget/validate` | ✅ [details](#cloud-cost) |
| `datadog_cost_custom_forecast` | `PUT /api/v2/cost/budget/custom-forecast`, `GET`/`DELETE /api/v2/cost/budget/{id}/custom-forecast` | ✅ [details](#cloud-cost) |
| Custom costs files (API clients) | `PUT`/`GET /api/v2/cost/custom_costs`, `GET`/`DELETE /api/v2/cost/custom_costs/{id}` | ✅ [details](#cloud-cost) |
| `datadog_aws_cur_config`, `datadog_azure_uc_config`, `datadog_gcp_uc_config`, `datadog_tag_pipeline_ruleset(s)`, `datadog_custom_allocation_rule(s)` | `/api/v2/cost/*_config`, `/api/v2/tags/enrichment`, `/api/v2/cost/arbitrary_rule` | Not yet |
| `datadog_apm_retention_filter`, `datadog_apm_retention_filter_order` | `/api/v2/apm/config/retention-filters` (create, list, get, update, delete), `PUT /api/v2/apm/config/retention-filters-execution-order` | ✅ [details](#apm-retention-filters-and-span-based-metrics) |
| `datadog_spans_metric` | `/api/v2/apm/config/metrics` (create, list, get, update, delete) | ✅ [details](#apm-retention-filters-and-span-based-metrics) |

## Cloud cost

Costs are daily line items in the FOCUS columns (`otel.cloud_costs`). They come from custom costs files: a JSON list
of line items (`ProviderName`, `ChargeDescription`, `ChargePeriodStart`, `ChargePeriodEnd`, `BilledCost`,
`BillingCurrency` `USD`, `Tags`), each spread evenly over the days of its charge period (the end date excluded). The
cloud providers' bills (AWS CUR, Azure and Google Cloud cost exports) aren't read yet.

They are queried as Datadog's cost metrics, in dashboards (`data_source: cloud_cost` or `metrics`), monitors, notebooks
and budgets: `all.cost`, `<provider>.cost.amortized` (`aws`, `azure`, `gcp`, or a custom provider's name) and
`<provider>.cost.unblended` / `.actual`, `custom.cost.basis` / `custom.cost.amortized`, summed per day, with the tags
`providername`, `servicename`, `subaccountname`, `regionname`, `chargecategory`, `chargedescription`, … and the line
items' own tags.

A budget's `metrics_query` has no aggregator (`aws.cost.amortized{env:prod} by {team}`). Its entries name each tag of
the query's `by {…}` once, and each tag combination has an entry for every month from `start_month` to `end_month`;
`/validate` answers `valid: false` with the problems, and an upsert with them answers 400. `GET …/budget/{id}` with
`actual=true` or `forecast=true` adds each entry's `costs`: what the month cost so far and, for the current month, the
cost at the pace so far (or the custom forecast).

## APM retention filters and span-based metrics

APM → Settings reads and writes the same records.

- Every organization has two default retention filters with fixed ids, `errors-default` (`status:error`) and
  `appsec-default` (App & API Protection traces). They can be changed and disabled but not deleted, and a
  `datadog_apm_retention_filter_order` lists them with the others. New filters are `spans-sampling-processor`.
- Retention filters are evaluated, not applied: every ingested span stays searchable for the organization's span
  retention. APM → Settings shows the spans each filter matches first and what its rate keeps.
- A span-based metric is computed every minute from the stored spans (two minutes behind, so late spans count) and
  queried like any other metric: `count` as a count, `distribution` of `@duration` (nanoseconds) or another numeric
  attribute as a distribution. The name is the id; characters outside `[A-Za-z0-9_.]` become `_`.
- Queries take Datadog's span search keys: `service`, `env`, `resource_name`, `operation_name`, `status`,
  `@duration`, `@_top_level`, `@http.status_code`, `@http.method` and any `@attribute`.

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

## Dashboards

Dashboards written through `/api/v1/dashboard` are ordinary ObserveCraft dashboards: they are in the dashboard list
(Custom), open in the UI and can be edited there.

- **What you write is what you read.** The Datadog document is stored as it was sent (widget ids are assigned when
  missing) and returned with the fields the server owns: `id` (`abc-def-ghi`), `url` (the dashboard's address in the
  UI), `author_handle`, `author_name`, `created_at`, `modified_at`. Fields Datadog would add on its own
  (`restricted_roles: []`, `is_read_only: false`, …) are not added, so `datadog_dashboard_json` plans stay clean.
- **Edits in the UI show up in Terraform.** Moving, resizing, renaming, removing or adding widgets, the title,
  the description, the teams and the template variables are folded back into the Datadog document on read; a
  `terraform plan` then shows the drift as it would against Datadog. Widgets the UI didn't touch keep their exact
  Datadog definition.
- **Nothing is dropped.** A dashboard with a widget the UI can't draw is refused as a whole with a 400 whose message
  starts with the widget's path, e.g. `widgets[3].definition.widgets[1].definition.type: widget type "hostmap" is not
  supported by ObserveCraft yet`. During `plan`, the provider asks `POST /api/unstable/dashboard/widgets/validate`,
  which answers the same check per widget, so the error appears before anything is applied.
- `tags` are teams (`team:<name>`, as in Datadog); they become the dashboard's teams.
- `filter[shared]=true` lists nothing: shared (public) dashboards are not part of this API.

### Widget types

| Datadog widget | Drawn as | Requests |
|---|---|---|
| `timeseries`, `query_value`, `toplist`, `query_table`, `change`, `heatmap`, `sunburst`, `treemap` | the Datadog definition itself (requests, formulas, units, conditional formats, display options) | `q`, or `queries` with `data_source` `metrics`, `logs` or `events`, and `formulas` |
| `check_status` | the check's status counts | `check`, `grouping`, `tags` |
| `list_stream` | events, or Kubernetes resources | `data_source` `event_stream` or `infrastructure` |
| `note`, `free_text` | a note | |
| `image` | an image (`url`) | |
| `group` | a group (title, color, banner image, widgets) | |
| `log_stream` | our log stream (`query`) | |
| `manage_status` | our monitor summary (`query`, matched against monitor names) | |

Not supported yet, and refused with the widget's path: the widget types `alert_graph`, `alert_value`, `bar_chart`,
`distribution`, `event_stream`, `event_timeline`, `funnel`, `geomap`, `hostmap`, `iframe`, `point_plot`, `powerpack`,
`run_workflow`, `sankey`, `scatterplot`, `servicemap`, `slo`, `slo_list`, `split_group`, `topology_map`,
`trace_service` (and any other); legacy per-product requests (`apm_query`, `log_query`, `rum_query`,
`process_query`, `profile_metrics_query`, `apm_stats_query`, …) and the single `query` request of histogram widgets;
`queries` with other data sources (`apm_metrics`, `apm_resource_stats`, `apm_dependency_stats`,
`ci_pipelines`, `ci_tests`, `audit`, `rum`, `slo`, `product_analytics`, `incident_analytics`, …); `list_stream`
data sources other than `event_stream` and `infrastructure` (`logs_stream`, `apm_issue_stream`, `trace_stream`, …).

Accepted, stored and returned, but not acted on: `notify_list`, `restricted_roles` (edit permissions follow the
user's role), `is_read_only`, `template_variable_presets` (the UI has no saved views yet), `available_values` of
template variables (the UI offers the tag's values), `reflow_type`, and widget options the panels don't draw
(`live_span` and `time` per widget, custom links, markers, events overlays). A template variable with several
`defaults` filters on the first.

The check is on widget types and data sources, not on the text of each query: a metric query the panels can't
evaluate yet (for example a percentile space aggregator such as `p95:…`) is stored, and the widget shows the
reason in the dashboard.

## Dashboard lists

`/api/v1/dashboard/lists/manual` and `/api/v2/dashboard/lists/manual/{id}/dashboards` work on the lists of the
dashboard list's sidebar, so a list made in Terraform shows there and one made in the UI can be read and managed
here. List ids are integers. An item's `type` comes from the dashboard: `custom_timeboard` / `custom_screenboard`
for dashboards made here or in the UI, `integration_timeboard` / `integration_screenboard` for integration
dashboards (a `free` layout is a screenboard). `is_favorite` is always `false`; deleting a list keeps its
dashboards.

## Monitors

A monitor written through `/api/v1/monitor` is an ordinary ObserveCraft monitor: it shows in Monitors, is
evaluated every minute and notifies its `@handles`, and a monitor made in the UI can be read, imported and managed
with Terraform. Monitor ids are the same integers in both APIs.

The document you send is kept and returned as you sent it, with the fields the server owns filled in (`id`,
`org_id`, `created`, `created_at`, `modified`, `creator`, `deleted`, `multi`, `overall_state`,
`overall_state_modified`, `options.silenced`, and `matching_downtimes` with `with_downtimes`). Fields left out get
Datadog's defaults (`options.notify_no_data`, `notify_audit`, `new_host_delay`, `include_tags`, and for log alerts
`groupby_simple_monitor`; `tags`, `priority`, `restricted_roles`, `draft_status`). An update (`PUT`) replaces the
fields it sends, `options` as a whole. When a monitor is changed in the UI, the next read shows the change in the
document (name, message, priority, tags, draft, query, thresholds, no-data settings), so `terraform plan` reports
it as drift.

| Datadog | ObserveCraft |
|---|---|
| `type: metric alert`, `query alert` | metric monitor; an APM monitor when every metric is `trace.*` |
| `type: log alert` | log monitor |
| `query`: `avg\|sum\|min\|max\|last(last_<n><m\|h\|d\|w>):<metric query> <op> <threshold>`, `change(…,last_…)`, `pct_change(…,last_…)`, arithmetic between metric queries | time aggregation, evaluation window, change detection, comparator (`>`, `>=`, `<`, `<=`) and alert threshold |
| `query`: `logs("<search>").index(…).rollup("count").by("<facets>").last("<window>") <op> <threshold>` | log search, group-by facets, window (counts summed over it), comparator, threshold |
| `options.thresholds.critical`, `warning`, `critical_recovery`, `warning_recovery` | thresholds and recovery thresholds (`critical` must match the query) |
| `options.notify_no_data`, `on_missing_data: show_and_notify_no_data` | notify on no data |
| `options.no_data_timeframe` | no-data timeframe (default twice the window) |
| `name`, `message` (its `@handles` are the recipients), `priority`, `tags`, `draft_status` | the same |

**Accepted, stored and returned, not enforced yet:** `options.evaluation_delay`, `new_group_delay`,
`new_host_delay`, `renotify_interval`, `renotify_occurrences`, `renotify_statuses`, `escalation_message`,
`include_tags`, `require_full_window`, `notify_by`, `notify_audit`, `timeout_h`, `on_missing_data` (other than
notifying on no data), `group_retention_duration`, `threshold_windows`, `scheduling_options`,
`notification_preset_name`, `groupby_simple_monitor`, `enable_logs_sample`, `enable_samples`, `locked`,
`thresholds.ok` and `thresholds.unknown`; `restricted_roles` (editing follows our roles), `assets`, the log
query's `.index(…)` (every index is searched). These are listed here only; the UI doesn't flag them.

**Rejected with a 400** naming what isn't supported: monitor types other than the three above (`composite`,
`service check`, `event alert`, `event-v2 alert`, `process alert`, `rum alert`, `trace-analytics alert`,
`slo alert`, `audit alert`, `ci-pipelines alert`, `ci-tests alert`, `error-tracking alert`,
`database-monitoring alert`, `network-performance alert`, `cost alert`, `data-quality alert`, `data-jobs alert`,
`llm-observability alert`, …); formula and function queries (`options.variables`); calendar-aligned windows
(`current_1d`, `current_1mo`); time aggregations other than avg, sum, min, max and last; metric functions our
query language doesn't have (e.g. `anomalies()`, `forecast()`, `.as_count()`); log rollups other than `count`;
changing a monitor's type between metric and log. `synthetics alert` monitors (our synthetic tests' own monitors)
can be read but are managed with their test.

`DELETE` refuses a monitor that an SLO uses unless `force=true`; `can_delete` reports those. `GET /api/v1/monitor`
filters by `name` (substring), `tags` (in the query's scope), `monitor_tags` (on the monitor), `id_offset`, `page`
and `page_size`, as the `datadog_monitor(s)` data sources call it; `/api/v1/monitor/search` takes `status:`,
`type:`, `muted:`, `tag:`, `priority:` terms and free text. `/api/v1/monitor/groups/search` isn't available.

Writes need the Standard or Admin role, as in the UI.

## Downtimes

Downtimes from either API are our downtimes (Monitors → Downtimes): v2 ids are UUIDs, v1 ids integers, and each
downtime has both, so one written through one version can be read through the other. Documents are returned as
written, with `status`, `canceled`, `created`, `modified` (and v1's `active`, `disabled`, `creator_id`,
`updater_id`) from the server. Deleting cancels; canceling twice is fine.

| Datadog | ObserveCraft |
|---|---|
| v2 `monitor_identifier.monitor_id`, v1 `monitor_id` | the monitor it silences |
| v2 `monitor_identifier.monitor_tags`, v1 `monitor_tags` (`["*"]` = all monitors) | monitors carrying all those tags |
| v2 `scope` (`*`, or `key:value` terms joined with `AND`, values may be quoted), v1 `scope` (tag list) | groups covered |
| v2 `schedule.start` / `end`, v1 `start` / `end` | one-time window (no end = until canceled) |
| v2 `recurrences[0]` with `rrule` `FREQ=DAILY` or `FREQ=WEEKLY` (+ `BYDAY`, `UNTIL`, `COUNT`), `duration`, `start` in `schedule.timezone`; v1 `recurrence` `days` / `weeks` with `period` 1 (+ `week_days`, `until_date`, `until_occurrences`) in `timezone` | a daily or weekly repetition, limited to weekdays, until a date |
| `message` | the same |

**Accepted, stored and returned, not enforced yet:** `display_timezone`, `mute_first_recovery_notification`,
`notify_end_states`, `notify_end_types`. Recurrences follow the timezone's offset at their start (they don't move
with daylight-saving changes).

**Rejected with a 400:** monthly and yearly recurrences, repeating every N > 1 days or weeks, v1 `rrule`
recurrences, more than one v2 recurrence, other RRULE parts (`BYMONTHDAY`, `BYHOUR`, …), and v2 scopes with `OR`,
`NOT`, `-`, parentheses or wildcards. Monitors in another organization are not found.

## Users, roles, teams and keys

These routes work on the same users, roles, teams and keys as Organization Settings, so what Terraform creates shows
there and the other way round. Only organization admins manage users, roles, role assignments, team memberships and
API keys, as in our own API; every member can read them. Standard users create and edit teams.

- **Users.** Creating a user invites them: a pending member with the given roles. `send_user_invitation` issues the
  one-time invitation link; ObserveCraft doesn't send it by email, an admin shares it from Organization Settings →
  Users. `disabled = true` deactivates the user (signed out everywhere, their application keys stop working), and
  destroying a `datadog_user` deactivates it too; creating it again re-enables it. A user's email can't change, and
  admins can't disable themselves.
- **Roles.** The built-in roles (ObserveCraft Admin, Standard and Read Only, also found as "Datadog Admin Role",
  "Datadog Standard Role" and "Datadog Read Only Role") report the permissions of their access level and can't be
  changed or deleted. A custom role keeps exactly the permissions it was given and reads them back, but access in
  ObserveCraft still comes from the built-in role a user has: give users one of those for what they may do, and use
  custom roles to group them (for example in log restriction queries). Permissions ObserveCraft doesn't know are
  rejected with a 400 that names them, and nobody can give a role a permission they don't have. Permission ids are
  ObserveCraft's own (stable on every install); look them up with `datadog_permissions` rather than copying Datadog's.
  Admins can't remove their own admin role.
- **Teams.** A team's handle can't change once created (incidents, on-call and log restrictions refer to it); create a
  new team instead. Team links and the display settings avatar, banner and visible or hidden modules aren't
  supported and are rejected. A membership's role is `admin` or unset.
- **API keys** are ingest keys (`ocik_…`), **application keys** ours (`ocak_…`). The secret is returned once, when the
  key is created, and stored hashed; Terraform keeps it in its state. Later reads don't include it, so the
  deprecated `datadog_api_key` data source, which needs it, reports that it can't read the key. Destroying a key
  revokes it. Creating application keys through this API is off by default (`OC_DATADOG_API_ALLOW_APP_KEY_CREATION=false`): the caller is itself an application key, and a leaked key could otherwise create keys that keep working after it is revoked. With it off, `POST /api/v2/current_user/application_keys` answers 403 and keys are created in ObserveCraft (Organization Settings → Application Keys); listing, renaming and revoking keys work either way. `datadog_application_key` creates a key for the provider's own user, with that user's role; scoped keys
  (`scopes`) and `remote_config_read_enabled = true` are rejected because ObserveCraft doesn't enforce scopes and has no
  remote configuration.

### Permissions

`GET /api/v2/permissions` lists the Datadog permission names ObserveCraft supports. Each is granted from one of our
access levels up (Read Only < Standard < Admin):

| Access level | Permissions |
|---|---|
| Read Only (every role) | `dashboards_read`, `monitors_read`, `slos_read`, `logs_read_data`, `logs_read_index_data`, `logs_live_tail`, `logs_read_archives`, `apm_read`, `apm_service_catalog_read`, `continuous_profiler_read`, `error_tracking_read`, `incident_read`, `notebooks_read`, `synthetics_read`, `synthetics_global_variable_read`, `ci_visibility_read`, `deployment_gates_read`, `on_call_read`, `status_pages_settings_read`, `reference_tables_read`, `sheets_read`, `feature_flag_config_read`, `workflows_read`, `dbm_read`, `network_connections_read`, `llm_observability_read`, `watchdog_insights_read`, `event_correlation_config_read`, `api_keys_read`, `user_app_keys` |
| Standard | `dashboards_write`, `dashboards_public_share`, `generate_dashboard_reports`, `monitors_write`, `monitors_downtime`, `slos_write`, `slos_corrections`, `logs_write_pipelines`, `logs_write_processors`, `logs_generate_metrics`, `apm_service_catalog_write`, `apm_generate_metrics`, `error_tracking_write`, `incident_write`, `notebooks_write`, `synthetics_write`, `synthetics_global_variable_write`, `deployment_gates_write`, `on_call_write`, `status_pages_settings_write`, `status_pages_incident_write`, `reference_tables_write`, `sheets_write`, `feature_flag_config_write`, `workflows_write`, `workflows_run`, `llm_observability_write`, `event_correlation_config_write` |
| Admin | `logs_modify_indexes`, `logs_write_exclusion_filters`, `logs_write_archives`, `api_keys_write`, `api_keys_delete`, `org_app_keys_read`, `org_app_keys_write`, `user_access_invite`, `user_access_manage`, `teams_manage`, `org_management`, `audit_logs_read` |

None of them is "restricted" in Datadog's sense, so `default_permissions_opt_out` and `include_restricted` change
nothing.

## How compatibility is tested

- **Contract fixtures.** The tests replay the request/response recordings that Datadog's Go API client
  (`datadog-api-client-go`, Apache-2.0) is tested with. They are not in this repository: `make datadog-fixtures`
  (`scripts/datadog-fixtures/fetch.sh`) downloads them from a pinned commit, checks the archive's SHA-256 and
  extracts the features we replay, unchanged, into `~/.cache/observecraft/datadog-api-client-go/<commit>`
  (`OC_DATADOG_FIXTURES` sets another directory). Each supported resource replays its recordings against the
  server and checks status codes and response shapes:

  ```bash
  make datadog-fixtures
  cd server && OC_TEST_POSTGRES_URL=… OC_TEST_CLICKHOUSE_URL=… cargo test -p oc-api datadog_api
  ```

  Without the fixtures the replay tests are skipped with a message saying how to fetch them; with
  `OC_REQUIRE_DATADOG_FIXTURES=1`, as in CI, they fail instead. For dashboards, every recorded scenario whose
  widgets we draw is replayed as recorded; scenarios about the dashboard itself (get, update, delete, list,
  restore, template variables, dashboard lists) whose fixture dashboards Datadog's client builds with a widget we
  don't draw (a `profile_metrics_query` timeseries, a `hostmap`) are replayed with that widget replaced by a metric
  timeseries; every other scenario must be refused with a 400 naming the widget.
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
