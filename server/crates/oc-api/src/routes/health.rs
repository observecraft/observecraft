use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct Readiness {
    pub clickhouse: bool,
    pub postgres: bool,
    pub nats: bool,
}

/// Liveness: the process is up.
#[utoipa::path(get, path = "/healthz", responses((status = 200, description = "ok")), tag = "health")]
pub async fn healthz() -> &'static str {
    "ok"
}

/// Readiness: every dependency answers. 503 when any is down.
#[utoipa::path(get, path = "/readyz", responses((status = 200, body = Readiness), (status = 503, body = Readiness)), tag = "health")]
pub async fn readyz(State(st): State<AppState>) -> (StatusCode, Json<Readiness>) {
    let (ch, pg) = tokio::join!(st.clickhouse.query("SELECT 1").execute(), sqlx::query("SELECT 1").execute(&st.postgres));
    let r = Readiness {
        clickhouse: ch.is_ok(),
        postgres: pg.is_ok(),
        nats: st.nats.connection_state() == async_nats::connection::State::Connected,
    };
    let code = if r.clickhouse && r.postgres && r.nats { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (code, Json(r))
}
