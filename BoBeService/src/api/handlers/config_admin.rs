use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::bootstrap::mcp_loader::load_mcp_servers;
use crate::config::manager::UpdateResult;
use crate::error::AppError;
use crate::provider::McpRuntimeConfig;

const MCP_ENABLED: &str = "mcp.enabled";

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
    let result = if key == MCP_ENABLED {
        unset_mcp_enabled(&state).await?
    } else {
        state.infra.config_manager.unset(&key)?
    };
    if key.starts_with("capture.") {
        state.runtime.runtime_session.wake_capture();
    }
    if key.starts_with("checkin.") {
        state.runtime.runtime_session.reconfigure_checkins().await;
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
    let result = if request.changes.contains_key(MCP_ENABLED) {
        update_with_mcp(&state, request.changes).await?
    } else {
        state.infra.config_manager.update(&request.changes)?
    };
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
    Ok(Json(ConfigUpdateResponse {
        applied_fields: result.applied_fields,
        restart_required_fields: result.restart_required_fields,
    }))
}

pub(super) async fn update_with_mcp(
    state: &Arc<AppState>,
    changes: HashMap<String, serde_json::Value>,
) -> Result<UpdateResult, AppError> {
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let _mcp_guard = state.infra.mcp_config_lock.lock().await;
        let manager = &state.infra.config_manager;
        let workers = &state.runtime.workers;
        let changes = &changes;
        let Some(enabled) = changes
            .get(MCP_ENABLED)
            .and_then(serde_json::Value::as_bool)
        else {
            return tokio::task::block_in_place(|| manager.update(changes));
        };
        let current = state.infra.config.load_full();
        let mut next = (*current).clone();
        next.mcp.enabled = enabled;
        reload_then_commit(
            move |runtime| workers.apply_mcp_config(runtime),
            load_mcp_servers(&current),
            load_mcp_servers(&next),
            move || tokio::task::block_in_place(|| manager.update(changes)),
            |result| !result.persist_failed,
        )
        .await
    })
    .await
    .map_err(|error| AppError::Internal(format!("configuration update stopped: {error}")))?
}

async fn unset_mcp_enabled(state: &Arc<AppState>) -> Result<UpdateResult, AppError> {
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let _mcp_guard = state.infra.mcp_config_lock.lock().await;
        let manager = &state.infra.config_manager;
        let workers = &state.runtime.workers;
        let config = &state.infra.config;
        let previous = config.load().mcp.enabled;
        commit_then_reload(
            move || manager.unset(MCP_ENABLED),
            move || workers.apply_mcp_config(load_mcp_servers(&config.load_full())),
            move || {
                if config.load().mcp.enabled == previous {
                    return Ok(());
                }
                restore_mcp_enabled(manager, previous)
            },
        )
        .await
    })
    .await
    .map_err(|error| AppError::Internal(format!("configuration update stopped: {error}")))?
}

fn restore_mcp_enabled(
    manager: &crate::config::manager::ConfigManager,
    enabled: bool,
) -> Result<(), AppError> {
    let restored = manager.update(&HashMap::from([(
        MCP_ENABLED.to_owned(),
        serde_json::Value::Bool(enabled),
    )]))?;
    if restored.persist_failed {
        return Err(AppError::Io(std::io::Error::other(
            "failed to persist configuration",
        )));
    }
    Ok(())
}

async fn reload_then_commit<T, F>(
    reload: impl Fn(McpRuntimeConfig) -> F,
    previous: McpRuntimeConfig,
    next: McpRuntimeConfig,
    commit: impl FnOnce() -> Result<T, AppError>,
    took_effect: impl FnOnce(&T) -> bool,
) -> Result<T, AppError>
where
    F: Future<Output = Result<(), AppError>>,
{
    reload(next).await?;
    let outcome = commit();
    if outcome.as_ref().is_ok_and(took_effect) {
        return outcome;
    }
    match reload(previous).await {
        Ok(()) => outcome,
        Err(restore) => Err(AppError::Internal(format!(
            "{}; restoring the previous MCP servers also failed: {restore}",
            outcome.as_ref().err().map_or_else(
                || "failed to persist configuration".to_owned(),
                ToString::to_string
            )
        ))),
    }
}

