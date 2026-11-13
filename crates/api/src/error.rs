use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict(String),
    TooLarge,
    RateLimited,
    Internal(anyhow::Error),
}

impl ApiError {
    pub fn internal(err: impl Into<anyhow::Error>) -> Self {
        Self::Internal(err.into())
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict(_) => "conflict",
            Self::TooLarge => "too_large",
            Self::RateLimited => "rate_limited",
            Self::Internal(_) => "internal",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::BadRequest(msg) | Self::Conflict(msg) => msg.clone(),
            Self::Unauthorized => "authentication required".into(),
            Self::Forbidden => "you lack permission for this action".into(),
            Self::NotFound => "not found".into(),
            Self::TooLarge => "payload too large".into(),
            Self::RateLimited => "too many requests".into(),
            Self::Internal(_) => "internal server error".into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal(err) = &self {
            tracing::error!(error = %err, "request failed");
        }
        let status = self.status();
        let body = Json(json!({ "code": self.code(), "message": self.message() }));
        (status, body).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

/// Map a unique-violation to a conflict without leaking database details.
pub fn unique_conflict(err: sqlx::Error, message: &str) -> ApiError {
    if let sqlx::Error::Database(db) = &err
        && db.code().as_deref() == Some("23505")
    {
        return ApiError::Conflict(message.to_owned());
    }
    ApiError::Internal(err.into())
}

pub type ApiResult<T> = Result<T, ApiError>;
