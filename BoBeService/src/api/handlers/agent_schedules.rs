use crate::app_state::AppState;
use crate::db::agent_schedule_repo::{AgentSchedule, ScheduleInput};
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
pub(crate) struct Submit {
    request_id: Uuid,
    schedule: ScheduleInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Enabled {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Time {
    generation: i64,
    at: chrono::DateTime<chrono::Utc>,
}

fn personal(state: &AppState) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, AppError> {
    Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_read_owned()
        .map_err(|_| AppError::Conflict("personal data maintenance is active".into()))
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AgentSchedule>>, AppError> {
    Ok(Json(state.runtime.background.jobs.schedules().await?))
}

pub(crate) async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentSchedule>, AppError> {
    Ok(Json(state.runtime.background.jobs.schedule(id).await?))
}

pub(crate) async fn create(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
    Json(input): Json<Submit>,
) -> Result<Json<AgentSchedule>, AppError> {
    super::agent_jobs::require_enabled(&state)?;
    let _personal = personal(&state)?;
    if let Some(goal) = input.schedule.goal_id {
        state
            .services
            .goals_service
            .get(goal)
            .await?
            .ok_or_else(|| AppError::NotFound("goal was not found".into()))?;
    }
    Ok(Json(
        state
            .runtime
            .background
            .jobs
            .create_schedule(
                &format!("owner:{}", input.request_id),
                input.schedule,
                principal.conversation_channel(),
            )
            .await?,
    ))
}

pub(crate) async fn set_enabled(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(input): Json<Enabled>,
) -> Result<Json<AgentSchedule>, AppError> {
    if input.enabled {
        super::agent_jobs::require_enabled(&state)?;
    }
    let _personal = personal(&state)?;
    Ok(Json(
        state
            .runtime
            .background
            .jobs
            .set_schedule_enabled(id, input.enabled)
            .await?,
    ))
}

pub(crate) async fn reschedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(input): Json<Time>,
) -> Result<Json<AgentSchedule>, AppError> {
    let _personal = personal(&state)?;
    Ok(Json(
        state
            .runtime
            .background
            .jobs
            .reschedule(id, input.generation, input.at)
            .await?,
    ))
}

pub(crate) async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let _personal = personal(&state)?;
    state.runtime.background.jobs.delete_schedule(id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
