//! AI Credits Management and Usage (Agent Console): what Assist's chat and runs used of the org's model,
//! per application, member and day, with the org's limits (1 credit = 1,000 tokens) enforced before each
//! chat turn and run.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::routes::logs::TimeRange;
use crate::state::AppState;

/// Tokens per credit.
const TOKENS_PER_CREDIT: i64 = 1000;

/// Every use of the model: chat messages and runs, as (application, member, time, input, output).
const USES: &str = "SELECT 'chat'::text AS app, user_id AS user_id, at, input_tokens, output_tokens FROM assist_usage WHERE org_id = $1 \
    UNION ALL SELECT kind, run_as, created_at, input_tokens, output_tokens FROM assist_runs WHERE org_id = $1";

fn credits(tokens: i64) -> i64 {
    (tokens + TOKENS_PER_CREDIT - 1) / TOKENS_PER_CREDIT
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CreditSettings {
    /// Credits per calendar month (UTC); none: unlimited.
    pub monthly_limit: Option<i64>,
    /// Credits per member per day (UTC); none: unlimited.
    pub user_daily_limit: Option<i64>,
    /// USD per million input tokens, to estimate cost.
    pub input_price_per_mtok: Option<f64>,
    pub output_price_per_mtok: Option<f64>,
}

async fn settings(st: &AppState, org: &str) -> Result<CreditSettings, ApiError> {
    Ok(sqlx::query_as::<_, CreditSettings>("SELECT monthly_limit, user_daily_limit, input_price_per_mtok, output_price_per_mtok FROM assist_credits WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?
        .unwrap_or_default())
}

/// Credits used this month by the org and today by `user`.
async fn used(st: &AppState, org: &str, user: Option<&str>) -> Result<(i64, i64), ApiError> {
    let (month, today): (i64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT COALESCE(sum(input_tokens + output_tokens) FILTER (WHERE at >= date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'), 0)::bigint, \
         COALESCE(sum(input_tokens + output_tokens) FILTER (WHERE user_id::text = $2 AND at >= date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'), 0)::bigint \
         FROM ({USES}) u"
    )))
    .bind(org)
    .bind(user)
    .fetch_one(&st.postgres)
    .await?;
    Ok((credits(month), credits(today)))
}