async fn commit_then_reload<T, F>(
    commit: impl FnOnce() -> Result<T, AppError>,
    reload: impl FnOnce() -> F,
    restore: impl FnOnce() -> Result<(), AppError>,
) -> Result<T, AppError>
where
    F: Future<Output = Result<(), AppError>>,
{
    let committed = commit()?;
    let Err(error) = reload().await else {
        return Ok(committed);
    };
    Err(match restore() {
        Ok(()) => error,
        Err(restore) => AppError::Internal(format!(
            "{error}; restoring the previous configuration also failed: {restore}"
        )),
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    fn servers(count: usize) -> McpRuntimeConfig {
        McpRuntimeConfig {
            servers: (0..count)
                .map(|index| {
                    (
                        format!("server-{index}"),
                        crate::provider::McpServerSpec {
                            command: "server".into(),
                            args: Vec::new(),
                            env: HashMap::new(),
                            timeout_ms: 1_000,
                            excluded_tools: Vec::new(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn saved(persist_failed: bool) -> UpdateResult {
        UpdateResult {
            applied_fields: vec![MCP_ENABLED.to_owned()],
            restart_required_fields: Vec::new(),
            persist_failed,
        }
    }

    #[tokio::test]
    async fn mcp_is_switched_only_after_the_worker_accepts_the_new_servers() {
        let reloads = RefCell::new(Vec::new());
        let committed = Cell::new(false);
        let error = reload_then_commit(
            |runtime: McpRuntimeConfig| {
                reloads.borrow_mut().push(runtime.servers.len());
                async { Err(AppError::Internal("MCP reload was rolled back".into())) }
            },
            servers(0),
            servers(2),
            || {
                committed.set(true);
                Ok(saved(false))
            },
            |result| !result.persist_failed,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("rolled back"), "{error}");
        assert!(!committed.get());
        assert_eq!(*reloads.borrow(), [2]);

        reloads.borrow_mut().clear();
        let result = reload_then_commit(
            |runtime: McpRuntimeConfig| {
                reloads.borrow_mut().push(runtime.servers.len());
                async { Ok(()) }
            },
            servers(0),
            servers(2),
            || Ok(saved(false)),
            |result| !result.persist_failed,
        )
        .await
        .unwrap();
        assert_eq!(result.applied_fields, [MCP_ENABLED]);
        assert_eq!(*reloads.borrow(), [2]);
    }

    #[tokio::test]
    async fn an_unsaved_switch_puts_the_previous_mcp_servers_back() {
        let reloads = RefCell::new(Vec::new());
        let reload = |runtime: McpRuntimeConfig| {
            reloads.borrow_mut().push(runtime.servers.len());
            async { Ok(()) }
        };
        let error = reload_then_commit(
            reload,
            servers(1),
            servers(2),
            || Err::<UpdateResult, _>(AppError::Validation("invalid value for voice.speed".into())),
            |result| !result.persist_failed,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
        assert_eq!(*reloads.borrow(), [2, 1]);

        reloads.borrow_mut().clear();
        let result = reload_then_commit(
            reload,
            servers(1),
            servers(2),
            || Ok(saved(true)),
            |result| !result.persist_failed,
        )
        .await
        .unwrap();
        assert!(result.persist_failed);
        assert_eq!(*reloads.borrow(), [2, 1]);

        let error = reload_then_commit(
            |runtime: McpRuntimeConfig| {
                let restoring = runtime.servers.len() == 1;
                async move {
                    if restoring {
                        Err(AppError::ServiceUnavailable("worker unavailable".into()))
                    } else {
                        Ok(())
                    }
                }
            },
            servers(1),
            servers(2),
            || Err::<UpdateResult, _>(AppError::Io(std::io::Error::other("disk full"))),
            |result| !result.persist_failed,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("disk full"), "{error}");
        assert!(error.contains("worker unavailable"), "{error}");
    }

    #[tokio::test]
    async fn an_unset_the_worker_rejects_restores_the_saved_value() {
        let restored = Cell::new(false);
        let error = commit_then_reload(
            || Ok(saved(false)),
            || async { Err(AppError::Internal("MCP reload was rolled back".into())) },
            || {
                restored.set(true);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("rolled back"), "{error}");
        assert!(restored.get());

        let error = commit_then_reload(
            || Ok(saved(false)),
            || async { Err(AppError::Internal("MCP reload was rolled back".into())) },
            || Err(AppError::Io(std::io::Error::other("disk full"))),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("rolled back"), "{error}");
        assert!(error.contains("disk full"), "{error}");

        restored.set(false);
        assert!(
            commit_then_reload(
                || Ok(saved(false)),
                || async { Ok(()) },
                || {
                    restored.set(true);
                    Ok(())
                },
            )
            .await
            .is_ok()
        );
        assert!(!restored.get());
    }
}
