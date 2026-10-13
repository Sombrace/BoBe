use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::api::principal::Principal;
use crate::app_state::AppState;
use crate::db::endpoint_repo::{EndpointRecord, EndpointUpsert};
use crate::error::AppError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointRequest {
    kind: String,
    label: String,
    room: Option<String>,
    certificate_sha256: Option<String>,
    protocol_min: i64,
    protocol_max: i64,
    #[serde(default = "default_capabilities")]
    capabilities: serde_json::Value,
}

fn default_capabilities() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisplayTextRequest {
    text: String,
}

pub(crate) async fn display_text(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<DisplayTextRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let request_id = state
        .body
        .gateway
        .send_device_text(&id, &request.text)
        .await?;
    Ok(Json(
        serde_json::json!({"request_id":request_id,"queued":true}),
    ))
}

pub(crate) async fn sensors(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::body::interactions::SensorSample>>, AppError> {
    let samples = state
        .body
        .gateway
        .device_sensor_snapshot(&id)
        .ok_or_else(|| AppError::Conflict("device is not connected".into()))?;
    Ok(Json(samples))
}

#[derive(serde::Serialize)]
pub(crate) struct EndpointResponse {
    id: String,
    kind: String,
    label: String,
    room: Option<String>,
    certificate_sha256: Option<String>,
    protocol_min: i64,
    protocol_max: i64,
    capabilities: serde_json::Value,
    observed_capabilities: serde_json::Value,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<EndpointRecord> for EndpointResponse {
    fn from(endpoint: EndpointRecord) -> Self {
        Self {
            id: endpoint.id,
            kind: endpoint.kind,
            label: endpoint.label,
            room: endpoint.room,
            certificate_sha256: endpoint
                .certificate_sha256
                .map(|digest| crate::util::text::hex_encode(&digest)),
            protocol_min: endpoint.protocol_min,
            protocol_max: endpoint.protocol_max,
            capabilities: serde_json::from_str(&endpoint.capabilities_json)
                .unwrap_or_else(|_| serde_json::json!({})),
            observed_capabilities: serde_json::from_str(&endpoint.observed_capabilities_json)
                .unwrap_or_else(|_| serde_json::json!({})),
            revoked_at: endpoint.revoked_at,
        }
    }
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<EndpointResponse>>, AppError> {
    Ok(Json(
        state
            .body
            .endpoints
            .list()
            .await?
            .into_iter()
            .map(EndpointResponse::from)
            .collect(),
    ))
}

pub(crate) async fn upsert(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<EndpointRequest>,
) -> Result<StatusCode, AppError> {
    let _guard = state.body.registry_lock.lock().await;
    let certificate_sha256 = request
        .certificate_sha256
        .as_deref()
        .map(decode_sha256)
        .transpose()?;
    state
        .body
        .endpoints
        .upsert(&EndpointUpsert {
            id: id.clone(),
            kind: request.kind,
            label: request.label,
            room: request.room,
            certificate_sha256,
            protocol_min: request.protocol_min,
            protocol_max: request.protocol_max,
            capabilities_json: serde_json::to_string(&request.capabilities)?,
        })
        .await?;
    state.apply_endpoint_registry_change(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn revoke(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    Ok(if state.revoke_endpoint(&id, "owner_requested").await? {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

pub(crate) async fn revoke_self(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
) -> Result<StatusCode, AppError> {
    let endpoint_id = principal.endpoint_id.as_deref().ok_or_else(|| {
        AppError::NotFound("this credential does not belong to a paired device".into())
    })?;
    Ok(
        if state.revoke_endpoint(endpoint_id, "self_requested").await? {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::NOT_FOUND
        },
    )
}

fn decode_sha256(value: &str) -> Result<Vec<u8>, AppError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::Config(
            "certificate_sha256 must be 64 hexadecimal characters".into(),
        ));
    }
    crate::util::text::hex_decode(value).ok_or_else(|| {
        AppError::Config("certificate_sha256 must be 64 hexadecimal characters".into())
    })
}
