use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::principal::{Principal, PrincipalKind};
use crate::api::scope::Scope;
use crate::app_state::AppState;
use crate::error::AppError;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SurfaceKind {
    MacApp,
    Phone,
    Cli,
    PhysicalBody,
    Adapter,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityState {
    Ready,
    Degraded,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeploymentMode {
    Loopback,
    Networked,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthMode {
    EndpointCredential,
    OwnerToken,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SurfaceCapabilities {
    #[serde(default)]
    microphone: bool,
    #[serde(default)]
    playback: bool,
    #[serde(default)]
    wake: bool,
    #[serde(default)]
    local_stt: bool,
    #[serde(default)]
    local_tts: bool,
    #[serde(default)]
    screen_context: bool,
}

impl SurfaceCapabilities {
    pub(crate) fn validate_for(&self, kind: SurfaceKind) -> Result<(), AppError> {
        if matches!(kind, SurfaceKind::Phone)
            && (self.wake || self.local_tts || self.screen_context)
        {
            return Err(AppError::Validation(
                "a phone can declare only microphone, playback and local_stt".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelloRequest {
    surface_id: Uuid,
    surface_kind: SurfaceKind,
    surface_app_version: String,
    protocol_min: u32,
    protocol_max: u32,
    #[serde(default)]
    capabilities: SurfaceCapabilities,
    #[serde(default)]
    requested_scopes: Vec<Scope>,
}

#[derive(Debug, Serialize)]
pub(crate) struct HelloResponse {
    runtime_authority_id: Uuid,
    runtime_authority_epoch: i64,
    runtime_incarnation_id: Uuid,
    daemon_version: &'static str,
    schema_version: i64,
    started_at: chrono::DateTime<chrono::Utc>,
    protocol_min: u32,
    protocol_max: u32,
    negotiated_protocol: u32,
    deployment_mode: DeploymentMode,
    initialization_state: crate::db::runtime_initialization_repo::InitializationState,
    readiness: Readiness,
    capabilities: RuntimeCapabilities,
    granted_scopes: Vec<Scope>,
    auth_mode: AuthMode,
    events: EventsContract,
}

#[derive(Debug, Serialize)]
struct Readiness {
    state: &'static str,
    blocking_reasons: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct RuntimeCapabilities {
    agent: CapabilityState,
    stt: CapabilityState,
    tts: CapabilityState,
    capture: CapabilityState,
    body: CapabilityState,
    mcp: CapabilityState,
    events: CapabilityState,
}

#[derive(Debug, Serialize)]
struct EventsContract {
    subscription: &'static str,
    cursor_start: Option<u64>,
}

pub(crate) async fn hello(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<HelloRequest>,
) -> Result<Json<HelloResponse>, AppError> {
    validate_request(&request)?;
    let negotiated = negotiate(
        request.protocol_min,
        request.protocol_max,
        crate::constants::attach::PROTOCOL_MIN,
        crate::constants::attach::PROTOCOL_MAX,
    )?;
    let granted_scopes = principal.scopes.grant_requested(request.requested_scopes);
    if principal.kind == PrincipalKind::Endpoint {
        let endpoint_id = principal.endpoint_id.as_deref().ok_or_else(|| {
            AppError::Internal("authenticated endpoint principal has no endpoint id".into())
        })?;
        if !endpoint_matches_surface(endpoint_id, request.surface_id) {
            return Err(AppError::Unauthorized(
                "authenticated endpoint identity does not match the surface".into(),
            ));
        }
        let capabilities = serde_json::to_string(&request.capabilities)?;
        if !state
            .body
            .endpoints
            .record_authenticated_hello(endpoint_id, &capabilities)
            .await?
        {
            return Err(AppError::Unauthorized(
                "authenticated endpoint is no longer active".into(),
            ));
        }
    }
    tracing::info!(
        surface_id = %request.surface_id,
        surface_kind = ?request.surface_kind,
        principal_kind = ?principal.kind,
        app_version = request.surface_app_version,
        granted_scopes = ?granted_scopes,
        microphone = request.capabilities.microphone,
        playback = request.capabilities.playback,
        wake = request.capabilities.wake,
        local_stt = request.capabilities.local_stt,
        local_tts = request.capabilities.local_tts,
        screen_context = request.capabilities.screen_context,
        "attach.hello"
    );

    let db_ok = sqlx::query("SELECT 1")
        .fetch_one(&state.infra.db)
        .await
        .is_ok();
    let config = state.config();
    let identity = &state.infra.identity;
    let deployment_mode = if config.server.is_networked() {
        DeploymentMode::Networked
    } else {
        DeploymentMode::Loopback
    };
    let auth_mode = match principal.kind {
        PrincipalKind::Endpoint => AuthMode::EndpointCredential,
        PrincipalKind::Owner | PrincipalKind::Adapter => AuthMode::OwnerToken,
    };
    let engines = state.voice.voice_engines.load();
    let stt = if engines.supports_daemon_stt() {
        CapabilityState::Ready
    } else {
        CapabilityState::Unsupported
    };
    let tts = if engines.is_complete() {
        CapabilityState::Ready
    } else {
        CapabilityState::Unsupported
    };
    let body = if !config.body.enabled {
        CapabilityState::Unsupported
    } else if state.body.gateway.is_failed() {
        CapabilityState::Degraded
    } else {
        CapabilityState::Ready
    };

    Ok(Json(HelloResponse {
        runtime_authority_id: identity.runtime_authority_id,
        runtime_authority_epoch: identity.runtime_authority_epoch,
        runtime_incarnation_id: identity.runtime_incarnation_id,
        daemon_version: identity.daemon_version,
        schema_version: identity.schema_version,
        started_at: identity.started_at,
        protocol_min: crate::constants::attach::PROTOCOL_MIN,
        protocol_max: crate::constants::attach::PROTOCOL_MAX,
        negotiated_protocol: negotiated,
        deployment_mode,
        initialization_state: **state.infra.initialization.load(),
        readiness: Readiness {
            state: if db_ok { "ready" } else { "not_ready" },
            blocking_reasons: if db_ok {
                Vec::new()
            } else {
                vec!["database_unavailable"]
            },
        },
        capabilities: RuntimeCapabilities {
            agent: CapabilityState::Ready,
            stt,
            tts,
            capture: if cfg!(target_os = "macos") && config.capture.enabled {
                CapabilityState::Ready
            } else {
                CapabilityState::Unsupported
            },
            body,
            mcp: if config.mcp.enabled {
                CapabilityState::Ready
            } else {
                CapabilityState::Unsupported
            },
            events: CapabilityState::Ready,
        },
        granted_scopes,
        auth_mode,
        events: EventsContract {
            subscription: "resumable_broadcast",
            cursor_start: state.infra.event_queue.latest_event_id(),
        },
    }))
}

fn endpoint_matches_surface(endpoint_id: &str, surface_id: Uuid) -> bool {
    Uuid::parse_str(endpoint_id).is_ok_and(|endpoint_id| endpoint_id == surface_id)
}

fn validate_request(request: &HelloRequest) -> Result<(), AppError> {
    if request.protocol_min > request.protocol_max {
        return Err(AppError::Validation(
            "protocol_min must not exceed protocol_max".into(),
        ));
    }
    if request.surface_app_version.is_empty()
        || request.surface_app_version.len() > 64
        || !request
            .surface_app_version
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
    {
        return Err(AppError::Validation(
            "surface_app_version must be 1-64 printable ASCII characters".into(),
        ));
    }
    request.capabilities.validate_for(request.surface_kind)
}

fn negotiate(
    client_min: u32,
    client_max: u32,
    daemon_min: u32,
    daemon_max: u32,
) -> Result<u32, AppError> {
    let negotiated = client_max.min(daemon_max);
    if negotiated < client_min.max(daemon_min) {
        return Err(AppError::ProtocolIncompatible(format!(
            "client supports {client_min}-{client_max}; daemon supports {daemon_min}-{daemon_max}"
        )));
    }
    Ok(negotiated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_identity_comparison_accepts_uuid_casing() {
        let surface_id = Uuid::parse_str("D9C9D04A-7BEF-4747-BA22-C7D610E62612").unwrap();

        assert!(endpoint_matches_surface(
            "d9c9d04a-7bef-4747-ba22-c7d610e62612",
            surface_id
        ));
        assert!(!endpoint_matches_surface(
            "d9c9d04a-7bef-4747-ba22-c7d610e62613",
            surface_id
        ));
        assert!(!endpoint_matches_surface("not-a-uuid", surface_id));
    }

    #[test]
    fn protocol_negotiation_uses_highest_common_version() {
        assert_eq!(negotiate(1, 2, 1, 3).expect("overlap"), 2);
        assert!(negotiate(3, 4, 1, 2).is_err());
        assert!(negotiate(1, 0, 1, 1).is_err());
    }

    fn hello_request(surface_kind: &str, capabilities: &str) -> HelloRequest {
        serde_json::from_value(serde_json::json!({
            "surface_id": "11111111-2222-3333-4444-555555555555",
            "surface_kind": surface_kind,
            "surface_app_version": "1.0",
            "protocol_min": 1,
            "protocol_max": 1,
            "capabilities": serde_json::from_str::<serde_json::Value>(capabilities).unwrap(),
            "requested_scopes": ["conversation_read", "voice_session"],
        }))
        .unwrap()
    }

    #[test]
    fn a_phone_declares_its_microphone_playback_and_recognition() {
        let phone = hello_request(
            "phone",
            r#"{"microphone":true,"playback":true,"local_stt":true}"#,
        );
        assert!(matches!(phone.surface_kind, SurfaceKind::Phone));
        assert!(validate_request(&phone).is_ok());
        for capability in ["wake", "local_tts", "screen_context"] {
            let phone = hello_request("phone", &format!(r#"{{"{capability}":true}}"#));
            assert!(validate_request(&phone).is_err(), "{capability}");
        }
        let mac = hello_request(
            "mac_app",
            r#"{"microphone":true,"local_tts":true,"screen_context":true}"#,
        );
        assert!(validate_request(&mac).is_ok());
    }

    #[test]
    fn unknown_capability_field_is_rejected() {
        let json = r#"{
            "surface_id":"11111111-2222-3333-4444-555555555555",
            "surface_kind":"mac_app",
            "surface_app_version":"1.0",
            "protocol_min":1,
            "protocol_max":1,
            "capabilities":{"telepathy":true}
        }"#;
        assert!(serde_json::from_str::<HelloRequest>(json).is_err());
    }
}
