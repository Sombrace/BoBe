use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::principal::{Principal, PrincipalKind};
use crate::api::readiness::{BlockingReason, Readiness, ReadinessStatus};
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
    Unsupported,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeploymentMode {
    Loopback,
    Networked,
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
    daemon_version: &'static str,
    negotiated_protocol: u32,
    deployment_mode: DeploymentMode,
    initialization_state: crate::db::runtime_initialization_repo::InitializationState,
    readiness: AttachReadiness,
    capabilities: RuntimeCapabilities,
    granted_scopes: Vec<Scope>,
}

#[derive(Debug, Serialize)]
struct AttachReadiness {
    state: ReadinessStatus,
    blocking_reasons: Vec<BlockingReason>,
}

impl From<Readiness> for AttachReadiness {
    fn from(readiness: Readiness) -> Self {
        Self {
            state: readiness.status,
            blocking_reasons: readiness.blocking_reasons,
        }
    }
}

#[derive(Debug, Serialize)]
struct RuntimeCapabilities {
    stt: CapabilityState,
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
            .update_observed_capabilities(endpoint_id, &capabilities)
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

    let readiness = Readiness::check(&state.infra).await;
    let deployment_mode = if state.config().server.is_networked() {
        DeploymentMode::Networked
    } else {
        DeploymentMode::Loopback
    };
    let stt = if state.voice.voice_engines.load().supports_daemon_stt() {
        CapabilityState::Ready
    } else {
        CapabilityState::Unsupported
    };
    let identity = &state.infra.identity;

    Ok(Json(HelloResponse {
        runtime_authority_id: identity.runtime_authority_id,
        runtime_authority_epoch: identity.runtime_authority_epoch,
        daemon_version: identity.daemon_version,
        negotiated_protocol: negotiated,
        deployment_mode,
        initialization_state: readiness.initialization_state,
        readiness: AttachReadiness::from(readiness),
        capabilities: RuntimeCapabilities { stt },
        granted_scopes,
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
    fn attach_reports_setup_in_progress_as_initializing() {
        use crate::db::runtime_initialization_repo::InitializationState;

        for initialization_state in [
            InitializationState::ProviderRequired,
            InitializationState::PersonalizationRequired,
        ] {
            assert_eq!(
                serde_json::to_value(AttachReadiness::from(Readiness::evaluate(
                    true,
                    initialization_state
                )))
                .unwrap(),
                serde_json::json!({
                    "state": "initializing",
                    "blocking_reasons": ["initialization_incomplete"],
                })
            );
        }
        assert_eq!(
            serde_json::to_value(AttachReadiness::from(Readiness::evaluate(
                true,
                InitializationState::Ready
            )))
            .unwrap(),
            serde_json::json!({"state": "ready", "blocking_reasons": []})
        );
        assert_eq!(
            serde_json::to_value(AttachReadiness::from(Readiness::evaluate(
                false,
                InitializationState::Ready
            )))
            .unwrap(),
            serde_json::json!({
                "state": "not_ready",
                "blocking_reasons": ["database_unavailable"],
            })
        );
    }

    #[test]
    fn the_attach_reply_carries_only_what_surfaces_read() {
        use crate::db::runtime_initialization_repo::InitializationState;

        let reply = serde_json::to_value(HelloResponse {
            runtime_authority_id: Uuid::from_u128(1),
            runtime_authority_epoch: 2,
            daemon_version: "0.9.1",
            negotiated_protocol: 1,
            deployment_mode: DeploymentMode::Loopback,
            initialization_state: InitializationState::Ready,
            readiness: AttachReadiness::from(Readiness::evaluate(true, InitializationState::Ready)),
            capabilities: RuntimeCapabilities {
                stt: CapabilityState::Unsupported,
            },
            granted_scopes: Vec::new(),
        })
        .unwrap();
        let mut fields: Vec<&str> = reply
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "capabilities",
                "daemon_version",
                "deployment_mode",
                "granted_scopes",
                "initialization_state",
                "negotiated_protocol",
                "readiness",
                "runtime_authority_epoch",
                "runtime_authority_id",
            ]
        );
        assert_eq!(
            reply["capabilities"],
            serde_json::json!({"stt": "unsupported"})
        );
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
