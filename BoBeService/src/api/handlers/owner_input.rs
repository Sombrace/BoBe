use crate::api::principal::Principal;
use crate::app_state::AppState;
use crate::db::owner_input_repo::InputView;
use crate::error::AppError;
use axum::{
    Json,
    extract::{Extension, Path, State},
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Answer {
    response_id: Uuid,
    response: String,
}

fn personal(state: &AppState) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, AppError> {
    Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_read_owned()
        .map_err(|_| AppError::Conflict("personal data maintenance is active".into()))
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<InputView>>, AppError> {
    let _personal = personal(&state)?;
    state.runtime.background.jobs.expire_owner_inputs().await?;
    Ok(Json(
        state
            .runtime
            .background
            .jobs
            .owner_inputs(principal.conversation_channel())
            .await?
            .into_iter()
            .map(crate::db::owner_input_repo::OwnerInput::view)
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

pub(crate) async fn get(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<InputView>, AppError> {
    let _personal = personal(&state)?;
    state.runtime.background.jobs.expire_owner_inputs().await?;
    let input = state.runtime.background.jobs.owner_input(id).await?;
    if !input.visible_in(principal.conversation_channel()) {
        return Err(AppError::NotFound(
            "owner question is not in this channel".into(),
        ));
    }
    Ok(Json(input.view()?))
}

pub(crate) async fn answer(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(answer): Json<Answer>,
) -> Result<Json<InputView>, AppError> {
    let _personal = personal(&state)?;
    let input = state
        .runtime
        .background
        .jobs
        .answer_owner_input(
            id,
            answer.response_id,
            &answer.response,
            principal.conversation_channel(),
        )
        .await?;
    Ok(Json(input.view()?))
}

pub(crate) async fn presented(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let _personal = personal(&state)?;
    state
        .runtime
        .background
        .jobs
        .present_owner_input(id, principal.conversation_channel())
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn cancel(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<InputView>, AppError> {
    let _personal = personal(&state)?;
    Ok(Json(
        state
            .runtime
            .background
            .jobs
            .cancel_owner_input(id, principal.conversation_channel())
            .await?
            .view()?,
    ))
}
