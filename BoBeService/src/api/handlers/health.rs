use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use std::sync::Arc;

use crate::api::readiness::Readiness;
use crate::app_state::AppState;
use crate::db::runtime_authority_repo::RuntimeIdentity;
use crate::runtime::session::RuntimeStatus;

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    status: &'static str,
    services: ServiceHealth,
    version: &'static str,
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
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct ReadinessResponse {
    #[serde(flatten)]
    readiness: Readiness,
    runtime_authority_id: uuid::Uuid,
    runtime_authority_epoch: i64,
    runtime_incarnation_id: uuid::Uuid,
    daemon_version: &'static str,
    schema_version: i64,
    started_at: chrono::DateTime<chrono::Utc>,
}

impl ReadinessResponse {
    pub(crate) fn new(readiness: Readiness, identity: &RuntimeIdentity) -> Self {
        Self {
            readiness,
            runtime_authority_id: identity.runtime_authority_id,
            runtime_authority_epoch: identity.runtime_authority_epoch,
            runtime_incarnation_id: identity.runtime_incarnation_id,
            daemon_version: identity.daemon_version,
            schema_version: identity.schema_version,
            started_at: identity.started_at,
        }
    }
}

pub(crate) async fn readiness(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let readiness = Readiness::check(&state.infra).await;
    let status = if readiness.status.is_serving() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(ReadinessResponse::new(readiness, &state.infra.identity)),
    )
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
