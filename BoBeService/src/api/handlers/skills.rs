use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::provider::SkillSummary;

#[derive(Debug, Serialize)]
pub(crate) struct SkillListResponse {
    skills: Vec<SkillSummary>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillReadResponse {
    id: String,
    body: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillReloadResponse {
    reloaded: bool,
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SkillListResponse>, AppError> {
    Ok(Json(SkillListResponse {
        skills: state.runtime.skill_store.list().await?,
    }))
}

pub(crate) async fn read(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SkillReadResponse>, AppError> {
    let body = state.runtime.skill_store.read(&id).await?;
    Ok(Json(SkillReadResponse { id, body }))
}

pub(crate) async fn reload(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SkillReloadResponse>, AppError> {
    state.runtime.workers.reload_skills().await?;
    Ok(Json(SkillReloadResponse { reloaded: true }))
}
