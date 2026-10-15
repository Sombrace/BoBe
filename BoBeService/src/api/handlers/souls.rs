use std::sync::Arc;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::models::ids::SoulId;
use crate::models::soul::Soul;
use crate::services::DeleteOutcome;
use crate::services::souls_service::SoulsService;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(crate) struct SoulResponse {
    pub(crate) id: SoulId,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) enabled: bool,
    pub(crate) is_default: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<&Soul> for SoulResponse {
    fn from(soul: &Soul) -> Self {
        Self {
            id: soul.id,
            name: soul.name.clone(),
            content: soul.content.clone(),
            enabled: soul.enabled,
            is_default: soul.is_default,
            created_at: soul.created_at,
            updated_at: soul.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct SoulListResponse {
    pub(crate) souls: Vec<SoulResponse>,
    pub(crate) count: usize,
    pub(crate) enabled_count: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SoulCreateRequest {
    pub(crate) name: String,
    pub(crate) content: String,
    #[serde(default = "super::default_true")]
    pub(crate) enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SoulUpdateRequest {
    pub(crate) content: Option<String>,
    pub(crate) enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SoulActionResponse {
    pub(crate) id: SoulId,
    pub(crate) name: String,
    pub(crate) enabled: bool,
    pub(crate) message: String,
}

async fn set_enabled(
    service: &SoulsService,
    soul_id: SoulId,
    enabled: bool,
) -> Result<Json<SoulActionResponse>, AppError> {
    let soul = service
        .set_enabled(soul_id, enabled)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Soul {soul_id} not found")))?;

    Ok(Json(SoulActionResponse {
        id: soul_id,
        name: soul.name,
        enabled,
        message: if enabled {
            "Soul enabled".into()
        } else {
            "Soul disabled".into()
        },
    }))
}

pub(crate) async fn list_souls(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SoulListResponse>, AppError> {
    let summary = state.services.souls_service.list(false).await?;
    Ok(Json(SoulListResponse {
        count: summary.souls.len(),
        enabled_count: summary.enabled_count,
        souls: summary.souls.iter().map(SoulResponse::from).collect(),
    }))
}

pub(crate) async fn create_soul(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SoulCreateRequest>,
) -> Result<(StatusCode, Json<SoulResponse>), AppError> {
    let saved = state
        .services
        .souls_service
        .create(body.name, body.content, body.enabled)
        .await?;
    Ok((StatusCode::CREATED, Json(SoulResponse::from(&saved))))
}

pub(crate) async fn update_soul(
    State(state): State<Arc<AppState>>,
    Path(soul_id): Path<SoulId>,
    Json(body): Json<SoulUpdateRequest>,
) -> Result<Json<SoulResponse>, AppError> {
    let updated = state
        .services
        .souls_service
        .update(soul_id, body.content, body.enabled)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Soul {soul_id} not found")))?;
    Ok(Json(SoulResponse::from(&updated)))
}

pub(crate) async fn enable_soul(
    State(state): State<Arc<AppState>>,
    Path(soul_id): Path<SoulId>,
) -> Result<Json<SoulActionResponse>, AppError> {
    set_enabled(&state.services.souls_service, soul_id, true).await
}

pub(crate) async fn disable_soul(
    State(state): State<Arc<AppState>>,
    Path(soul_id): Path<SoulId>,
) -> Result<Json<SoulActionResponse>, AppError> {
    set_enabled(&state.services.souls_service, soul_id, false).await
}

pub(crate) async fn delete_soul(
    State(state): State<Arc<AppState>>,
    Path(soul_id): Path<SoulId>,
) -> Result<StatusCode, AppError> {
    match state.services.souls_service.delete(soul_id).await? {
        DeleteOutcome::Deleted => Ok(StatusCode::NO_CONTENT),
        DeleteOutcome::NotFound => Err(AppError::NotFound(format!("Soul {soul_id} not found"))),
    }
}