/// Refuses new work once the org's monthly credits or the member's daily credits are used up.
pub async fn check_credits(st: &AppState, org: &str, user: Option<&str>) -> Result<(), ApiError> {
    let s = settings(st, org).await?;
    if s.monthly_limit.is_none() && (s.user_daily_limit.is_none() || user.is_none()) {
        return Ok(());
    }
    let (month, today) = used(st, org, user).await?;
    if let Some(limit) = s.monthly_limit.filter(|l| month >= *l) {
        return Err(ApiError::TooManyRequests(format!("This organization used its {limit} Assist credits for the month; an admin can raise the limit in AI Credits Management")));
    }
    if let (Some(limit), Some(_)) = (s.user_daily_limit, user) {
        if today >= limit {
            return Err(ApiError::TooManyRequests(format!("You used your {limit} daily Assist credits; they renew at midnight UTC")));
        }
    }
    Ok(())
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreditsView {
    #[serde(flatten)]
    pub settings: CreditSettings,
    pub used_this_month: i64,
    pub my_used_today: i64,
    pub tokens_per_credit: i64,
}

#[utoipa::path(get, path = "/api/v1/assist/credits", operation_id = "assist_get_credits", responses((status = 200, body = CreditsView)), tag = "assist")]
pub async fn get_credits(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<CreditsView> {
    let settings = settings(&st, &ctx.org_id).await?;
    let (used_this_month, my_used_today) = used(&st, &ctx.org_id, Some(&ctx.user_id)).await?;
    Ok(Json(CreditsView { settings, used_this_month, my_used_today, tokens_per_credit: TOKENS_PER_CREDIT }))
}

#[utoipa::path(put, path = "/api/v1/assist/credits", operation_id = "assist_put_credits", request_body = CreditSettings, responses((status = 204)), tag = "assist")]
pub async fn put_credits(State(st): State<AppState>, Admin(ctx): Admin, Json(c): Json<CreditSettings>) -> Result<StatusCode, ApiError> {
    if c.monthly_limit.is_some_and(|l| l < 0) || c.user_daily_limit.is_some_and(|l| l < 0) {
        return Err(ApiError::BadRequest("Limits can't be negative".into()));
    }
    if c.input_price_per_mtok.is_some_and(|p| !(0.0..=10_000.0).contains(&p)) || c.output_price_per_mtok.is_some_and(|p| !(0.0..=10_000.0).contains(&p)) {
        return Err(ApiError::BadRequest("Prices are 0 to 10000 USD per million tokens".into()));
    }
    sqlx::query(
        "INSERT INTO assist_credits (org_id, monthly_limit, user_daily_limit, input_price_per_mtok, output_price_per_mtok, updated_by, updated_at) VALUES ($1, $2, $3, $4, $5, $6, now()) \
         ON CONFLICT (org_id) DO UPDATE SET monthly_limit = $2, user_daily_limit = $3, input_price_per_mtok = $4, output_price_per_mtok = $5, updated_by = $6, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(c.monthly_limit)
    .bind(c.user_daily_limit)
    .bind(c.input_price_per_mtok)
    .bind(c.output_price_per_mtok)
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await?;
    let limit = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_else(|| "unlimited".into());
    ctx.audit(&st, "assist.credits.update", "", &format!("monthly {}, per member per day {}", limit(c.monthly_limit), limit(c.user_daily_limit))).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageRequest {
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    /// chat | investigation | security | agent | code
    pub app: String,
    pub users: i64,
    pub uses: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub credits: i64,
    pub est_cost: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserUsage {
    pub user_id: String,
    pub name: String,
    pub uses: i64,
    pub credits: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DayUsage {
    pub at: i64,
    pub users: i64,
    pub credits: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    pub users: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub credits: i64,
    /// From the token prices in AI Credits Management; none when they aren't set.
    pub est_cost: Option<f64>,
    pub daily: Vec<DayUsage>,
    pub by_app: Vec<AppUsage>,
    pub by_user: Vec<UserUsage>,
}

#[utoipa::path(post, path = "/api/v1/assist/usage", operation_id = "assist_usage", request_body = UsageRequest, responses((status = 200, body = UsageReport)), tag = "assist")]
pub async fn usage(State(st): State<AppState>, ctx: AuthContext, Json(r): Json<UsageRequest>) -> ApiResult<UsageReport> {
    if r.range.to <= r.range.from {
        return Err(ApiError::BadRequest("The range ends before it starts".into()));
    }
    let prices = settings(&st, &ctx.org_id).await?;
    let cost = |input: i64, output: i64| match (prices.input_price_per_mtok, prices.output_price_per_mtok) {
        (Some(i), Some(o)) => Some((input as f64 * i + output as f64 * o) / 1_000_000.0),
        _ => None,
    };
    let within = format!("FROM ({USES}) u WHERE u.at >= to_timestamp($2::float8 / 1000) AND u.at < to_timestamp($3::float8 / 1000)");
    let q = |sql: String| sqlx::AssertSqlSafe(sql);
    let (users, input_tokens, output_tokens): (i64, i64, i64) =
        sqlx::query_as(q(format!("SELECT count(DISTINCT user_id), COALESCE(sum(input_tokens), 0)::bigint, COALESCE(sum(output_tokens), 0)::bigint {within}")))
            .bind(&ctx.org_id)
            .bind(r.range.from)
            .bind(r.range.to)
            .fetch_one(&st.postgres)
            .await?;
    let daily: Vec<(i64, i64, i64)> = sqlx::query_as(q(format!(
        "SELECT (extract(epoch FROM date_trunc('day', u.at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') * 1000)::bigint, count(DISTINCT user_id), COALESCE(sum(input_tokens + output_tokens), 0)::bigint {within} GROUP BY 1 ORDER BY 1"
    )))
    .bind(&ctx.org_id)
    .bind(r.range.from)
    .bind(r.range.to)
    .fetch_all(&st.postgres)
    .await?;
    let apps: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(q(format!(
        "SELECT app, count(DISTINCT user_id), count(*), COALESCE(sum(input_tokens), 0)::bigint, COALESCE(sum(output_tokens), 0)::bigint {within} GROUP BY 1 ORDER BY 1"
    )))
    .bind(&ctx.org_id)
    .bind(r.range.from)
    .bind(r.range.to)
    .fetch_all(&st.postgres)
    .await?;
    let by_user: Vec<(String, String, i64, i64)> = sqlx::query_as(q(format!(
        "SELECT u.user_id::text, COALESCE(m.name, 'Deleted member'), count(*), COALESCE(sum(u.input_tokens + u.output_tokens), 0)::bigint {within} AND u.user_id IS NOT NULL \
         GROUP BY 1, 2 ORDER BY 4 DESC LIMIT 50"
    ).replace(&format!("FROM ({USES}) u"), &format!("FROM ({USES}) u LEFT JOIN users m ON m.id = u.user_id"))))
    .bind(&ctx.org_id)
    .bind(r.range.from)
    .bind(r.range.to)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(UsageReport {
        users,
        input_tokens,
        output_tokens,
        credits: credits(input_tokens + output_tokens),
        est_cost: cost(input_tokens, output_tokens),
        daily: daily.into_iter().map(|(at, users, tokens)| DayUsage { at, users, credits: credits(tokens) }).collect(),
        by_app: apps.into_iter().map(|(app, users, uses, i, o)| AppUsage { app, users, uses, input_tokens: i, output_tokens: o, credits: credits(i + o), est_cost: cost(i, o) }).collect(),
        by_user: by_user.into_iter().map(|(user_id, name, uses, tokens)| UserUsage { user_id, name, uses, credits: credits(tokens) }).collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Usage adds up the chat and every kind of run; limits stop new work once reached. Skipped unless
    /// OC_TEST_POSTGRES_URL and OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn usage_adds_up_and_limits_hold() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = states[0].clone();
        let admin = crate::test_support::admin_of_new_org(&st, "credits").await;
        let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Test Admin') RETURNING id::text")
            .bind(&admin.org_id)
            .bind(format!("{}@example.com", admin.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let ctx = AuthContext { user_id, ..admin };
        sqlx::query("INSERT INTO assist_usage (org_id, user_id, conversation_id, surface, new_conversation, input_tokens, output_tokens) VALUES ($1, $2::uuid, gen_random_uuid(), 'chat', true, 1200, 300)")
            .bind(&ctx.org_id)
            .bind(&ctx.user_id)
            .execute(&st.postgres)
            .await
            .unwrap();
        sqlx::query("INSERT INTO assist_runs (org_id, kind, title, status, run_as, input_tokens, output_tokens) VALUES ($1, 'investigation', 'x', 'concluded', $2::uuid, 4000, 1000)")
            .bind(&ctx.org_id)
            .bind(&ctx.user_id)
            .execute(&st.postgres)
            .await
            .unwrap();

        // Unlimited until set.
        check_credits(&st, &ctx.org_id, Some(&ctx.user_id)).await.unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let u = usage(State(st.clone()), ctx.clone(), Json(UsageRequest { range: TimeRange { from: now - 86_400_000, to: now + 60_000 } })).await.unwrap().0;
        assert_eq!((u.users, u.input_tokens, u.output_tokens, u.credits), (1, 5200, 1300, 7));
        assert_eq!(u.est_cost, None);
        let apps: Vec<(&str, i64)> = u.by_app.iter().map(|a| (a.app.as_str(), a.credits)).collect();
        assert!(apps.contains(&("chat", 2)) && apps.contains(&("investigation", 5)), "{apps:?}");
        assert_eq!(u.by_user[0].name, "Test Admin");

        put_credits(State(st.clone()), Admin(ctx.clone()), Json(CreditSettings { monthly_limit: Some(100), user_daily_limit: Some(5), input_price_per_mtok: Some(5.0), output_price_per_mtok: Some(25.0) })).await.unwrap();
        // 7 credits today against 5 a day for this member.
        assert!(matches!(check_credits(&st, &ctx.org_id, Some(&ctx.user_id)).await, Err(ApiError::TooManyRequests(m)) if m.contains("daily")));
        // Background runs without a member only count against the month.
        check_credits(&st, &ctx.org_id, None).await.unwrap();
        let c = get_credits(State(st.clone()), ctx.clone()).await.unwrap().0;
        assert_eq!((c.used_this_month, c.my_used_today), (7, 7));
        let u = usage(State(st.clone()), ctx.clone(), Json(UsageRequest { range: TimeRange { from: now - 86_400_000, to: now + 60_000 } })).await.unwrap().0;
        // 5200 × $5 + 1300 × $25 per million.
        assert!((u.est_cost.unwrap() - 0.0585).abs() < 1e-9, "{:?}", u.est_cost);

        put_credits(State(st.clone()), Admin(ctx.clone()), Json(CreditSettings { monthly_limit: Some(6), user_daily_limit: None, input_price_per_mtok: None, output_price_per_mtok: None })).await.unwrap();
        assert!(matches!(check_credits(&st, &ctx.org_id, None).await, Err(ApiError::TooManyRequests(m)) if m.contains("month")));
    }
}
