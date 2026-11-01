//! Cloud Cost Management, Datadog-compatible: budgets (`/api/v2/cost/budget(s)`, the provider's
//! `datadog_cost_budget`), their custom forecasts (`/api/v2/cost/budget/custom-forecast`,
//! `datadog_cost_custom_forecast`) and custom costs files (`/api/v2/cost/custom_costs`).
//!
//! - A budget is a cost query (`aws.cost.amortized{env:prod} by {team}`, no aggregator) and an
//!   amount per month and combination of its `by` tags; every combination has every month. `PUT`
//!   creates it, or replaces the one its `id` names. Reads with `?actual=true` / `?forecast=true` add
//!   what each entry's month cost so far ([`cloud_cost::monthly_costs`]) and where it is heading.
//! - A custom costs file is a list of FOCUS line items (`ProviderName`, `ChargeDescription`,
//!   `ChargePeriodStart` / `End`, `BilledCost`, `BillingCurrency`, `Tags`). Each item's cost is spread
//!   evenly over the days of its charge period (end exclusive) into `otel.cloud_costs`, where the
//!   cost metrics read it (`custom.cost.basis`, `all.cost`, `<provider>.cost.*`).
//!
//! Tenancy: every Postgres query filters on `org_id`; ClickHouse goes through `AppState::ch(org)`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{Datelike, NaiveDate, Utc};
use serde_json::{json, Value};

use super::users::{bad_request, body, data, not_found, ok, require, uuid, DocResult};
use super::{DdAuth, DdError};
use crate::auth::{Access, Writer};
use crate::error::{ApiError, ApiResult};
use crate::routes::cloud_cost::{self, CostLine};
use crate::state::{AppState, OrgId};

/// Line items a custom costs file may hold.
const MAX_LINE_ITEMS: usize = 100_000;
/// Days one line item's charge period may cover.
const MAX_PERIOD_DAYS: i64 = 366;

fn bool_param(q: &HashMap<String, String>, key: &str) -> bool {
    q.get(key).is_some_and(|v| v == "true" || v == "1")
}

/// `YYYYMM` → its first day.
fn month_start(m: i64) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt((m / 100) as i32, u32::try_from(m % 100).ok()?, 1).filter(|_| (190001..=299912).contains(&m))
}

fn months(start: i64, end: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let (Some(mut d), Some(last)) = (month_start(start), month_start(end)) else { return out };
    while d <= last && out.len() < 1200 {
        out.push(d.year() as i64 * 100 + d.month() as i64);
        d = d.checked_add_months(chrono::Months::new(1)).unwrap_or(last + chrono::Duration::days(1));
    }
    out
}

// ================================================================ budgets

/// A budget entry's tag filters, as `(key, value)` pairs.
type Filters = Vec<(String, String)>;

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    month: i64,
    amount: f64,
    filters: Filters,
}

#[derive(Debug)]
struct BudgetInput {
    name: String,
    metrics_query: String,
    start_month: i64,
    end_month: i64,
    entries: Vec<Entry>,
}

fn filters_of(v: &Value) -> Result<Filters, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|f| match (f.get("tag_key").and_then(Value::as_str), f.get("tag_value").and_then(Value::as_str)) {
                (Some(k), Some(v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
                _ => Err("each tag filter needs a `tag_key` and a `tag_value`".to_string()),
            })
            .collect(),
        _ => Err("`tag_filters` must be a list".into()),
    }
}

fn filters_json(f: &Filters) -> Value {
    Value::Array(f.iter().map(|(k, v)| json!({ "tag_key": k, "tag_value": v })).collect())
}

/// The entries as stored and answered.
fn entries_json(entries: &[Entry]) -> Value {
    Value::Array(entries.iter().map(|e| json!({ "month": e.month, "amount": e.amount, "tag_filters": filters_json(&e.filters) })).collect())
}

