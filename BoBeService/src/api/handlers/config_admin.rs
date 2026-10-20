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

pub(super) enum ConfigChange {
    Update(HashMap<String, serde_json::Value>),
    Unset(String),
}

pub(super) struct Committed {
    pub(super) result: UpdateResult,
    pub(super) body_theme_delivered: Option<bool>,
}

pub(crate) async fn unset(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<Json<ConfigUpdateResponse>, AppError> {
    let committed = commit(&state, ConfigChange::Unset(key)).await?;
    Ok(Json(ConfigUpdateResponse {
        applied_fields: committed.result.applied_fields,
        restart_required_fields: committed.result.restart_required_fields,
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
    let committed = commit(&state, ConfigChange::Update(request.changes)).await?;
    if committed.result.persist_failed {
        return Err(AppError::Io(std::io::Error::other(
            "failed to persist configuration",
        )));
    }
    Ok(Json(ConfigUpdateResponse {
        applied_fields: committed.result.applied_fields,
        restart_required_fields: committed.result.restart_required_fields,
    }))
}

pub(super) async fn commit(
    state: &Arc<AppState>,
    change: ConfigChange,
) -> Result<Committed, AppError> {
    let applying = Arc::clone(state);
    let following = Arc::clone(state);
    commit_in_background(
        async move { apply_change(&applying, change).await },
        async move |result| {
            let body_theme_delivered = apply_follow_ups(&following, &result).await;
            Committed {
                result,
                body_theme_delivered,
            }
        },
    )
    .await
}

async fn commit_in_background<T, R, Following>(
    committing: impl Future<Output = Result<T, AppError>> + Send + 'static,
    follow_up: impl FnOnce(T) -> Following + Send + 'static,
) -> Result<R, AppError>
where
    T: Send + 'static,
    R: Send + 'static,
    Following: Future<Output = R> + Send,
{
    tokio::spawn(async move { Ok::<R, AppError>(follow_up(committing.await?).await) })
        .await
        .map_err(|error| AppError::Internal(format!("configuration update stopped: {error}")))?
}

async fn apply_change(state: &AppState, change: ConfigChange) -> Result<UpdateResult, AppError> {
    let manager = &state.infra.config_manager;
    match change {
        ConfigChange::Update(changes) => {
            match changes
                .get(MCP_ENABLED)
                .and_then(serde_json::Value::as_bool)
            {
                Some(enabled) => update_with_mcp(state, &changes, enabled).await,
                None => tokio::task::block_in_place(|| manager.update(&changes)),
            }
        }
        ConfigChange::Unset(key) if key == MCP_ENABLED => unset_mcp_enabled(state).await,
        ConfigChange::Unset(key) => tokio::task::block_in_place(|| manager.unset(&key)),
    }
}

async fn apply_follow_ups(state: &AppState, result: &UpdateResult) -> Option<bool> {
    let applied = |prefix: &str| {
        result
            .applied_fields
            .iter()
            .any(|field| field.starts_with(prefix))
    };
    if applied("capture.") {
        state.runtime.runtime_session.wake_capture();
    }
    if applied("checkin.") {
        state.runtime.runtime_session.reconfigure_checkins().await;
    }
    if !result
        .applied_fields
        .iter()
        .any(|field| field == "body.theme")
    {
        return None;
    }
    let theme = state.config().body.theme;
    Some(state.body.gateway.apply_theme(theme).await)
}

async fn update_with_mcp(
    state: &AppState,
    changes: &HashMap<String, serde_json::Value>,
    enabled: bool,
) -> Result<UpdateResult, AppError> {
    let _mcp_guard = state.infra.mcp_config_lock.lock().await;
    let manager = &state.infra.config_manager;
    let workers = &state.runtime.workers;
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
}

async fn unset_mcp_enabled(state: &AppState) -> Result<UpdateResult, AppError> {
    let _mcp_guard = state.infra.mcp_config_lock.lock().await;
    let manager = &state.infra.config_manager;
    let workers = &state.runtime.workers;
    let config = &state.infra.config;
    let previous = config.load().mcp.enabled;
    commit_then_reload(
        move || tokio::task::block_in_place(|| manager.unset(MCP_ENABLED)),
        move || workers.apply_mcp_config(load_mcp_servers(&config.load_full())),
        move || {
            if config.load().mcp.enabled == previous {
                return Ok(());
            }
            tokio::task::block_in_place(|| restore_mcp_enabled(manager, previous))
        },
    )
    .await
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

async fn commit_then_reload<T>(
    commit: impl FnOnce() -> Result<T, AppError>,
    reload: impl AsyncFnOnce() -> Result<(), AppError>,
    restore: impl FnOnce() -> Result<(), AppError>,
) -> Result<T, AppError> {
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
    use std::assert_matches;
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
    async fn follow_ups_run_once_a_change_commits_even_if_the_request_is_gone() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let committing = Arc::new(tokio::sync::Notify::new());
        let followed = Arc::new(AtomicBool::new(false));
        let request = commit_in_background(
            {
                let committing = Arc::clone(&committing);
                async move {
                    committing.notified().await;
                    Ok(saved(false))
                }
            },
            {
                let followed = Arc::clone(&followed);
                async move |result: UpdateResult| {
                    followed.store(true, Ordering::SeqCst);
                    result
                }
            },
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), request)
                .await
                .is_err()
        );
        assert!(!followed.load(Ordering::SeqCst));

        committing.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !followed.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn follow_ups_wait_for_a_change_that_commits() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let followed = Arc::new(AtomicBool::new(false));
        let error = commit_in_background(
            async {
                Err::<UpdateResult, _>(AppError::Validation(
                    "invalid value for checkin.times".into(),
                ))
            },
            {
                let followed = Arc::clone(&followed);
                async move |result: UpdateResult| {
                    followed.store(true, Ordering::SeqCst);
                    result
                }
            },
        )
        .await
        .unwrap_err();

        assert_matches!(error, AppError::Validation(_));
        assert!(!followed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn mcp_is_switched_only_after_the_worker_accepts_the_new_servers() {
        let reloads = RefCell::new(Vec::new());
        let committed = Cell::new(false);
        let error = reload_then_commit(
            async |runtime: McpRuntimeConfig| {
                reloads.borrow_mut().push(runtime.servers.len());
                Err(AppError::Internal("MCP reload was rolled back".into()))
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
            async |runtime: McpRuntimeConfig| {
                reloads.borrow_mut().push(runtime.servers.len());
                Ok(())
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
        let reload = async |runtime: McpRuntimeConfig| {
            reloads.borrow_mut().push(runtime.servers.len());
            Ok(())
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
        assert_matches!(error, AppError::Validation(_));
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
            async |runtime: McpRuntimeConfig| {
                if runtime.servers.len() == 1 {
                    Err(AppError::ServiceUnavailable("worker unavailable".into()))
                } else {
                    Ok(())
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
            async || Err(AppError::Internal("MCP reload was rolled back".into())),
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
            async || Err(AppError::Internal("MCP reload was rolled back".into())),
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
                async || Ok(()),
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
