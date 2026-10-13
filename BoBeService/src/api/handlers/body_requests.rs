use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;

use crate::api::principal::Principal;
use crate::app_state::AppState;
use crate::error::AppError;
use crate::util::sse::body_requests::BodyReply;

pub(crate) async fn complete(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(request_id): Path<String>,
    Json(reply): Json<BodyReply>,
) -> Result<StatusCode, AppError> {
    state
        .infra
        .connection_manager
        .complete(&principal, &request_id, reply)
        .map_err(|_| AppError::NotFound("body request".into()))?;
    Ok(StatusCode::NO_CONTENT)
}
