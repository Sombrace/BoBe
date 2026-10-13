use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::error::AppError;
use crate::models::ids::GoalId;
use crate::models::types::GoalStatus;
use crate::services::goals::goal_md::GoalDoc;
use crate::services::goals::goals_service::GoalPatch;

#[derive(Debug, Serialize)]
pub(crate) struct GoalResponse {
    pub(crate) id: GoalId,
    pub(crate) title: String,
    pub(crate) status: GoalStatus,
    pub(crate) priority: u8,
    pub(crate) summary: String,
    pub(crate) why_it_matters: String,
    pub(crate) how_working_on_it: String,
    pub(crate) patterns_observed: String,
    pub(crate) attitude_feelings: String,
    pub(crate) open_questions: String,
    pub(crate) notes: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<&GoalDoc> for GoalResponse {
    fn from(doc: &GoalDoc) -> Self {
        Self {
            id: doc.id,
            title: doc.title.clone(),
            status: doc.status,
            priority: doc.priority,
            summary: doc.summary.clone(),
            why_it_matters: doc.why_it_matters.clone(),
            how_working_on_it: doc.how_working_on_it.clone(),
            patterns_observed: doc.patterns_observed.clone(),
            attitude_feelings: doc.attitude_feelings.clone(),
            open_questions: doc.open_questions.clone(),
            notes: doc.notes.clone(),
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct GoalListResponse {
    pub(crate) goals: Vec<GoalResponse>,
    pub(crate) count: usize,
    pub(crate) active_count: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoalCreateRequest {
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) why_it_matters: String,
    #[serde(default = "default_priority")]
    pub(crate) priority: u8,
}

fn default_priority() -> u8 {
    2
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoalUpdateRequest {
    pub(crate) title: Option<String>,
    pub(crate) status: Option<GoalStatus>,
    pub(crate) priority: Option<u8>,
    pub(crate) summary: Option<String>,
    pub(crate) why_it_matters: Option<String>,
    pub(crate) notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GoalActionResponse {
    pub(crate) id: GoalId,
    pub(crate) status: GoalStatus,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoalListQuery {
    pub(crate) status: Option<GoalStatus>,
    #[serde(default)]
    pub(crate) include_archived: bool,
}

pub(crate) async fn list_goals(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GoalListQuery>,
) -> Result<Json<GoalListResponse>, AppError> {
    let mut goals = if let Some(status) = params.status {
        let all = state.services.goals_service.list_all().await?;
        all.into_iter().filter(|g| g.status == status).collect()
    } else {
        state.services.goals_service.list_all().await?
    };

    if !params.include_archived && params.status.is_none() {
        goals.retain(|g| g.status != GoalStatus::Archived);
    }

    let active_count = goals
        .iter()
        .filter(|g| g.status == GoalStatus::Active)
        .count();
    Ok(Json(GoalListResponse {
        count: goals.len(),
        active_count,
        goals: goals.iter().map(GoalResponse::from).collect(),
    }))
}

pub(crate) async fn get_goal(
    State(state): State<Arc<AppState>>,
    Path(goal_id): Path<GoalId>,
) -> Result<Json<GoalResponse>, AppError> {
    let doc = state
        .services
        .goals_service
        .get(goal_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Goal {goal_id} not found")))?;
    Ok(Json(GoalResponse::from(&doc)))
}

pub(crate) async fn create_goal(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GoalCreateRequest>,
) -> Result<(StatusCode, Json<GoalResponse>), AppError> {
    let mut doc = GoalDoc::new(body.title, body.summary);
    doc.priority = body.priority;
    doc.why_it_matters = body.why_it_matters;

    let saved = state.services.goals_service.create(doc).await?;
    Ok((StatusCode::CREATED, Json(GoalResponse::from(&saved))))
}

pub(crate) async fn update_goal(
    State(state): State<Arc<AppState>>,
    Path(goal_id): Path<GoalId>,
    Json(body): Json<GoalUpdateRequest>,
) -> Result<Json<GoalResponse>, AppError> {
    let updated = state
        .services
        .goals_service
        .update(
            goal_id,
            GoalPatch {
                title: body.title,
                status: body.status,
                priority: body.priority,
                summary: body.summary,
                why_it_matters: body.why_it_matters,
                how_working_on_it: None,
                patterns_observed: None,
                attitude_feelings: None,
                open_questions: None,
                notes: body.notes,
            },
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Goal {goal_id} not found")))?;

    Ok(Json(GoalResponse::from(&updated)))
}

pub(crate) async fn complete_goal(
    State(state): State<Arc<AppState>>,
    Path(goal_id): Path<GoalId>,
) -> Result<Json<GoalActionResponse>, AppError> {
    let updated = state
        .services
        .goals_service
        .set_status(goal_id, GoalStatus::Completed)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Goal {goal_id} not found")))?;
    Ok(Json(GoalActionResponse {
        id: goal_id,
        status: updated.status,
        message: "Goal marked as completed".into(),
    }))
}

pub(crate) async fn archive_goal(
    State(state): State<Arc<AppState>>,
    Path(goal_id): Path<GoalId>,
) -> Result<Json<GoalActionResponse>, AppError> {
    let updated = state
        .services
        .goals_service
        .set_status(goal_id, GoalStatus::Archived)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Goal {goal_id} not found")))?;
    Ok(Json(GoalActionResponse {
        id: goal_id,
        status: updated.status,
        message: "Goal archived".into(),
    }))
}

pub(crate) async fn delete_goal(
    State(state): State<Arc<AppState>>,
    Path(goal_id): Path<GoalId>,
) -> Result<StatusCode, AppError> {
    if !state.services.goals_service.delete(goal_id).await? {
        return Err(AppError::NotFound(format!("Goal {goal_id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}
