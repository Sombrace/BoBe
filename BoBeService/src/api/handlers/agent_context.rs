use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, State},
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::provider::diagnostics::{AgentContextStatus, CompactionResult};

pub(crate) async fn status(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
) -> Result<Json<AgentContextStatus>, AppError> {
    Ok(Json(
        state
            .runtime
            .workers
            .context_status(principal.conversation_channel())
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompactRequest {
    keep_recent_tokens: Option<u32>,
}

pub(crate) async fn compact(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
    Json(request): Json<CompactRequest>,
) -> Result<Json<CompactionResult>, AppError> {
    if request
        .keep_recent_tokens
        .is_some_and(|tokens| !(1..=64_000).contains(&tokens))
    {
        return Err(AppError::Validation(
            "keep_recent_tokens must be between 1 and 64000".into(),
        ));
    }
    if super::providers::restart_required(&state) {
        return Err(AppError::Conflict(
            "Restart the daemon to apply provider changes before compaction".into(),
        ));
    }
    let _admission = state
        .runtime
        .runtime_session
        .try_begin_personal_data_access()
        .map_err(|message| AppError::Conflict(message.into()))?;
    let _personal_data = Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_write_owned()
        .map_err(|_| AppError::Conflict("Background work is active".into()))?;
    Ok(Json(
        state
            .runtime
            .workers
            .compact_context(request.keep_recent_tokens, principal.conversation_channel())
            .await?,
    ))
}
