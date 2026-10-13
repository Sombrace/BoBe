use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{ConnectInfo, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::scope::Scope;
use crate::app_state::AppState;
use crate::db::endpoint_repo::EndpointUpsert;
use crate::error::AppError;
use crate::util::text::constant_time_eq;

pub(crate) struct LocalEnrollmentBootstrap {
    state: Arc<Mutex<BootstrapState>>,
}

#[derive(Clone, Copy)]
struct BootstrapState {
    verifier: Option<[u8; 32]>,
    reserved: bool,
}

struct BootstrapReservation {
    state: Arc<Mutex<BootstrapState>>,
    committed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalEnrollmentRequest {
    bootstrap_secret: String,
    endpoint_id: String,
    label: String,
    protocol_min: i64,
    protocol_max: i64,
    #[serde(default)]
    capabilities: serde_json::Value,
}

#[derive(Serialize)]
pub(crate) struct LocalEnrollmentResponse {
    endpoint_id: String,
    credential: String,
    runtime_authority_id: uuid::Uuid,
    runtime_authority_epoch: i64,
}

impl LocalEnrollmentBootstrap {
    pub(crate) fn disabled() -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(BootstrapState {
                verifier: None,
                reserved: false,
            })),
        })
    }

    pub(crate) fn from_secret(secret: &[u8]) -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(BootstrapState {
                verifier: Some(secret_verifier(secret)),
                reserved: false,
            })),
        })
    }

    fn reserve(&self, supplied: &[u8]) -> Option<BootstrapReservation> {
        let mut state = lock_or_recover(&self.state);
        let expected = state.verifier?;
        if state.reserved || !constant_time_eq(&expected, &secret_verifier(supplied)) {
            return None;
        }
        state.reserved = true;
        Some(BootstrapReservation {
            state: Arc::clone(&self.state),
            committed: false,
        })
    }
}

impl BootstrapReservation {
    fn commit(mut self) {
        let mut state = lock_or_recover(&self.state);
        state.verifier = None;
        state.reserved = false;
        self.committed = true;
    }
}

impl Drop for BootstrapReservation {
    fn drop(&mut self) {
        if !self.committed {
            lock_or_recover(&self.state).reserved = false;
        }
    }
}

pub(crate) async fn complete(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Json(request): Json<LocalEnrollmentRequest>,
) -> Result<Json<LocalEnrollmentResponse>, AppError> {
    let bootstrap = decode_bootstrap(&request.bootstrap_secret)?;
    let Some(reservation) = peer
        .ip()
        .is_loopback()
        .then(|| state.auth.local_enrollment.reserve(&bootstrap))
        .flatten()
    else {
        return Err(AppError::Unauthorized(
            "local enrollment is unavailable".into(),
        ));
    };
    let scopes = Scope::ALL.to_vec();
    let issued = state
        .body
        .trust
        .enroll_endpoint_with_bearer(
            &EndpointUpsert {
                id: request.endpoint_id,
                kind: "mac_app".into(),
                label: request.label,
                room: None,
                certificate_sha256: None,
                protocol_min: request.protocol_min,
                protocol_max: request.protocol_max,
                capabilities_json: serde_json::to_string(&request.capabilities)?,
            },
            &scopes,
            state.infra.identity.runtime_authority_epoch,
            "managed_local_bootstrap",
            Some(&peer.ip().to_string()),
        )
        .await?;
    state.disconnect_credentials(&issued.superseded_credential_ids);
    reservation.commit();
    Ok(Json(LocalEnrollmentResponse {
        endpoint_id: issued.endpoint_id,
        credential: issued.token,
        runtime_authority_id: state.infra.identity.runtime_authority_id,
        runtime_authority_epoch: state.infra.identity.runtime_authority_epoch,
    }))
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::error!("local enrollment mutex poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

fn decode_bootstrap(value: &str) -> Result<Vec<u8>, AppError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| AppError::Unauthorized("local enrollment is unavailable".into()))?;
    if bytes.len() != 32 {
        return Err(AppError::Unauthorized(
            "local enrollment is unavailable".into(),
        ));
    }
    Ok(bytes)
}

fn secret_verifier(secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"bobe.managed-local.bootstrap.v1\0");
    hasher.update(secret);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_reservation_releases_bootstrap_for_retry() {
        let bootstrap = LocalEnrollmentBootstrap::from_secret(b"managed-local-secret");
        let reservation = bootstrap.reserve(b"managed-local-secret").unwrap();
        assert!(bootstrap.reserve(b"managed-local-secret").is_none());
        drop(reservation);
        assert!(bootstrap.reserve(b"managed-local-secret").is_some());
    }

    #[test]
    fn committed_reservation_consumes_bootstrap() {
        let bootstrap = LocalEnrollmentBootstrap::from_secret(b"managed-local-secret");
        bootstrap.reserve(b"managed-local-secret").unwrap().commit();
        assert!(bootstrap.reserve(b"managed-local-secret").is_none());
    }

    #[test]
    fn wrong_or_different_length_secret_cannot_reserve() {
        let bootstrap = LocalEnrollmentBootstrap::from_secret(b"managed-local-secret");
        assert!(bootstrap.reserve(b"managed-local-wrong").is_none());
        assert!(bootstrap.reserve(b"short").is_none());
        assert!(bootstrap.reserve(b"managed-local-secret").is_some());
    }
}