fn entries_of(v: &Value) -> Vec<Entry> {
    v.as_array()
        .map(|items| {
            items
                .iter()
                .map(|e| Entry {
                    month: e.get("month").and_then(Value::as_i64).unwrap_or_default(),
                    amount: e.get("amount").and_then(Value::as_f64).unwrap_or_default(),
                    filters: filters_of(e.get("tag_filters").unwrap_or(&Value::Null)).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A budget's attributes, checked as Datadog checks them: every problem, not only the first.
fn parse_budget(attrs: &Value) -> Result<BudgetInput, Vec<String>> {
    let mut errors = Vec::new();
    let name = attrs.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    if name.is_empty() {
        errors.push("`name` is required".to_string());
    } else if name.chars().count() > 256 {
        errors.push("`name` can't exceed 256 characters".to_string());
    }
    let metrics_query = attrs.get("metrics_query").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let by = match cloud_cost::budget_query(&metrics_query) {
        Ok(q) => q.by,
        Err(e) => {
            errors.push(if metrics_query.is_empty() { "`metrics_query` is required".to_string() } else { format!("invalid `metrics_query`: {e}") });
            Vec::new()
        }
    };
    let month = |key: &str, errors: &mut Vec<String>| match attrs.get(key).and_then(Value::as_i64) {
        Some(m) if month_start(m).is_some() => m,
        _ => {
            errors.push(format!("`{key}` must be a month as YYYYMM"));
            0
        }
    };
    let start_month = month("start_month", &mut errors);
    let end_month = month("end_month", &mut errors);
    if start_month > 0 && end_month > 0 && start_month > end_month {
        errors.push("`start_month` must not be after `end_month`".into());
    }
    let range = months(start_month, end_month);
    if range.len() > 60 {
        errors.push("a budget can't span more than 60 months".into());
    }
    let mut entries = Vec::new();
    match attrs.get("entries") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for (i, e) in items.iter().enumerate() {
                let month = e.get("month").and_then(Value::as_i64).unwrap_or_default();
                if !range.contains(&month) {
                    errors.push(format!("entry {i}: month {month} is outside the budget's months"));
                }
                let amount = e.get("amount").and_then(Value::as_f64);
                if amount.is_none_or(|a| a < 0.0 || !a.is_finite()) {
                    errors.push(format!("entry {i}: `amount` must be a number of at least 0"));
                }
                let filters = filters_of(e.get("tag_filters").unwrap_or(&Value::Null)).unwrap_or_else(|err| {
                    errors.push(format!("entry {i}: {err}"));
                    Vec::new()
                });
                let keys: BTreeSet<&str> = filters.iter().map(|(k, _)| k.as_str()).collect();
                let want: BTreeSet<&str> = by.iter().map(String::as_str).collect();
                if keys != want || keys.len() != filters.len() {
                    errors.push(format!("entry {i}: the tag filters must name each tag of the query's `by {{…}}` once ({})", by.join(", ")));
                }
                entries.push(Entry { month, amount: amount.unwrap_or_default(), filters });
            }
        }
        Some(_) => errors.push("`entries` must be a list".into()),
    }
    // Every tag combination has each month, once.
    let mut combos: BTreeMap<Filters, Vec<i64>> = BTreeMap::new();
    for e in &entries {
        let mut key = e.filters.clone();
        key.sort();
        combos.entry(key).or_default().push(e.month);
    }
    for (filters, mut ms) in combos {
        ms.sort_unstable();
        let label = if filters.is_empty() { "the budget".to_string() } else { filters.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(", ") };
        if ms.windows(2).any(|w| w[0] == w[1]) {
            errors.push(format!("{label} has more than one entry for a month"));
        } else if ms != range && !range.is_empty() {
            errors.push(format!("{label} needs an entry for every month from {start_month} to {end_month}"));
        }
    }
    if errors.is_empty() {
        Ok(BudgetInput { name, metrics_query, start_month, end_month, entries })
    } else {
        Err(errors)
    }
}

#[derive(sqlx::FromRow)]
struct BudgetRow {
    id: String,
    name: String,
    metrics_query: String,
    start_month: i32,
    end_month: i32,
    entries: Value,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const BUDGET_COLS: &str = "id::text AS id, name, metrics_query, start_month, end_month, entries, created_by::text AS created_by, updated_by::text AS updated_by, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

/// Costs of an entry or budget: `amount` budgeted, `actual` so far, `forecast` for the whole month.
#[derive(Default, Clone, Copy)]
struct Costs {
    amount: f64,
    actual: Option<f64>,
    forecast: Option<f64>,
    custom_forecast: Option<f64>,
}

impl Costs {
    fn json(&self, entry: bool) -> Value {
        let mut v = json!({ "amount": self.amount, "actual": self.actual, "forecast": self.forecast.or(self.custom_forecast), "ootb_forecast": self.forecast });
        if entry {
            v["custom_forecast"] = json!(self.custom_forecast);
        }
        v
    }
}

fn budget_doc(b: &BudgetRow, entries: Option<&[(Entry, Option<Costs>)]>) -> Value {
    let all = entries_of(&b.entries);
    let total: f64 = all.iter().map(|e| e.amount).sum();
    let mut attrs = json!({
        "name": b.name, "metrics_query": b.metrics_query, "start_month": b.start_month, "end_month": b.end_month,
        "total_amount": total, "created_at": b.created_at, "created_by": b.created_by.clone().unwrap_or_default(),
        "updated_at": b.updated_at, "updated_by": b.updated_by.clone().unwrap_or_default(),
    });
    if let Some(entries) = entries {
        attrs["entries"] = Value::Array(
            entries
                .iter()
                .map(|(e, costs)| {
                    let mut v = json!({ "month": e.month, "amount": e.amount, "tag_filters": filters_json(&e.filters) });
                    if let Some(c) = costs {
                        v["costs"] = c.json(true);
                    }
                    v
                })
                .collect(),
        );
        if entries.iter().any(|(_, c)| c.is_some()) {
            let sum = |f: fn(&Costs) -> Option<f64>| -> Option<f64> {
                let vals: Vec<f64> = entries.iter().filter_map(|(_, c)| c.as_ref().and_then(f)).collect();
                (!vals.is_empty()).then(|| vals.iter().sum())
            };
            let total = Costs { amount: total, actual: sum(|c| c.actual), forecast: sum(|c| c.forecast.or(c.custom_forecast)), custom_forecast: None };
            attrs["costs"] = total.json(false);
        }
    }
    json!({ "type": "budget", "id": b.id, "attributes": attrs })
}

async fn find_budget(st: &AppState, org: &str, id: &str) -> Result<Option<BudgetRow>, DdError> {
    let Some(id) = uuid(id) else { return Ok(None) };
    Ok(sqlx::query_as(&format!("SELECT {BUDGET_COLS} FROM cost_budgets WHERE org_id = $1 AND id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?)
}

/// What each entry's month cost so far and where it's heading (this month: at the pace so far).
async fn entry_costs(st: &AppState, org: &str, b: &BudgetRow, custom: &[Entry]) -> Result<Vec<(Entry, Option<Costs>)>, DdError> {
    let q = cloud_cost::budget_query(&b.metrics_query).map_err(bad_request)?;
    let actuals = cloud_cost::monthly_costs(st, org, &q, b.start_month as i64, b.end_month as i64).await?;
    let by_month: HashMap<(i64, Vec<String>), f64> = actuals.into_iter().map(|(m, g, c)| ((m, g), c)).collect();
    let today = Utc::now().date_naive();
    let this_month = today.year() as i64 * 100 + today.month() as i64;
    Ok(entries_of(&b.entries)
        .into_iter()
        .map(|e| {
            let group: Vec<String> = q.by.iter().map(|k| e.filters.iter().find(|(fk, _)| fk == k).map(|(_, v)| v.clone()).unwrap_or_default()).collect();
            let actual = (e.month <= this_month).then(|| by_month.get(&(e.month, group)).copied().unwrap_or(0.0));
            let forecast = match actual {
                Some(a) if e.month == this_month => {
                    let first = month_start(e.month).unwrap_or(today);
                    let days = (first.checked_add_months(chrono::Months::new(1)).unwrap_or(today) - first).num_days() as f64;
                    let elapsed = (today - first).num_days().max(1) as f64;
                    Some((a / elapsed * days * 100.0).round() / 100.0)
                }
                other => other,
            };
            let mut sorted = e.filters.clone();
            sorted.sort();
            let custom_forecast = custom
                .iter()
                .find(|c| {
                    let mut f = c.filters.clone();
                    f.sort();
                    c.month == e.month && f == sorted
                })
                .map(|c| c.amount);
            (e.clone(), Some(Costs { amount: e.amount, actual, forecast, custom_forecast }))
        })
        .collect())
}

/// List budgets (Datadog-compatible), without their entries.
#[utoipa::path(get, path = "/api/v2/cost/budgets", operation_id = "datadog_list_budgets", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_budgets(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows: Vec<BudgetRow> = sqlx::query_as(&format!("SELECT {BUDGET_COLS} FROM cost_budgets WHERE org_id = $1 ORDER BY created_at, id")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    ok(json!({ "data": rows.iter().map(|b| budget_doc(b, None)).collect::<Vec<_>>() }))
}

/// Get a budget with its entries (Datadog-compatible); `actual` / `forecast` add their costs.
#[utoipa::path(get, path = "/api/v2/cost/budget/{budget_id}", operation_id = "datadog_get_budget", params(("budget_id" = String, Path)),
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_budget(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> DocResult {
    let b = find_budget(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found("Not Found"))?;
    let entries: Vec<(Entry, Option<Costs>)> = if bool_param(&q, "actual") || bool_param(&q, "forecast") {
        let custom = custom_forecast_entries(&st, &ctx.org_id, &b.id).await?;
        entry_costs(&st, &ctx.org_id, &b, &custom).await?
    } else {
        entries_of(&b.entries).into_iter().map(|e| (e, None)).collect()
    };
    ok(json!({ "data": budget_doc(&b, Some(&entries)) }))
}

fn budget_type(d: &Value) -> Result<(), DdError> {
    match d.get("type").and_then(Value::as_str).unwrap_or_default() {
        "budget" => Ok(()),
        other => Err(bad_request(format!("got type \"{other}\" expected one of \"budget\""))),
    }
}

/// Create a budget, or replace the one `data.id` names (Datadog-compatible).
#[utoipa::path(put, path = "/api/v2/cost/budget", operation_id = "datadog_upsert_budget", request_body = Value,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn upsert_budget(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let doc = body(&raw)?;
    let id = save_budget(&st, &ctx.org_id, &ctx.user_id, &doc).await?;
    let b = find_budget(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found("Not Found"))?;
    let entries: Vec<(Entry, Option<Costs>)> = entries_of(&b.entries).into_iter().map(|e| (e, None)).collect();
    ok(json!({ "data": budget_doc(&b, Some(&entries)) }))
}

/// Saves a budget document (`{"data": {"type": "budget", "id"?, "attributes": …}}`) as `user`: the
/// Datadog endpoint and the Cloud Cost pages share it. Returns the budget's id.
async fn save_budget(st: &AppState, org: &str, user: &str, doc: &Value) -> Result<String, DdError> {
    let d = data(doc)?;
    budget_type(d)?;
    let attrs = d.get("attributes").cloned().unwrap_or(Value::Null);
    let input = parse_budget(&attrs).map_err(|errors| DdError { status: StatusCode::BAD_REQUEST, errors })?;
    let entries = entries_json(&input.entries);
    Ok(match d.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(id) => {
            let not_found = || not_found("failed to upsert budget: budget not found");
            let id = uuid(id).ok_or_else(not_found)?;
            let updated = sqlx::query(
                "UPDATE cost_budgets SET name = $3, metrics_query = $4, start_month = $5, end_month = $6, entries = $7, updated_by = $8::uuid, updated_at = now() \
                 WHERE org_id = $1 AND id = $2::uuid",
            )
            .bind(org)
            .bind(&id)
            .bind(&input.name)
            .bind(&input.metrics_query)
            .bind(input.start_month as i32)
            .bind(input.end_month as i32)
            .bind(&entries)
            .bind(user)
            .execute(&st.postgres)
            .await?;
            if updated.rows_affected() == 0 {
                return Err(not_found());
            }
            id
        }
        None => {
            let (id,): (String,) = sqlx::query_as(
                "INSERT INTO cost_budgets (org_id, name, metrics_query, start_month, end_month, entries, created_by, updated_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $7::uuid) RETURNING id::text",
            )
            .bind(org)
            .bind(&input.name)
            .bind(&input.metrics_query)
            .bind(input.start_month as i32)
            .bind(input.end_month as i32)
            .bind(&entries)
            .bind(user)
            .fetch_one(&st.postgres)
            .await?;
            id
        }
    })
}

/// Delete a budget (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/cost/budget/{budget_id}", operation_id = "datadog_delete_budget", params(("budget_id" = String, Path)),
    responses((status = 204), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_budget(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Standard)?;
    remove_budget(&st, &ctx.org_id, &id).await
}

async fn remove_budget(st: &AppState, org: &str, id: &str) -> Result<StatusCode, DdError> {
    let id = uuid(id).ok_or_else(|| bad_request("invalid budgetId"))?;
    let deleted = sqlx::query("DELETE FROM cost_budgets WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(&id).execute(&st.postgres).await?;
    if deleted.rows_affected() == 0 {
        return Err(not_found("Not Found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Check a budget without saving it (Datadog-compatible; Terraform calls it while planning).
#[utoipa::path(post, path = "/api/v2/cost/budget/validate", operation_id = "datadog_validate_budget", request_body = Value,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn validate_budget(DdAuth(_ctx): DdAuth, raw: Bytes) -> DocResult {
    let doc = body(&raw)?;
    let d = data(&doc)?;
    budget_type(d)?;
    let errors = parse_budget(d.get("attributes").unwrap_or(&Value::Null)).err().unwrap_or_default();
    ok(json!({ "data": { "type": "budget_validation", "id": d.get("id").cloned().unwrap_or(Value::Null), "attributes": { "valid": errors.is_empty(), "errors": errors } } }))
}

/// A budget for the Cloud Cost pages: this month's amount, cost so far and forecast.
#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CostBudgetSummary {
    pub id: String,
    pub name: String,
    pub metrics_query: String,
    pub start_month: i32,
    pub end_month: i32,
    pub total_amount: f64,
    /// This month (`YYYYMM`), when the budget covers it.
    pub month: Option<i64>,
    pub month_amount: f64,
    pub month_actual: f64,
    pub month_forecast: f64,
}

/// The org's budgets with this month's figures (Cloud Cost → Plan, Overview).
#[utoipa::path(get, path = "/api/v1/cloud-cost/budgets", operation_id = "cloud_cost_budgets", responses((status = 200, body = Vec<CostBudgetSummary>)), tag = "cloud-cost")]
pub async fn budget_summaries(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<CostBudgetSummary>> {
    let rows: Vec<BudgetRow> = sqlx::query_as(&format!("SELECT {BUDGET_COLS} FROM cost_budgets WHERE org_id = $1 ORDER BY lower(name), id")).bind(&org).fetch_all(&st.postgres).await?;
    let today = Utc::now().date_naive();
    let this_month = today.year() as i64 * 100 + today.month() as i64;
    let mut out = Vec::new();
    for b in rows {
        let covers = (b.start_month as i64..=b.end_month as i64).contains(&this_month);
        let (mut amount, mut actual, mut forecast) = (0.0, 0.0, 0.0);
        if covers {
            let custom = custom_forecast_entries(&st, &org, &b.id).await.map_err(app_error)?;
            let entries = entry_costs(&st, &org, &b, &custom).await.map_err(app_error)?;
            for (e, c) in entries.iter().filter(|(e, _)| e.month == this_month) {
                amount += e.amount;
                if let Some(c) = c {
                    actual += c.actual.unwrap_or_default();
                    forecast += c.custom_forecast.or(c.forecast).unwrap_or_default();
                }
            }
        }
        let total_amount = entries_of(&b.entries).iter().map(|e| e.amount).sum();
        out.push(CostBudgetSummary {
            id: b.id,
            name: b.name,
            metrics_query: b.metrics_query,
            start_month: b.start_month,
            end_month: b.end_month,
            total_amount,
            month: covers.then_some(this_month),
            month_amount: amount,
            month_actual: (actual * 100.0).round() / 100.0,
            month_forecast: (forecast * 100.0).round() / 100.0,
        });
    }
    Ok(axum::Json(out))
}

/// The Cloud Cost pages' errors as the app's (`{"error": "…"}`).
fn app_error(e: DdError) -> ApiError {
    let message = e.errors.join("; ");
    match e.status {
        StatusCode::NOT_FOUND => ApiError::NotFound(message),
        StatusCode::FORBIDDEN => ApiError::Forbidden(message),
        StatusCode::BAD_REQUEST => ApiError::BadRequest(message),
        _ => ApiError::Internal(anyhow::anyhow!(message)),
    }
}

/// A budget with its entries and what each month cost so far and is forecast to cost, as Datadog's
/// API answers it (`GET /api/v2/cost/budget/{id}?actual=true&forecast=true`), for Plan → Budgets.
#[utoipa::path(get, path = "/api/v1/cloud-cost/budgets/{budget_id}", operation_id = "cloud_cost_budget", params(("budget_id" = String, Path)),
    responses((status = 200, body = Value), (status = 404)), tag = "cloud-cost")]
pub async fn ui_budget(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> ApiResult<Value> {
    let b = find_budget(&st, &org, &id).await.map_err(app_error)?.ok_or_else(|| ApiError::NotFound("budget not found".into()))?;
    let custom = custom_forecast_entries(&st, &org, &b.id).await.map_err(app_error)?;
    let entries = entry_costs(&st, &org, &b, &custom).await.map_err(app_error)?;
    Ok(axum::Json(json!({ "data": budget_doc(&b, Some(&entries)) })))
}

/// Create or replace a budget from Plan → Budgets: the same document and checks as
/// `PUT /api/v2/cost/budget`, so Terraform and the page manage the same budgets.
#[utoipa::path(put, path = "/api/v1/cloud-cost/budgets", operation_id = "cloud_cost_save_budget", request_body = Value,
    responses((status = 200, body = Value), (status = 400), (status = 404)), tag = "cloud-cost")]
pub async fn ui_save_budget(State(st): State<AppState>, Writer(ctx): Writer, axum::Json(doc): axum::Json<Value>) -> ApiResult<Value> {
    let id = save_budget(&st, &ctx.org_id, &ctx.user_id, &doc).await.map_err(app_error)?;
    let b = find_budget(&st, &ctx.org_id, &id).await.map_err(app_error)?.ok_or_else(|| ApiError::NotFound("budget not found".into()))?;
    let entries: Vec<(Entry, Option<Costs>)> = entries_of(&b.entries).into_iter().map(|e| (e, None)).collect();
    Ok(axum::Json(json!({ "data": budget_doc(&b, Some(&entries)) })))
}

#[utoipa::path(delete, path = "/api/v1/cloud-cost/budgets/{budget_id}", operation_id = "cloud_cost_delete_budget", params(("budget_id" = String, Path)),
    responses((status = 204), (status = 404)), tag = "cloud-cost")]
pub async fn ui_delete_budget(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    remove_budget(&st, &ctx.org_id, &id).await.map_err(app_error)
}

// ================================================================ custom forecasts

#[derive(sqlx::FromRow)]
struct ForecastRow {
    id: String,
    budget_id: String,
    entries: Value,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const FORECAST_COLS: &str = "id::text AS id, budget_id::text AS budget_id, entries, created_by::text AS created_by, updated_by::text AS updated_by, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

async fn find_forecast(st: &AppState, org: &str, budget: &str) -> Result<Option<ForecastRow>, DdError> {
    let Some(budget) = uuid(budget) else { return Ok(None) };
    Ok(sqlx::query_as(&format!("SELECT {FORECAST_COLS} FROM cost_custom_forecasts WHERE org_id = $1 AND budget_id = $2::uuid")).bind(org).bind(&budget).fetch_optional(&st.postgres).await?)
}

async fn custom_forecast_entries(st: &AppState, org: &str, budget: &str) -> Result<Vec<Entry>, DdError> {
    Ok(find_forecast(st, org, budget).await?.map(|f| entries_of(&f.entries)).unwrap_or_default())
}

fn forecast_doc(f: &ForecastRow) -> Value {
    json!({ "data": { "type": "custom_forecast", "id": f.id, "attributes": {
        "budget_uid": f.budget_id, "entries": entries_json(&entries_of(&f.entries)),
        "created_at": f.created_at, "created_by": f.created_by.clone().unwrap_or_default(),
        "updated_at": f.updated_at, "updated_by": f.updated_by.clone().unwrap_or_default(),
    } } })
}

/// Set a budget's custom forecast (Datadog-compatible): overrides of the forecast for some of its entries.
#[utoipa::path(put, path = "/api/v2/cost/budget/custom-forecast", operation_id = "datadog_upsert_custom_forecast", request_body = Value,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn upsert_custom_forecast(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let doc = body(&raw)?;
    let d = data(&doc)?;
    if d.get("type").and_then(Value::as_str) != Some("custom_forecast") {
        return Err(bad_request("`data.type` must be \"custom_forecast\""));
    }
    let attrs = d.get("attributes").cloned().unwrap_or(Value::Null);
    let budget_id = attrs.get("budget_uid").and_then(Value::as_str).unwrap_or_default();
    let b = find_budget(&st, &ctx.org_id, budget_id).await?.ok_or_else(|| not_found("budget not found"))?;
    let budget_entries = entries_of(&b.entries);
    let Some(items) = attrs.get("entries").and_then(Value::as_array) else { return Err(bad_request("`entries` must be a list")) };
    let mut entries = Vec::new();
    for (i, e) in items.iter().enumerate() {
        let month = e.get("month").and_then(Value::as_i64).unwrap_or_default();
        let amount = e.get("amount").and_then(Value::as_f64).filter(|a| *a >= 0.0 && a.is_finite()).ok_or_else(|| bad_request(format!("entry {i}: `amount` must be a number of at least 0")))?;
        let mut filters = filters_of(e.get("tag_filters").unwrap_or(&Value::Null)).map_err(|err| bad_request(format!("entry {i}: {err}")))?;
        filters.sort();
        let exists = budget_entries.iter().any(|be| {
            let mut f = be.filters.clone();
            f.sort();
            be.month == month && f == filters
        });
        if !exists {
            return Err(bad_request(format!("entry {i}: the budget has no entry for month {month} with these tag filters")));
        }
        entries.push(Entry { month, amount, filters });
    }
    sqlx::query(
        "INSERT INTO cost_custom_forecasts (org_id, budget_id, entries, created_by, updated_by) VALUES ($1, $2::uuid, $3, $4::uuid, $4::uuid) \
         ON CONFLICT (org_id, budget_id) DO UPDATE SET entries = EXCLUDED.entries, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&b.id)
    .bind(entries_json(&entries))
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    let f = find_forecast(&st, &ctx.org_id, &b.id).await?.ok_or_else(|| not_found("Not Found"))?;
    ok(forecast_doc(&f))
}

/// Get a budget's custom forecast (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/cost/budget/{budget_id}/custom-forecast", operation_id = "datadog_get_custom_forecast", params(("budget_id" = String, Path)),
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_custom_forecast(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(budget): Path<String>) -> DocResult {
    let f = find_forecast(&st, &ctx.org_id, &budget).await?.ok_or_else(|| not_found("Not Found"))?;
    ok(forecast_doc(&f))
}

/// Remove a budget's custom forecast (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/cost/budget/{budget_id}/custom-forecast", operation_id = "datadog_delete_custom_forecast", params(("budget_id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_custom_forecast(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(budget): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Standard)?;
    let budget = uuid(&budget).ok_or_else(|| not_found("Not Found"))?;
    let deleted = sqlx::query("DELETE FROM cost_custom_forecasts WHERE org_id = $1 AND budget_id = $2::uuid").bind(&ctx.org_id).bind(&budget).execute(&st.postgres).await?;
    if deleted.rows_affected() == 0 {
        return Err(not_found("Not Found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ custom costs files

/// LineNo of an item's days: item × this + day.
const ITEM_SPAN: u32 = MAX_PERIOD_DAYS as u32 + 1;

#[derive(Debug, PartialEq)]
struct LineItem {
    provider: String,
    description: String,
    start: NaiveDate,
    /// Exclusive.
    end: NaiveDate,
    billed: f64,
    currency: String,
    tags: BTreeMap<String, String>,
}

fn charge_day(v: Option<&Value>) -> Option<NaiveDate> {
    let s = v?.as_str()?.trim();
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

/// The file's line items, or every problem found ("errors in object 3: …").
fn parse_line_items(v: &Value) -> Result<Vec<LineItem>, Vec<String>> {
    let Some(items) = v.as_array() else { return Err(vec!["the body must be a list of line items".into()]) };
    if items.is_empty() {
        return Err(vec!["the file has no line items".into()]);
    }
    if items.len() > MAX_LINE_ITEMS {
        return Err(vec![format!("a file can hold at most {MAX_LINE_ITEMS} line items")]);
    }
    let mut errors = Vec::new();
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let mut problems = Vec::new();
        let text = |key: &str, problems: &mut Vec<String>| match item.get(key).and_then(Value::as_str).map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                problems.push(format!("{key} is required"));
                String::new()
            }
        };
        let provider = text("ProviderName", &mut problems);
        let description = text("ChargeDescription", &mut problems);
        let currency = text("BillingCurrency", &mut problems);
        if !currency.is_empty() && currency != "USD" {
            problems.push("BillingCurrency must be USD".into());
        }
        let billed = item.get("BilledCost").and_then(Value::as_f64).filter(|c| c.is_finite());
        if billed.is_none() {
            problems.push("BilledCost must be a number".into());
        }
        let start = charge_day(item.get("ChargePeriodStart"));
        let end = charge_day(item.get("ChargePeriodEnd"));
        if start.is_none() {
            problems.push("ChargePeriodStart must be a date (YYYY-MM-DD)".into());
        }
        if end.is_none() {
            problems.push("ChargePeriodEnd must be a date (YYYY-MM-DD)".into());
        }
        let (start, end) = (start.unwrap_or_default(), end.unwrap_or_default());
        // A period ending where it starts is that one day.
        let end = if end <= start { start + chrono::Duration::days(1) } else { end };
        if (end - start).num_days() > MAX_PERIOD_DAYS {
            problems.push(format!("a charge period can't exceed {MAX_PERIOD_DAYS} days"));
        }
        let tags: BTreeMap<String, String> = match item.get("Tags") {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(Value::Object(m)) => m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())).or_else(|| (!v.is_null()).then(|| (k.clone(), v.to_string())))).collect(),
            Some(_) => {
                problems.push("Tags must be an object of strings".into());
                BTreeMap::new()
            }
        };
        if problems.is_empty() {
            out.push(LineItem { provider, description, start, end, billed: billed.unwrap_or_default(), currency, tags });
        } else {
            errors.push(format!("errors in object {i}: {}", problems.join("; ")));
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        errors.truncate(20);
        Err(errors)
    }
}

/// An item's cost spread over its days.
fn item_lines(org: &str, file_id: &str, index: usize, item: &LineItem) -> Vec<CostLine> {
    let days = (item.end - item.start).num_days().max(1);
    let per_day = item.billed / days as f64;
    let service = ["ServiceName", "servicename", "service_name"].iter().find_map(|k| item.tags.get(*k)).cloned().unwrap_or_else(|| item.description.clone());
    (0..days)
        .map(|d| CostLine {
            org: org.to_string(),
            file_id: file_id.to_string(),
            line_no: index as u32 * ITEM_SPAN + d as u32,
            date: (item.start + chrono::Duration::days(d)).to_string(),
            provider: item.provider.clone(),
            service: service.clone(),
            sub_account: item.tags.get("SubAccountName").or_else(|| item.tags.get("subaccountname")).cloned().unwrap_or_default(),
            region: item.tags.get("RegionName").or_else(|| item.tags.get("regionname")).cloned().unwrap_or_default(),
            zone: String::new(),
            category: "usage".into(),
            description: item.description.clone(),
            resource_id: String::new(),
            pricing_unit: String::new(),
            quantity: 0.0,
            billed: per_day,
            effective: per_day,
            list: per_day,
            pricing_category: String::new(),
            commitment_id: String::new(),
            commitment_type: String::new(),
            commitment_status: String::new(),
            currency: item.currency.clone(),
            tags: item.tags.clone(),
            source: "custom".into(),
            version: 1,
            deleted: 0,
        })
        .collect()
}

#[derive(sqlx::FromRow)]
struct FileRow {
    id: String,
    name: String,
    provider_names: Vec<String>,
    billed_cost: f64,
    currency: String,
    period_start: i64,
    period_end: i64,
    status: String,
    uploaded_at: i64,
    uploader_name: Option<String>,
    uploader_email: Option<String>,
}

const FILE_COLS: &str = "f.id::text AS id, f.name, f.provider_names, f.billed_cost, f.currency, f.period_start, f.period_end, f.status, \
    (extract(epoch FROM f.uploaded_at) * 1000)::bigint AS uploaded_at, u.name AS uploader_name, u.email AS uploader_email";
const FILE_FROM: &str = "cost_custom_files f LEFT JOIN users u ON u.id = f.uploaded_by AND u.org_id = f.org_id";

fn file_doc(f: &FileRow, content: Option<Value>) -> Value {
    let mut attrs = json!({
        "name": f.name, "provider_names": f.provider_names, "billed_cost": f.billed_cost, "billing_currency": f.currency,
        "charge_period": { "start": f.period_start, "end": f.period_end }, "status": f.status.to_uppercase(), "uploaded_at": f.uploaded_at,
        "uploaded_by": { "name": f.uploader_name.clone().unwrap_or_default(), "email": f.uploader_email.clone().unwrap_or_default(), "icon": "" },
    });
    if let Some(c) = content {
        attrs["content"] = c;
    }
    json!({ "type": "cost_metadata", "id": f.id, "attributes": attrs })
}

async fn find_file(st: &AppState, org: &str, id: &str) -> Result<Option<FileRow>, DdError> {
    let Some(id) = uuid(id) else { return Ok(None) };
    Ok(sqlx::query_as(&format!("SELECT {FILE_COLS} FROM {FILE_FROM} WHERE f.org_id = $1 AND f.id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?)
}

/// Upload a custom costs file (Datadog-compatible): FOCUS line items as JSON.
#[utoipa::path(put, path = "/api/v2/cost/custom_costs", operation_id = "datadog_upload_custom_costs_file", request_body = Value,
    responses((status = 202, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn upload_custom_costs(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let items = parse_line_items(&body(&raw)?).map_err(|errors| DdError { status: StatusCode::BAD_REQUEST, errors })?;
    let providers: Vec<String> = items.iter().map(|i| i.provider.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    let start = items.iter().map(|i| i.start).min().unwrap_or_default();
    let end = items.iter().map(|i| i.end).max().unwrap_or_default();
    let ms = |d: NaiveDate| d.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc().timestamp_millis();
    let billed: f64 = items.iter().map(|i| i.billed).sum();
    let mut tx = st.postgres.begin().await?;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO cost_custom_files (org_id, name, provider_names, billed_cost, period_start, period_end, line_items, uploaded_by) \
         VALUES ($1, 'data.json', $2, $3, $4, $5, $6, $7::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&providers)
    .bind(billed)
    .bind(ms(start))
    .bind(ms(end))
    .bind(items.len() as i32)
    .bind(&ctx.user_id)
    .fetch_one(&mut *tx)
    .await?;
    let lines: Vec<CostLine> = items.iter().enumerate().flat_map(|(i, item)| item_lines(&ctx.org_id, &id, i, item)).collect();
    cloud_cost::insert(&st, &lines).await?;
    tx.commit().await?;
    let f = find_file(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found("Not Found"))?;
    Ok((StatusCode::ACCEPTED, axum::Json(json!({ "data": file_doc(&f, None), "meta": { "version": "1.0" } }))))
}

/// List custom costs files (Datadog-compatible): paging, status, name and provider filters, sort.
#[utoipa::path(get, path = "/api/v2/cost/custom_costs", operation_id = "datadog_list_custom_costs_files", responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_custom_costs(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<Vec<(String, String)>>) -> DocResult {
    let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
    let number: i64 = get("page[number]").map(|v| v.parse().map_err(|_| bad_request("page[number] must be a number"))).transpose()?.unwrap_or(0).max(0);
    let size: i64 = get("page[size]").map(|v| v.parse().map_err(|_| bad_request("page[size] must be a number"))).transpose()?.unwrap_or(100).clamp(1, 1000);
    let status = get("filter[status]").map(|s| s.to_lowercase());
    let name = get("filter[name]").map(|s| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
    let providers: Vec<String> = q.iter().filter(|(k, _)| k == "filter[provider]").flat_map(|(_, v)| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>()).collect();
    let order = match get("sort").as_deref().unwrap_or("-uploaded_at") {
        "name" => "f.name, f.id",
        "-name" => "f.name DESC, f.id",
        "uploaded_at" | "created_at" => "f.uploaded_at, f.id",
        "-uploaded_at" | "-created_at" => "f.uploaded_at DESC, f.id",
        "billed_cost" => "f.billed_cost, f.id",
        "-billed_cost" => "f.billed_cost DESC, f.id",
        other => return Err(bad_request(format!("unknown sort '{other}'"))),
    };
    let filter = "f.org_id = $1 AND ($2::text IS NULL OR f.status = $2) AND ($3::text IS NULL OR f.name ILIKE $3) AND (cardinality($4::text[]) = 0 OR f.provider_names && $4)";
    let rows: Vec<FileRow> = sqlx::query_as(&format!("SELECT {FILE_COLS} FROM {FILE_FROM} WHERE {filter} ORDER BY {order} LIMIT $5 OFFSET $6"))
        .bind(&ctx.org_id)
        .bind(&status)
        .bind(&name)
        .bind(&providers)
        .bind(size)
        .bind(number * size)
        .fetch_all(&st.postgres)
        .await?;
    let (total,): (i64,) = sqlx::query_as(&format!("SELECT count(*) FROM {FILE_FROM} WHERE {filter}")).bind(&ctx.org_id).bind(&status).bind(&name).bind(&providers).fetch_one(&st.postgres).await?;
    let counts: Vec<(String, i64)> = sqlx::query_as("SELECT upper(status), count(*) FROM cost_custom_files WHERE org_id = $1 GROUP BY 1").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let (all_providers,): (Vec<String>,) =
        sqlx::query_as("SELECT COALESCE(array_agg(DISTINCT p ORDER BY p), '{}') FROM cost_custom_files, unnest(provider_names) p WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    ok(json!({
        "data": rows.iter().map(|f| file_doc(f, None)).collect::<Vec<_>>(),
        "meta": { "count_by_status": counts.into_iter().collect::<BTreeMap<_, _>>(), "providers": all_providers, "total_filtered_count": total, "version": "1.0" },
    }))
}

/// Get a custom costs file with its line items (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/cost/custom_costs/{file_id}", operation_id = "datadog_get_custom_costs_file", params(("file_id" = String, Path)),
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_custom_costs(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    let f = find_file(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found("Not Found"))?;
    let lines = cloud_cost::file_lines(&st, &ctx.org_id, &f.id).await?;
    ok(json!({ "data": file_doc(&f, Some(content(lines))), "meta": { "version": "1.0" } }))
}

/// The line items again, from their days.
fn content(lines: Vec<cloud_cost::StoredLine>) -> Value {
    let mut items: BTreeMap<u32, (cloud_cost::StoredLine, f64, String, String)> = BTreeMap::new();
    for l in lines {
        let item = l.line_no / ITEM_SPAN;
        match items.get_mut(&item) {
            Some((_, billed, first, last)) => {
                *billed += l.billed;
                if l.date < *first {
                    *first = l.date.clone();
                }
                if l.date > *last {
                    *last = l.date.clone();
                }
            }
            None => {
                let (billed, date) = (l.billed, l.date.clone());
                items.insert(item, (l, billed, date.clone(), date));
            }
        }
    }
    Value::Array(
        items
            .into_values()
            .map(|(l, billed, first, last)| {
                let end = NaiveDate::parse_from_str(&last, "%Y-%m-%d").map(|d| (d + chrono::Duration::days(1)).to_string()).unwrap_or(last);
                json!({
                    "ProviderName": l.provider, "ChargeDescription": l.description, "BilledCost": (billed * 1e6).round() / 1e6,
                    "BillingCurrency": l.currency, "ChargePeriodStart": first, "ChargePeriodEnd": end,
                    "Tags": l.tags.into_iter().collect::<BTreeMap<_, _>>(),
                })
            })
            .collect(),
    )
}

/// Delete a custom costs file and its costs (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/cost/custom_costs/{file_id}", operation_id = "datadog_delete_custom_costs_file", params(("file_id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_custom_costs(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Standard)?;
    let f = find_file(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found("Not Found"))?;
    cloud_cost::delete_file(&st, &ctx.org_id, &f.id).await?;
    sqlx::query("DELETE FROM cost_custom_files WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&f.id).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(entries: Value) -> Value {
        json!({ "name": "my budget", "metrics_query": "aws.cost.amortized{service:ec2} by {service}", "start_month": 202501, "end_month": 202502, "entries": entries })
    }

    #[test]
    fn a_budget_as_datadogs_client_sends_it() {
        let b = parse_budget(&budget(json!([
            { "amount": 500, "month": 202501, "tag_filters": [{ "tag_key": "service", "tag_value": "ec2" }] },
            { "amount": 500, "month": 202502, "tag_filters": [{ "tag_key": "service", "tag_value": "ec2" }] },
        ])))
        .unwrap();
        assert_eq!(b.entries.len(), 2);
        assert_eq!(entries_json(&b.entries)[0], json!({ "month": 202501, "amount": 500.0, "tag_filters": [{ "tag_key": "service", "tag_value": "ec2" }] }));
    }

    #[test]
    fn budget_entries_cover_every_month_with_the_querys_tags() {
        let errors = parse_budget(&budget(json!([{ "amount": 500, "month": 202501, "tag_filters": [{ "tag_key": "service", "tag_value": "ec2" }] }]))).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("every month")), "{errors:?}");
        let errors = parse_budget(&budget(json!([
            { "amount": 1, "month": 202501, "tag_filters": [{ "tag_key": "team", "tag_value": "ml" }] },
            { "amount": 1, "month": 202502, "tag_filters": [{ "tag_key": "team", "tag_value": "ml" }] },
        ])))
        .unwrap_err();
        assert!(errors.iter().any(|e| e.contains("by {")), "{errors:?}");
        let errors = parse_budget(&json!({ "name": "", "metrics_query": "system.cpu.user{*}", "start_month": 202513, "end_month": 202501 })).unwrap_err();
        assert!(errors.len() >= 3, "{errors:?}");
        assert!(parse_budget(&json!({ "name": "n", "metrics_query": "all.cost{*}", "start_month": 202601, "end_month": 202603, "entries": [
            { "amount": 1, "month": 202601 }, { "amount": 1, "month": 202602 }, { "amount": 1, "month": 202603 },
        ] }))
        .is_ok());
    }

    #[test]
    fn custom_costs_spread_over_their_charge_period() {
        let items = parse_line_items(&json!([{ "BilledCost": 250, "BillingCurrency": "USD", "ChargeDescription": "my_description", "ChargePeriodEnd": "2023-06-06", "ChargePeriodStart": "2023-05-06", "ProviderName": "my_provider", "Tags": { "key": "value" } }])).unwrap();
        let lines = item_lines("o", "f", 2, &items[0]);
        assert_eq!(lines.len(), 31);
        assert_eq!(lines[0].date, "2023-05-06");
        assert_eq!(lines[30].date, "2023-06-05");
        assert_eq!(lines[0].line_no, 2 * ITEM_SPAN);
        assert!((lines.iter().map(|l| l.billed).sum::<f64>() - 250.0).abs() < 1e-9);
        assert_eq!(lines[0].service, "my_description");
        let stored = lines
            .iter()
            .map(|l| cloud_cost::StoredLine { date: l.date.clone(), provider: l.provider.clone(), description: l.description.clone(), billed: l.billed, currency: l.currency.clone(), tags: l.tags.clone().into_iter().collect(), line_no: l.line_no })
            .collect();
        let c = content(stored);
        assert_eq!(c[0]["ChargePeriodStart"], "2023-05-06");
        assert_eq!(c[0]["ChargePeriodEnd"], "2023-06-06");
        assert_eq!(c[0]["BilledCost"], 250.0);
    }

    #[test]
    fn invalid_line_items_are_all_reported() {
        let errors = parse_line_items(&json!([{ "BilledCost": "x" }, { "ProviderName": "p", "ChargeDescription": "d", "BillingCurrency": "USD", "BilledCost": 1, "ChargePeriodStart": "2023-01-01", "ChargePeriodEnd": "2023-01-02" }])).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("errors in object 0:"));
        assert!(parse_line_items(&json!({})).is_err());
    }

    #[tokio::test]
    async fn replays_datadogs_cost_scenarios() {
        use crate::routes::datadog_api::replay::{Feature, TestServer};
        let Some(f) = Feature::load("v2/cloud_cost_management") else { return };
        let Some(s) = TestServer::start("dd-cost-replay").await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        // Budget upserts are recorded under the scenarios' earlier names.
        for name in ["Update if exists or create a new budget returns \"OK\" response", "Update if exists or create a new budget returns \"Bad Request\" response", "Update if exists or create a new budget returns \"Not Found\" response"] {
            let Some(interactions) = f.cassette_named(name) else { continue };
            let sends = f.sends(&f.scenarios[0]);
            let (_, p) = s.replayer().replay_interactions(sends, name, &interactions).await;
            problems.extend(p);
            replayed += 1;
        }
        // Datadog's numeric org id: ours is a name, and no client reads it back.
        problems.retain(|m| !m.contains("`data.attributes.org_id`") && !m.contains("`data[].attributes.org_id`"));
        assert!(replayed >= 10, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[tokio::test]
    async fn the_pages_and_terraform_share_budgets_and_read_commitments() {
        use crate::routes::datadog_api::replay::TestServer;
        let Some(s) = TestServer::start("dd-cost-pages").await else { return };
        cloud_cost::seed_demo(&s.st, &s.org).await.expect("seed");
        let http = reqwest::Client::new();
        let auth = format!("Bearer {}", s.app_key);
        // Demo billing covers completed days only. On the first of a month, today's month
        // has no billed rows yet; choose a month containing a seeded day instead.
        let billed_day = Utc::now().date_naive() - chrono::Duration::days(1);
        let month = billed_day.year() as i64 * 100 + billed_day.month() as i64;
        let doc = json!({ "data": { "type": "budget", "attributes": {
            "name": "Platform", "metrics_query": "aws.cost.amortized{*} by {subaccountname}", "start_month": month, "end_month": month,
            "entries": [{ "month": month, "amount": 1000, "tag_filters": [{ "tag_key": "subaccountname", "tag_value": "ecommerce-prod" }] }],
        } } });
        let res = http.put(format!("{}/api/v1/cloud-cost/budgets", s.base)).header("authorization", &auth).json(&doc).send().await.unwrap();
        let status = res.status();
        let text = res.text().await.unwrap();
        assert_eq!(status, 200, "{text}");
        let saved: Value = serde_json::from_str(&text).unwrap();
        let id = saved["data"]["id"].as_str().expect("id").to_string();
        // Terraform reads the budget the page saved.
        let res = http.get(format!("{}/api/v2/cost/budget/{id}", s.base)).header("dd-application-key", &s.app_key).header("dd-api-key", &s.api_key).send().await.unwrap();
        let (status, text) = (res.status(), res.text().await.unwrap());
        assert_eq!(status, 200, "{text}");
        let dd: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(dd["data"]["attributes"]["name"], "Platform");
        let page: Value = http.get(format!("{}/api/v1/cloud-cost/budgets/{id}", s.base)).header("authorization", &auth).send().await.unwrap().json().await.unwrap();
        assert!(page["data"]["attributes"]["entries"][0]["costs"]["actual"].as_f64().unwrap_or_default() > 0.0, "{page}");
        // The same checks as Datadog's API: tag filters must match the query's `by`.
        let mut bad = doc.clone();
        bad["data"]["attributes"]["entries"][0]["tag_filters"][0]["tag_key"] = json!("team");
        let res = http.put(format!("{}/api/v1/cloud-cost/budgets", s.base)).header("authorization", &auth).json(&bad).send().await.unwrap();
        assert_eq!(res.status(), 400);
        assert!(res.text().await.unwrap().contains("tag filters"));

        let range = json!({ "from": (Utc::now() - chrono::Duration::days(30)).timestamp_millis(), "to": Utc::now().timestamp_millis() });
        let c: Value = http.post(format!("{}/api/v1/cloud-cost/commitments", s.base)).header("authorization", &auth).json(&json!({ "provider": "aws", "range": range })).send().await.unwrap().json().await.unwrap();
        let esr = c["effectiveSavingsRate"].as_f64().expect("savings rate");
        assert!(esr > 0.0 && esr < 0.5, "{esr}");
        assert_eq!(c["commitments"].as_array().unwrap().len(), 2, "{c}");
        assert!(c["coverage"].as_f64().unwrap() > 0.0);
        let u: Value = http
            .post(format!("{}/api/v1/cloud-cost/unit-costs", s.base))
            .header("authorization", &auth)
            .json(&json!({ "query": "providername:aws servicename:AmazonEC2 pricingunit:Hrs", "range": range, "groupBy": ["aws_instance_type"] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let m6i = u.as_array().unwrap().iter().find(|r| r["tags"]["aws_instance_type"] == "m6i.xlarge").expect("m6i");
        let hourly = m6i["amortizedCost"].as_f64().unwrap() / m6i["usageQuantity"].as_f64().unwrap();
        // The Savings Plan brings the on-demand 0.192/h down.
        assert!(hourly < 0.192 && hourly > 0.13, "{hourly}");

        let res = http.delete(format!("{}/api/v1/cloud-cost/budgets/{id}", s.base)).header("authorization", &auth).send().await.unwrap();
        assert_eq!(res.status(), 204);
        let res = http.get(format!("{}/api/v2/cost/budget/{id}", s.base)).header("dd-application-key", &s.app_key).header("dd-api-key", &s.api_key).send().await.unwrap();
        assert_eq!(res.status(), 404);
    }

    #[test]
    fn months_of_a_budget() {
        assert_eq!(months(202511, 202602), vec![202511, 202512, 202601, 202602]);
        assert!(months(202513, 202601).is_empty());
    }
}
