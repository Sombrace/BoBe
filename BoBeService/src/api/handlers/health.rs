use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::runtime::session::RuntimeStatus;

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    status: &'static str,
    services: ServiceHealth,
    version: &'static str,
    bodies: std::collections::HashMap<String, crate::body::gateway::BodyUpdateStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_offer_build: Option<u32>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ServiceHealth {
    database: &'static str,
    body_link: &'static str,
}

pub(crate) async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let db_ok = sqlx::query("SELECT 1")
        .fetch_one(&state.infra.db)
        .await
        .is_ok();
    let body_failed = state.body.gateway.is_failed();
    let status = if db_ok && !body_failed {
        "healthy"
    } else {
        "degraded"
    };

    Json(HealthResponse {
        status,
        services: ServiceHealth {
            database: if db_ok { "ok" } else { "error" },
            body_link: state.body.gateway.health_status(),
        },
        version: env!("CARGO_PKG_VERSION"),
        bodies: state.body.gateway.update_statuses(),
        body_offer_build: state.body.gateway.configured_offer_build(),
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct ReadinessResponse {
    status: &'static str,
    runtime_authority_id: uuid::Uuid,
    runtime_authority_epoch: i64,
    runtime_incarnation_id: uuid::Uuid,
    daemon_version: &'static str,
    schema_version: i64,
    initialization_state: crate::db::runtime_initialization_repo::InitializationState,
    started_at: chrono::DateTime<chrono::Utc>,
    blocking_reasons: Vec<&'static str>,
}

pub(crate) async fn readiness(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1")
        .fetch_one(&state.infra.db)
        .await
        .is_ok();
    let identity = &state.infra.identity;
    let initialization_state = **state.infra.initialization.load();
    let (status, blocking_reasons) = readiness_state(db_ok, initialization_state);
    let response = ReadinessResponse {
        status,
        runtime_authority_id: identity.runtime_authority_id,
        runtime_authority_epoch: identity.runtime_authority_epoch,
        runtime_incarnation_id: identity.runtime_incarnation_id,
        daemon_version: identity.daemon_version,
        schema_version: identity.schema_version,
        initialization_state,
        started_at: identity.started_at,
        blocking_reasons,
    };
    (
        if db_ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(response),
    )
}

fn readiness_state(
    database_available: bool,
    initialization_state: crate::db::runtime_initialization_repo::InitializationState,
) -> (&'static str, Vec<&'static str>) {
    if !database_available {
        return ("not_ready", vec!["database_unavailable"]);
    }
    if initialization_state != crate::db::runtime_initialization_repo::InitializationState::Ready {
        return ("initializing", vec!["initialization_incomplete"]);
    }
    ("ready", Vec::new())
}

#[derive(Debug, serde::Deserialize, Serialize)]
pub(crate) struct ProviderDiagnosticResponse {
    pub(crate) ok: bool,
    pub(crate) profile: crate::provider::ProfileId,
    pub(crate) model: Option<String>,
    pub(crate) runtime_authority_id: uuid::Uuid,
    pub(crate) runtime_incarnation_id: uuid::Uuid,
    #[serde(flatten)]
    pub(crate) checks: crate::provider::diagnostics::ProviderProbe,
}

pub(crate) async fn diagnose_provider(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProviderDiagnosticResponse>, crate::error::AppError> {
    let checks = probe_provider(&state).await?;
    let route = &state.runtime.active_routes.chat;
    Ok(Json(ProviderDiagnosticResponse {
        ok: checks.passed(),
        profile: route.profile.clone(),
        model: route.model.clone(),
        runtime_authority_id: state.infra.identity.runtime_authority_id,
        runtime_incarnation_id: state.infra.identity.runtime_incarnation_id,
        checks,
    }))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderProbeRequest {
    model: String,
    reasoning_effort: Option<String>,
}

pub(crate) async fn diagnose_profile(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(profile): axum::extract::Path<String>,
    Json(request): Json<ProviderProbeRequest>,
) -> Result<Json<ProviderDiagnosticResponse>, crate::error::AppError> {
    use crate::error::AppError;
    let profile = crate::provider::ProfileId::parse(&profile)?;
    if !state.config().providers.profiles.contains_key(&profile) {
        return Err(AppError::NotFound("provider profile was not found".into()));
    }
    super::providers::require_live_profile(&state, &profile)?;
    if request.model.trim().is_empty() || request.model.len() > 256 {
        return Err(AppError::Validation(
            "Select a provider model to test".into(),
        ));
    }
    let _personal_data = Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_read_owned()
        .map_err(|_| AppError::Conflict("agent maintenance is active".into()))?;
    let _probe = crate::util::atomic_flag_guard::AtomicFlagGuard::try_acquire(Arc::clone(
        &state.runtime.provider_probe_active,
    ))
    .ok_or_else(|| AppError::Conflict("a provider diagnostic is already running".into()))?;
    let route = crate::provider::WorkloadRoute {
        profile: profile.clone(),
        model: Some(request.model.clone()),
        reasoning_effort: request.reasoning_effort,
    };
    let checks = state.auth.agent.probe_profile(route).await?;
    Ok(Json(ProviderDiagnosticResponse {
        ok: checks.passed(),
        profile,
        model: Some(request.model),
        runtime_authority_id: state.infra.identity.runtime_authority_id,
        runtime_incarnation_id: state.infra.identity.runtime_incarnation_id,
        checks,
    }))
}

pub(super) async fn probe_provider(
    state: &AppState,
) -> Result<crate::provider::diagnostics::ProviderProbe, crate::error::AppError> {
    if super::providers::restart_required(state) {
        return Err(crate::error::AppError::Conflict(
            "Restart the daemon to apply provider changes before testing the selected model".into(),
        ));
    }
    let _personal_data = Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_read_owned()
        .map_err(|_| crate::error::AppError::Conflict("agent maintenance is active".into()))?;
    let _probe = crate::util::atomic_flag_guard::AtomicFlagGuard::try_acquire(Arc::clone(
        &state.runtime.provider_probe_active,
    ))
    .ok_or_else(|| {
        crate::error::AppError::Conflict("a provider diagnostic is already running".into())
    })?;
    let checks = match state.runtime.workers.probe_chat_provider().await {
        Ok(checks) => checks,
        Err(error) => crate::provider::diagnostics::ProviderProbe {
            error: Some(error.to_string()),
            ..Default::default()
        },
    };
    Ok(checks)
}

#[derive(Debug, Serialize)]
pub(crate) struct StatusResponse {
    #[serde(flatten)]
    runtime: RuntimeStatus,
    version: &'static str,
}

pub(crate) async fn get_status(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(principal): axum::extract::Extension<crate::api::principal::Principal>,
) -> Json<StatusResponse> {
    Json(StatusResponse {
        runtime: state
            .runtime
            .runtime_session
            .get_status(principal.conversation_channel()),
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::runtime_initialization_repo::InitializationState;

    #[test]
    fn operational_readiness_requires_completed_initialization() {
        assert_eq!(
            readiness_state(true, InitializationState::ProviderRequired),
            ("initializing", vec!["initialization_incomplete"])
        );
        assert_eq!(
            readiness_state(true, InitializationState::PersonalizationRequired),
            ("initializing", vec!["initialization_incomplete"])
        );
        assert_eq!(
            readiness_state(true, InitializationState::Ready),
            ("ready", Vec::new())
        );
        assert_eq!(
            readiness_state(false, InitializationState::Ready),
            ("not_ready", vec!["database_unavailable"])
        );
    }
}
