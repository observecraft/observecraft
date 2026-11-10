//! Packages' connections in Postgres (`connector_connections`, `connector_stream_state`, `connector_request_budgets`,
//! migration 0102). Every query filters on the org; secrets are sealed one by one and only unsealed for a run or a
//! test.

use oc_connectors::engine::{Connection, StreamState};
use oc_connectors::manifest::{Budget, BudgetPeriod};
use serde_json::{Map, Value};

use crate::state::AppState;

/// A connection as stored (secrets sealed).
#[derive(Clone, Debug)]
pub struct Saved {
    pub id: String,
    pub name: String,
    pub config: Map<String, Value>,
    /// Field → sealed value.
    pub secrets: Map<String, Value>,
    pub streams: Vec<String>,
    pub enabled: bool,
    /// Epoch ms.
    pub updated_at: i64,
}

type Row = (String, String, Value, Value, Vec<String>, bool, i64);

const COLUMNS: &str = "id::text, name, config, secrets, streams, enabled, (extract(epoch FROM updated_at) * 1000)::bigint";

fn saved((id, name, config, secrets, streams, enabled, updated_at): Row) -> Saved {
    Saved { id, name, config: config.as_object().cloned().unwrap_or_default(), secrets: secrets.as_object().cloned().unwrap_or_default(), streams, enabled, updated_at }
}

/// The org's connections of a connector.
pub async fn list(st: &AppState, org: &str, connector: &str) -> Result<Vec<Saved>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM connector_connections WHERE org_id = $1 AND connector_id = $2 ORDER BY name")))
        .bind(org)
        .bind(connector)
        .fetch_all(&st.postgres)
        .await?;
    Ok(rows.into_iter().map(saved).collect())
}

/// One of the org's connections (None when the id isn't the org's, or isn't of this connector).
pub async fn get(st: &AppState, org: &str, connector: &str, id: &str) -> Result<Option<Saved>, sqlx::Error> {
    let Ok(uuid) = id.parse::<uuid_text::Uuid>() else { return Ok(None) };
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM connector_connections WHERE org_id = $1 AND connector_id = $2 AND id = $3::uuid")))
        .bind(org)
        .bind(connector)
        .bind(uuid.0)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.map(saved))
}

/// UUID text, checked before it reaches a `::uuid` cast (a bad id is "not found", not a database error).
mod uuid_text {
    pub struct Uuid(pub String);

    impl std::str::FromStr for Uuid {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, ()> {
            let ok = s.len() == 36 && s.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
            if ok {
                Ok(Uuid(s.to_ascii_lowercase()))
            } else {
                Err(())
            }
        }
    }
}

/// The connection with its secrets unsealed, for a run or a test.
pub fn unseal(st: &AppState, s: &Saved) -> Result<Connection, String> {
    let mut secrets = Map::new();
    for (k, v) in &s.secrets {
        let sealed = v.as_str().unwrap_or_default();
        let plain = st.secrets.decrypt(sealed).map_err(|_| format!("the saved {k} can't be read (the server's secret key changed): enter it again"))?;
        secrets.insert(k.clone(), Value::String(plain));
    }
    Ok(Connection { id: s.id.clone(), name: s.name.clone(), config: s.config.clone(), secrets })
}

/// Seals each secret.
pub fn seal(st: &AppState, secrets: &Map<String, Value>) -> Map<String, Value> {
    secrets.iter().filter_map(|(k, v)| v.as_str().map(|p| (k.clone(), Value::String(st.secrets.encrypt(p))))).collect()
}

pub async fn state(st: &AppState, org: &str, connection: &str, stream: &str) -> Result<StreamState, sqlx::Error> {
    let v: Option<Value> = sqlx::query_scalar("SELECT state FROM connector_stream_state WHERE org_id = $1 AND connection_id = $2::uuid AND stream = $3").bind(org).bind(connection).bind(stream).fetch_optional(&st.postgres).await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
}

pub async fn save_state(st: &AppState, org: &str, connection: &str, stream: &str, state: &StreamState) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO connector_stream_state (connection_id, org_id, stream, state) SELECT id, org_id, $3, $4 FROM connector_connections WHERE org_id = $1 AND id = $2::uuid \
         ON CONFLICT (connection_id, stream) DO UPDATE SET state = EXCLUDED.state, updated_at = now()",
    )
    .bind(org)
    .bind(connection)
    .bind(stream)
    .bind(serde_json::to_value(state).unwrap_or_default())
    .execute(&st.postgres)
    .await?;
    Ok(())
}

/// Saves a rotated secret (a refresh token the vendor replaced).
pub async fn rotate(st: &AppState, org: &str, connection: &str, field: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE connector_connections SET secrets = jsonb_set(secrets, ARRAY[$3], to_jsonb($4::text)) WHERE org_id = $1 AND id = $2::uuid")
        .bind(org)
        .bind(connection)
        .bind(field)
        .bind(st.secrets.encrypt(value))
        .execute(&st.postgres)
        .await?;
    Ok(())
}

fn period(p: BudgetPeriod) -> &'static str {
    match p {
        BudgetPeriod::Hour => "hour",
        BudgetPeriod::Day => "day",
        BudgetPeriod::Month => "month",
    }
}

/// Requests left in a quota's current period (`scope`: '' the connection's, else a stream's).
pub async fn budget_left(st: &AppState, org: &str, connection: &str, scope: &str, budget: &Budget) -> Result<u32, sqlx::Error> {
    let used: Option<i32> = sqlx::query_scalar(
        "SELECT used FROM connector_request_budgets WHERE org_id = $1 AND connection_id = $2::uuid AND scope = $3 AND period_start = date_trunc($4, now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'",
    )
    .bind(org)
    .bind(connection)
    .bind(scope)
    .bind(period(budget.per))
    .fetch_optional(&st.postgres)
    .await?;
    Ok(budget.requests.saturating_sub(used.unwrap_or(0).max(0) as u32))
}

/// Counts requests against a quota.
pub async fn budget_use(st: &AppState, org: &str, connection: &str, scope: &str, budget: &Budget, requests: u32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO connector_request_budgets (connection_id, org_id, scope, period_start, used) \
         SELECT id, org_id, $3, date_trunc($4, now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC', $5 FROM connector_connections WHERE org_id = $1 AND id = $2::uuid \
         ON CONFLICT (connection_id, scope) DO UPDATE SET used = CASE WHEN connector_request_budgets.period_start = EXCLUDED.period_start \
           THEN connector_request_budgets.used + EXCLUDED.used ELSE EXCLUDED.used END, period_start = EXCLUDED.period_start",
    )
    .bind(org)
    .bind(connection)
    .bind(scope)
    .bind(period(budget.per))
    .bind(requests as i32)
    .execute(&st.postgres)
    .await?;
    Ok(())
}
