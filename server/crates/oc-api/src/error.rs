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
            // Storage errors are logged in full but not echoed to clients.
            ApiError::ClickHouse(e) => {
                tracing::error!(error = %e, "clickhouse query failed");
                (StatusCode::BAD_GATEWAY, "Query failed".to_string())
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
