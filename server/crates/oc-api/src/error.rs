use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

/// JSON error body returned by every endpoint.
#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    TooManyRequests(String),
    /// The resource changed since the client read it (optimistic concurrency).
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error(transparent)]
    Postgres(#[from] sqlx::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            ApiError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m.clone()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            // The org's query limits (org_limits.rs): busy is temporary, retry in a second.
            ApiError::ClickHouse(e) if crate::org_limits::is_org_busy(e) => {
                tracing::warn!("organization query slots busy");
                let body = Json(ErrorBody { error: crate::org_limits::ORG_BUSY.to_string() });
                return (StatusCode::TOO_MANY_REQUESTS, [(axum::http::header::RETRY_AFTER, "1")], body).into_response();
            }
            // Storage errors are logged in full but not echoed to clients.
            ApiError::ClickHouse(e) => {
                tracing::error!(error = %e, "clickhouse query failed");
                match limit_error(e) {
                    Some(answer) => answer,
                    None => (StatusCode::BAD_GATEWAY, "Query failed".to_string()),
                }
            }
            ApiError::Postgres(e) => {
                tracing::error!(error = %e, "postgres query failed");
                (StatusCode::BAD_GATEWAY, "Query failed".to_string())
            }
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal error".to_string())
            }
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// What to answer when ClickHouse stopped a query at one of the org's limits (time, memory, rows,
/// quota, the server's concurrent queries): a message the user can act on instead of "Query failed".
fn limit_error(e: &clickhouse::error::Error) -> Option<(StatusCode, String)> {
    let clickhouse::error::Error::BadResponse(text) = e else {
        return None;
    };
    let code: u32 = text.strip_prefix("Code: ")?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    let (status, message) = match code {
        // TIMEOUT_EXCEEDED, TOO_SLOW
        159 | 160 => (StatusCode::SERVICE_UNAVAILABLE, "The query took longer than this organization's limit; narrow the time range or the query."),
        // MEMORY_LIMIT_EXCEEDED
        241 => (StatusCode::SERVICE_UNAVAILABLE, "The query needed more memory than this organization's limit; narrow the time range or the query."),
        // TOO_MANY_ROWS, TOO_MANY_BYTES, TOO_MANY_ROWS_OR_BYTES
        158 | 307 | 396 => (StatusCode::SERVICE_UNAVAILABLE, "The query would read more data than this organization's limit; narrow the time range or the query."),
        // QUOTA_EXCEEDED
        201 => (StatusCode::TOO_MANY_REQUESTS, "This organization used up its query quota for now; retry in a minute."),
        // TOO_MANY_SIMULTANEOUS_QUERIES
        202 => (StatusCode::SERVICE_UNAVAILABLE, "The database is running too many queries; retry shortly."),
        _ => return None,
    };
    Some((status, message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(e: clickhouse::error::Error) -> (StatusCode, String) {
        let response = ApiError::ClickHouse(e).into_response();
        let status = response.status();
        let body = futures::executor::block_on(axum::body::to_bytes(response.into_body(), 1 << 20)).unwrap();
        (status, serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"].as_str().unwrap().to_string())
    }

    #[test]
    fn queries_stopped_at_a_limit_get_an_answer_the_user_can_act_on() {
        let bad = |text: &str| clickhouse::error::Error::BadResponse(text.into());
        for (text, status, says) in [
            ("Code: 159. DB::Exception: Timeout exceeded: elapsed 60.0 seconds, maximum: 60. (TIMEOUT_EXCEEDED)", StatusCode::SERVICE_UNAVAILABLE, "took longer"),
            ("Code: 241. DB::Exception: Query memory limit exceeded: would use 4.00 GiB (MEMORY_LIMIT_EXCEEDED)", StatusCode::SERVICE_UNAVAILABLE, "more memory"),
            ("Code: 158. DB::Exception: Limit for rows (controlled by 'max_rows_to_read' setting) exceeded (TOO_MANY_ROWS)", StatusCode::SERVICE_UNAVAILABLE, "more data"),
            ("Code: 201. DB::Exception: Quota for user `observecraft` for 60s has been exceeded (QUOTA_EXCEEDED)", StatusCode::TOO_MANY_REQUESTS, "quota"),
            ("Code: 202. DB::Exception: Too many simultaneous queries. Maximum: 100. (TOO_MANY_SIMULTANEOUS_QUERIES)", StatusCode::SERVICE_UNAVAILABLE, "too many queries"),
        ] {
            let (got, message) = answer(bad(text));
            assert_eq!(got, status, "{text}");
            assert!(message.contains(says), "{message}");
            assert!(!message.contains("DB::Exception"), "the server's text isn't echoed");
        }
        // Anything else stays a plain failure.
        assert_eq!(answer(bad("Code: 62. DB::Exception: Syntax error")), (StatusCode::BAD_GATEWAY, "Query failed".to_string()));
        assert_eq!(answer(clickhouse::error::Error::RowNotFound).0, StatusCode::BAD_GATEWAY);
    }
}
