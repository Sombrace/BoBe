use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::config::AgentTool;
use crate::error::AppError;

#[derive(Serialize)]
pub(crate) struct ToolSelection {
    tools: Vec<ToolState>,
    restart_required: bool,
}

#[derive(Serialize)]
struct ToolState {
    name: AgentTool,
    description: &'static str,
    enabled: bool,
    active: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolSelectionRequest {
    enabled_tools: BTreeSet<AgentTool>,
}

pub(crate) async fn get(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ToolSelection>, AppError> {
    Ok(Json(selection(&state)?))
}

pub(crate) async fn set(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ToolSelectionRequest>,
) -> Result<Json<ToolSelection>, AppError> {
    let changes = HashMap::from([(
        "agent.enabled_tools".into(),
        serde_json::to_value(request.enabled_tools)?,
    )]);
    let result = state.infra.config_manager.update(&changes)?;
    if result.persist_failed {
        return Err(AppError::Config(
            "could not persist the agent tool selection".into(),
        ));
    }
    Ok(Json(selection(&state)?))
}

fn selection(state: &AppState) -> Result<ToolSelection, AppError> {
    let desired = crate::config::Config::load_from(
        &state.infra.config_root.join("config.toml"),
        state.infra.config_root.as_path(),
    )?
    .config
    .agent;
    Ok(ToolSelection {
        tools: AgentTool::ALL
            .into_iter()
            .map(|tool| ToolState {
                name: tool,
                description: tool.description(),
                enabled: desired.enabled_tools.contains(&tool),
                active: state.runtime.active_agent.enabled_tools.contains(&tool),
            })
            .collect(),
        restart_required: desired != state.runtime.active_agent,
    })
}
