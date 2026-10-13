use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::provider::{
    WorkspaceEntry, WorkspaceListParams, WorkspaceRead, WorkspaceReadParams, WorkspaceWriteParams,
};

#[derive(Debug, Deserialize)]
pub(crate) struct WorkspaceListQuery {
    #[serde(default)]
    path: String,
    #[serde(default = "default_list_entries")]
    max_entries: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkspaceReadQuery {
    path: String,
    #[serde(default = "default_read_bytes")]
    max_bytes: usize,
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WorkspaceListQuery>,
) -> Result<Json<Vec<WorkspaceEntry>>, AppError> {
    Ok(Json(
        state
            .runtime
            .agent_workspace
            .list(WorkspaceListParams::new(query.path, query.max_entries))
            .await?,
    ))
}

pub(crate) async fn read(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WorkspaceReadQuery>,
) -> Result<Json<WorkspaceRead>, AppError> {
    Ok(Json(
        state
            .runtime
            .agent_workspace
            .read(WorkspaceReadParams::new(query.path, query.max_bytes))
            .await?,
    ))
}

pub(crate) async fn write(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkspaceWriteParams>,
) -> Result<Json<WorkspaceEntry>, AppError> {
    Ok(Json(state.runtime.agent_workspace.write(request).await?))
}

const fn default_list_entries() -> usize {
    100
}

const fn default_read_bytes() -> usize {
    256 * 1024
}
