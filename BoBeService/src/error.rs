use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Protocol incompatible: {0}")]
    ProtocolIncompatible(String),

    #[error("{0}")]
    RequestReplayUnsafe(String),

    #[error("{0}")]
    NotNetworked(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Capture error: {0}")]
    Capture(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Canceled: {0}")]
    Canceled(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;

        let (status, code) = match &self {
            AppError::Validation(_) => (StatusCode::BAD_REQUEST, "VALIDATION_ERROR"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            AppError::ProtocolIncompatible(_) => (StatusCode::CONFLICT, "PROTOCOL_INCOMPATIBLE"),
            AppError::RequestReplayUnsafe(_) => (StatusCode::CONFLICT, "REQUEST_REPLAY_UNSAFE"),
            AppError::NotNetworked(_) => (StatusCode::CONFLICT, "NOT_NETWORKED"),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            AppError::Database(_) => (StatusCode::SERVICE_UNAVAILABLE, "DATABASE_ERROR"),
            AppError::ServiceUnavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE")
            }
            AppError::Network(_) => (StatusCode::SERVICE_UNAVAILABLE, "NETWORK_ERROR"),
            AppError::Canceled(_) => (StatusCode::CONFLICT, "CANCELED"),
            AppError::Config(_)
            | AppError::Capture(_)
            | AppError::Mcp(_)
            | AppError::Serialization(_)
            | AppError::Io(_)
            | AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        let body = serde_json::json!({
            "error": {
                "code": code,
                "message": self.to_string(),
            }
        });

        (status, axum::Json(body)).into_response()
    }
}
