use std::sync::Arc;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::models::ids::UserProfileId;
use crate::models::user_profile::UserProfile;
use crate::services::DeleteOutcome;
use crate::services::user_profile_service::UserProfileService;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(crate) struct UserProfileResponse {
    pub(crate) id: UserProfileId,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) enabled: bool,
    pub(crate) is_default: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<&UserProfile> for UserProfileResponse {
    fn from(profile: &UserProfile) -> Self {
        Self {
            id: profile.id,
            name: profile.name.clone(),
            content: profile.content.clone(),
            enabled: profile.enabled,
            is_default: profile.is_default,
            created_at: profile.created_at,
            updated_at: profile.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct UserProfileListResponse {
    pub(crate) profiles: Vec<UserProfileResponse>,
    pub(crate) count: usize,
    pub(crate) enabled_count: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserProfileCreateRequest {
    pub(crate) name: String,
    pub(crate) content: String,
    #[serde(default = "super::default_true")]
    pub(crate) enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserProfileUpdateRequest {
    pub(crate) content: Option<String>,
    pub(crate) enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserProfileActionResponse {
    pub(crate) id: UserProfileId,
    pub(crate) name: String,
    pub(crate) enabled: bool,
    pub(crate) message: String,
}

async fn set_enabled(
    service: &UserProfileService,
    profile_id: UserProfileId,
    enabled: bool,
) -> Result<Json<UserProfileActionResponse>, AppError> {
    let profile = service
        .set_enabled(profile_id, enabled)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("User profile {profile_id} not found")))?;

    Ok(Json(UserProfileActionResponse {
        id: profile_id,
        name: profile.name,
        enabled,
        message: if enabled {
            "User profile enabled".into()
        } else {
            "User profile disabled".into()
        },
    }))
}

pub(crate) async fn list_profiles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<UserProfileListResponse>, AppError> {
    let summary = state.services.user_profile_service.list(false).await?;
    Ok(Json(UserProfileListResponse {
        count: summary.profiles.len(),
        enabled_count: summary.enabled_count,
        profiles: summary
            .profiles
            .iter()
            .map(UserProfileResponse::from)
            .collect(),
    }))
}

pub(crate) async fn create_profile(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UserProfileCreateRequest>,
) -> Result<(StatusCode, Json<UserProfileResponse>), AppError> {
    let saved = state
        .services
        .user_profile_service
        .create(body.name, body.content, body.enabled)
        .await?;
    Ok((StatusCode::CREATED, Json(UserProfileResponse::from(&saved))))
}

pub(crate) async fn update_profile(
    State(state): State<Arc<AppState>>,
    Path(profile_id): Path<UserProfileId>,
    Json(body): Json<UserProfileUpdateRequest>,
) -> Result<Json<UserProfileResponse>, AppError> {
    let updated = state
        .services
        .user_profile_service
        .update(profile_id, body.content, body.enabled)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("User profile {profile_id} not found")))?;
    Ok(Json(UserProfileResponse::from(&updated)))
}

pub(crate) async fn enable_profile(
    State(state): State<Arc<AppState>>,
    Path(profile_id): Path<UserProfileId>,
) -> Result<Json<UserProfileActionResponse>, AppError> {
    set_enabled(&state.services.user_profile_service, profile_id, true).await
}

pub(crate) async fn disable_profile(
    State(state): State<Arc<AppState>>,
    Path(profile_id): Path<UserProfileId>,
) -> Result<Json<UserProfileActionResponse>, AppError> {
    set_enabled(&state.services.user_profile_service, profile_id, false).await
}

pub(crate) async fn delete_profile(
    State(state): State<Arc<AppState>>,
    Path(profile_id): Path<UserProfileId>,
) -> Result<StatusCode, AppError> {
    match state
        .services
        .user_profile_service
        .delete(profile_id)
        .await?
    {
        DeleteOutcome::Deleted => Ok(StatusCode::NO_CONTENT),
        DeleteOutcome::NotFound => Err(AppError::NotFound(format!(
            "User profile {profile_id} not found"
        ))),
    }
}
