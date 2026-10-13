use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path, State},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::db::agent_job_repo::{AgentJob, JobInput};
use crate::error::AppError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Submit {
    request_id: Uuid,
    title: String,
    task: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    budget: crate::db::agent_job_repo::TaskBudget,
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AgentJob>>, AppError> {
    Ok(Json(state.runtime.background.jobs.list().await?))
}

pub(crate) async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentJob>, AppError> {
    Ok(Json(state.runtime.background.jobs.get(id).await?))
}

pub(crate) async fn start(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
    Json(request): Json<Submit>,
) -> Result<Json<AgentJob>, AppError> {
    require_enabled(&state)?;
    let _personal_data = Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_read_owned()
        .map_err(|_| AppError::Conflict("Personal data maintenance is active".into()))?;
    let _lifecycle = state.runtime.agent_authority_lifecycle.read().await?;
    Ok(Json(
        state
            .runtime
            .background
            .jobs
            .submit(
                &format!("owner:{}", request.request_id),
                JobInput {
                    title: request.title,
                    task: request.task,
                    context: request.context,
                    channel_id: Some(principal.conversation_channel()),
                    goal_id: None,
                    budget: request.budget,
                    ..JobInput::default()
                },
                &state.runtime.active_routes.goals,
            )
            .await?,
    ))
}

pub(crate) async fn cancel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentJob>, AppError> {
    Ok(Json(state.runtime.background.jobs.cancel(id).await?))
}

pub(crate) async fn resume(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentJob>, AppError> {
    require_enabled(&state)?;
    let _personal_data = Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_read_owned()
        .map_err(|_| AppError::Conflict("Personal data maintenance is active".into()))?;
    let _lifecycle = state.runtime.agent_authority_lifecycle.read().await?;
    Ok(Json(state.runtime.background.jobs.resume(id).await?))
}

pub(super) fn require_enabled(state: &AppState) -> Result<(), AppError> {
    if !state
        .runtime
        .active_agent
        .enabled_tools
        .contains(&crate::config::AgentTool::BackgroundJobs)
    {
        return Err(AppError::Conflict(
            "background work is disabled in the active tool selection".into(),
        ));
    }
    if **state.infra.initialization.load()
        != crate::db::runtime_initialization_repo::InitializationState::Ready
    {
        return Err(AppError::Conflict(
            "complete provider initialization before starting background work".into(),
        ));
    }
    if super::providers::restart_required(state) {
        return Err(AppError::Conflict(
            "restart the daemon to apply provider changes before starting work".into(),
        ));
    }
    Ok(())
}
