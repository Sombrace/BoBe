use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::db::runtime_initialization_repo::InitializationState;
use crate::error::AppError;

#[derive(Debug, Serialize)]
pub(crate) struct InitializationResponse {
    initialization_state: InitializationState,
}

pub(crate) async fn get(State(state): State<Arc<AppState>>) -> Json<InitializationResponse> {
    Json(InitializationResponse {
        initialization_state: **state.infra.initialization.load(),
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdvanceRequest {
    initialization_state: InitializationState,
}

pub(crate) async fn advance(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AdvanceRequest>,
) -> Result<Json<InitializationResponse>, AppError> {
    let _initialization = state.runtime.initialization_lock.try_lock().map_err(|_| {
        AppError::Conflict(
            "Initialization is already being verified; retry when it finishes".into(),
        )
    })?;
    let current = **state.infra.initialization.load();
    validate_transition(current, request.initialization_state)?;
    if matches!(
        (current, request.initialization_state),
        (
            InitializationState::ProviderRequired,
            InitializationState::PersonalizationRequired
        )
    ) {
        require_usable_provider(&state).await?;
        let probe = super::health::probe_provider(&state).await?;
        if !probe.passed() {
            return Err(AppError::Validation(format!(
                "The selected provider could not complete a streamed response and diagnostic tool call: {}",
                probe
                    .error
                    .as_deref()
                    .unwrap_or("provider diagnostic failed"),
            )));
        }
        if super::providers::restart_required(&state) {
            return Err(AppError::Conflict(
                "Provider settings changed during verification; restart the daemon and retry setup"
                    .into(),
            ));
        }
    }
    crate::db::runtime_initialization_repo::set(&state.infra.db, request.initialization_state)
        .await?;
    state
        .infra
        .initialization
        .store(Arc::new(request.initialization_state));
    Ok(Json(InitializationResponse {
        initialization_state: request.initialization_state,
    }))
}

async fn require_usable_provider(state: &AppState) -> Result<(), AppError> {
    if super::providers::restart_required(state) {
        return Err(AppError::Validation(
            "provider changes require a daemon restart before setup can continue".into(),
        ));
    }
    let config = state.infra.config.load();
    let profile = config.routes.chat.profile.clone();
    drop(config);
    let status = state.auth.agent.auth_status(&profile).await?;
    if !status.is_authenticated {
        return Err(AppError::Validation(
            status
                .status_message
                .unwrap_or_else(|| "active provider is not authenticated".into()),
        ));
    }
    Ok(())
}

pub(crate) fn validate_transition(
    current: InitializationState,
    requested: InitializationState,
) -> Result<(), AppError> {
    use InitializationState::{PersonalizationRequired, ProviderRequired, Ready};
    if current == requested
        || matches!(
            (current, requested),
            (ProviderRequired, PersonalizationRequired) | (PersonalizationRequired, Ready)
        )
    {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "invalid initialization transition: {current} -> {requested}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_only_moves_forward_one_stage() {
        assert!(
            validate_transition(
                InitializationState::ProviderRequired,
                InitializationState::PersonalizationRequired
            )
            .is_ok()
        );
        assert!(
            validate_transition(
                InitializationState::PersonalizationRequired,
                InitializationState::Ready
            )
            .is_ok()
        );
        assert!(
            validate_transition(
                InitializationState::ProviderRequired,
                InitializationState::Ready
            )
            .is_err()
        );
        assert!(
            validate_transition(
                InitializationState::Ready,
                InitializationState::ProviderRequired
            )
            .is_err()
        );
    }
}
