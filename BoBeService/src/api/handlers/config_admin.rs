use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigUpdateRequest {
    changes: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConfigUpdateResponse {
    applied_fields: Vec<String>,
    restart_required_fields: Vec<String>,
}

pub(crate) async fn unset(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<Json<ConfigUpdateResponse>, AppError> {
    let normalized = key;
    let _mcp_guard = if normalized == "mcp.enabled" {
        Some(state.infra.mcp_config_lock.lock().await)
    } else {
        None
    };
    let result = state.infra.config_manager.unset(&normalized)?;
    if normalized.starts_with("capture.") {
        state.runtime.runtime_session.wake_capture();
    }
    if normalized.starts_with("checkin.") {
        state.runtime.runtime_session.reconfigure_checkins().await;
    }
    if normalized == "mcp.enabled" {
        apply_mcp(&state).await?;
    }
    Ok(Json(ConfigUpdateResponse {
        applied_fields: result.applied_fields,
        restart_required_fields: result.restart_required_fields,
    }))
}

pub(crate) async fn update(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfigUpdateRequest>,
) -> Result<Json<ConfigUpdateResponse>, AppError> {
    if let Some(key) = request
        .changes
        .keys()
        .find(|key| !crate::config::manager_fields::is_known_key(key))
    {
        return Err(AppError::Validation(format!(
            "unknown configuration key: {key}"
        )));
    }
    let capture_changed = request
        .changes
        .keys()
        .any(|key| key.starts_with("capture."));
    let checkin_changed = request
        .changes
        .keys()
        .any(|key| key.starts_with("checkin."));
    let mcp_changed = request.changes.keys().any(|key| key == "mcp.enabled");
    let _mcp_guard = if mcp_changed {
        Some(state.infra.mcp_config_lock.lock().await)
    } else {
        None
    };
    let result = state.infra.config_manager.update(&request.changes)?;
    if result.persist_failed {
        return Err(AppError::Io(std::io::Error::other(
            "failed to persist configuration",
        )));
    }
    if capture_changed {
        state.runtime.runtime_session.wake_capture();
    }
    if checkin_changed {
        state.runtime.runtime_session.reconfigure_checkins().await;
    }
    if mcp_changed {
        apply_mcp(&state).await?;
    }
    Ok(Json(ConfigUpdateResponse {
        applied_fields: result.applied_fields,
        restart_required_fields: result.restart_required_fields,
    }))
}

async fn apply_mcp(state: &AppState) -> Result<(), AppError> {
    let config = state.config();
    let runtime = crate::bootstrap::mcp_loader::load_mcp_servers(&config);
    drop(config);
    state.runtime.workers.apply_mcp_config(runtime).await
}
