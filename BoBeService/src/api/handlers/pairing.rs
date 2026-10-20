use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::api::handlers::hello::{SurfaceCapabilities, SurfaceKind};
use crate::api::pairing::{
    FinishAttempt, Finished, PairingReservation, SealedEnrollment, pairing_unavailable,
};
use crate::api::scope::ScopeProfile;
use crate::app_state::AppState;
use crate::db::endpoint_repo::EndpointUpsert;
use crate::error::AppError;

const PAIRING_PROTOCOL: &str = "bobe-pairing-opaque-ristretto255-v1";
const ENROLLMENT_INFO: &[u8] = b"bobe.pairing.enrollment.aead.v1";
const TRANSPORT_HTTPS: &str = "https";
const TRANSPORT_LOOPBACK_HTTP: &str = "loopback_http";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairingBeginRequest {
    origin: String,
    tls_spki_sha256: String,
    request: String,
    client_nonce: String,
}

#[derive(Serialize)]
pub(crate) struct PairingBeginResponse {
    exchange_id: Uuid,
    response: String,
    server_nonce: String,
    runtime_authority_id: Uuid,
    runtime_authority_epoch: i64,
    tls_spki_sha256: String,
    context: String,
    protocol: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairingCompleteRequest {
    exchange_id: Uuid,
    origin: String,
    tls_spki_sha256: String,
    client_nonce: String,
    server_nonce: String,
    finalization: String,
    nonce: String,
    enrollment: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentRequest {
    endpoint_id: String,
    label: String,
    protocol_min: i64,
    protocol_max: i64,
    #[serde(default)]
    capabilities: serde_json::Value,
}

#[derive(Serialize)]
pub(crate) struct PairingCompleteResponse {
    nonce: String,
    enrollment: String,
}

impl From<SealedEnrollment> for PairingCompleteResponse {
    fn from(sealed: SealedEnrollment) -> Self {
        Self {
            nonce: encode(&sealed.nonce),
            enrollment: encode(&sealed.ciphertext),
        }
    }
}

#[derive(Serialize)]
struct EnrollmentResponse {
    endpoint_id: String,
    credential: String,
    granted_scopes: Vec<&'static str>,
    runtime_authority_id: Uuid,
    runtime_authority_epoch: i64,
}

struct CanonicalPairingOrigin {
    value: String,
    transport: &'static str,
}

pub(crate) async fn begin(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Json(request): Json<PairingBeginRequest>,
) -> Result<Json<PairingBeginResponse>, AppError> {
    let origin = canonical_pairing_origin(&request.origin)?;
    let active_spki =
        validate_transport_identity(&state, &origin, &request.tls_spki_sha256).await?;
    let client_nonce = decode_fixed::<32>(&request.client_nonce)?;
    let profile = state
        .auth
        .pairing
        .invitation_profile()
        .ok_or_else(pairing_unavailable)?;
    let server_nonce: [u8; 32] = rand::random();
    let context = pairing_context(
        &state.infra.identity,
        profile,
        &origin,
        &active_spki,
        &client_nonce,
        &server_nonce,
    )?;
    let opaque_request = decode(&request.request)?;
    let result = state
        .auth
        .pairing
        .begin(profile, &opaque_request, &context, peer.ip())?;
    Ok(Json(PairingBeginResponse {
        exchange_id: result.exchange_id,
        response: encode(&result.response),
        server_nonce: encode(&server_nonce),
        runtime_authority_id: state.infra.identity.runtime_authority_id,
        runtime_authority_epoch: state.infra.identity.runtime_authority_epoch,
        tls_spki_sha256: active_spki,
        context: encode(&context),
        protocol: PAIRING_PROTOCOL,
    }))
}

pub(crate) async fn complete(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Json(request): Json<PairingCompleteRequest>,
) -> Result<Json<PairingCompleteResponse>, AppError> {
    let origin = canonical_pairing_origin(&request.origin)?;
    let active_spki =
        validate_transport_identity(&state, &origin, &request.tls_spki_sha256).await?;
    let client_nonce = decode_fixed::<32>(&request.client_nonce)?;
    let server_nonce = decode_fixed::<32>(&request.server_nonce)?;
    let profile = state
        .auth
        .pairing
        .completion_profile(request.exchange_id)
        .ok_or_else(pairing_unavailable)?;
    let context = pairing_context(
        &state.infra.identity,
        profile,
        &origin,
        &active_spki,
        &client_nonce,
        &server_nonce,
    )?;
    let finalization = decode(&request.finalization)?;
    let nonce = decode_fixed::<12>(&request.nonce)?;
    let enrollment = decode(&request.enrollment)?;
    let reservation = match state.auth.pairing.finish(&FinishAttempt {
        exchange_id: request.exchange_id,
        profile,
        finalization: &finalization,
        context: &context,
        nonce: &nonce,
        enrollment: &enrollment,
    })? {
        Finished::Delivered(sealed) => return Ok(Json(sealed.into())),
        Finished::Reserved(reservation) => reservation,
    };
    let sealed = enroll(&state, &reservation, peer, &context, nonce, &enrollment).await?;
    reservation.deliver(sealed.clone());
    Ok(Json(sealed.into()))
}

async fn enroll(
    state: &AppState,
    reservation: &PairingReservation,
    peer: std::net::SocketAddr,
    context: &[u8],
    nonce: [u8; 12],
    encrypted_enrollment: &[u8],
) -> Result<SealedEnrollment, AppError> {
    let cipher = enrollment_cipher(&reservation.session_key)?;
    let enrollment = cipher
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: encrypted_enrollment,
                aad: context,
            },
        )
        .map_err(|_| pairing_unavailable())?;
    let enrollment: EnrollmentRequest =
        serde_json::from_slice(&enrollment).map_err(|_| pairing_unavailable())?;
    let label = if enrollment.label.trim().is_empty() {
        reservation.label.clone()
    } else {
        enrollment.label
    };
    let scopes = reservation.profile.scopes();
    let source = peer.ip().to_string();
    let issued = state
        .body
        .trust
        .enroll_endpoint_with_bearer(
            &EndpointUpsert {
                id: enrollment.endpoint_id,
                kind: reservation.profile.endpoint_kind().into(),
                label,
                room: None,
                certificate_sha256: None,
                protocol_min: enrollment.protocol_min,
                protocol_max: enrollment.protocol_max,
                capabilities_json: enrolled_capabilities(
                    reservation.profile,
                    enrollment.capabilities,
                )?,
            },
            scopes,
            state.infra.identity.runtime_authority_epoch,
            "pairing",
            Some(&source),
        )
        .await?;
    state.disconnect_credentials(&issued.superseded_credential_ids);
    let response = EnrollmentResponse {
        endpoint_id: issued.endpoint_id,
        credential: issued.token,
        granted_scopes: scopes.iter().map(|scope| scope.as_str()).collect(),
        runtime_authority_id: state.infra.identity.runtime_authority_id,
        runtime_authority_epoch: state.infra.identity.runtime_authority_epoch,
    };
    let response_nonce: [u8; 12] = rand::random();
    let ciphertext = cipher
        .encrypt(
            &Nonce::from(response_nonce),
            Payload {
                msg: &serde_json::to_vec(&response)?,
                aad: context,
            },
        )
        .map_err(|_| AppError::Internal("encrypt pairing result".into()))?;
    Ok(SealedEnrollment {
        nonce: response_nonce,
        ciphertext,
    })
}

fn canonical_pairing_origin(raw: &str) -> Result<CanonicalPairingOrigin, AppError> {
    let mut url = url::Url::parse(raw).map_err(|_| pairing_unavailable())?;
    if url.host_str().is_none()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(pairing_unavailable());
    }
    let loopback = match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    let transport = match url.scheme() {
        "https" => TRANSPORT_HTTPS,
        "http" if loopback => TRANSPORT_LOOPBACK_HTTP,
        _ => return Err(pairing_unavailable()),
    };
    url.set_path("");
    let value = url.origin().ascii_serialization();
    Ok(CanonicalPairingOrigin { value, transport })
}

fn enrolled_capabilities(
    profile: ScopeProfile,
    declared: serde_json::Value,
) -> Result<String, AppError> {
    let surface = match profile {
        ScopeProfile::MacApp => return Ok(serde_json::to_string(&declared)?),
        ScopeProfile::Phone => SurfaceKind::Phone,
    };
    let capabilities = if declared.is_null() {
        SurfaceCapabilities::default()
    } else {
        serde_json::from_value::<SurfaceCapabilities>(declared).map_err(|error| {
            AppError::Validation(format!("invalid {} capabilities: {error}", profile.kind()))
        })?
    };
    capabilities.validate_for(surface)?;
    let mut value = serde_json::to_value(capabilities)?;
    if let Some(fields) = value.as_object_mut() {
        fields.insert("scope_profile".into(), profile.id().into());
    }
    Ok(value.to_string())
}

fn pairing_context(
    identity: &crate::db::runtime_authority_repo::RuntimeIdentity,
    profile: ScopeProfile,
    origin: &CanonicalPairingOrigin,
    spki: &str,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
) -> Result<Vec<u8>, AppError> {
    let value = serde_json::json!({
        "protocol": PAIRING_PROTOCOL,
        "client_role": "endpoint",
        "server_role": "authority",
        "authority_id": identity.runtime_authority_id,
        "authority_epoch": identity.runtime_authority_epoch,
        "origin": origin.value,
        "transport": origin.transport,
        "tls_spki_sha256": spki,
        "client_nonce": encode(client_nonce),
        "server_nonce": encode(server_nonce),
        "scope_profile": profile.id()
    });
    serde_json::to_vec(&value).map_err(Into::into)
}

async fn validate_transport_identity(
    state: &AppState,
    origin: &CanonicalPairingOrigin,
    supplied_spki: &str,
) -> Result<String, AppError> {
    if origin.transport == TRANSPORT_LOOPBACK_HTTP {
        if supplied_spki.is_empty() {
            return Ok(String::new());
        }
        return Err(pairing_unavailable());
    }
    let certificate_path = state.config().server.tls_cert_path.clone();
    let active_spki = active_tls_spki(certificate_path.as_deref()).await?;
    if supplied_spki == active_spki {
        Ok(active_spki)
    } else {
        Err(pairing_unavailable())
    }
}

async fn active_tls_spki(certificate_path: Option<&str>) -> Result<String, AppError> {
    let certificate_path = certificate_path
        .ok_or_else(|| AppError::Config("pairing requires an active TLS certificate".into()))?;
    crate::util::automatic_tls::spki_sha256_from_certificate_pem(
        &tokio::fs::read(certificate_path).await?,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvitationRequest {
    kind: String,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PairingInvitation {
    code: String,
    expires_in_seconds: u64,
    kind: &'static str,
    label: String,
    #[serde(flatten)]
    phone: Option<PhoneInvitation>,
}

#[derive(Debug, Serialize)]
struct PhoneInvitation {
    link: String,
    pin: String,
    origins: Vec<String>,
}

pub(crate) async fn create_invitation(
    State(state): State<Arc<AppState>>,
    Json(request): Json<InvitationRequest>,
) -> Result<Json<PairingInvitation>, AppError> {
    let profile = ScopeProfile::from_kind(&request.kind)
        .filter(|profile| *profile == ScopeProfile::Phone)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "{} cannot be invited here; only a phone can",
                request.kind
            ))
        })?;
    let label = request
        .label
        .unwrap_or_else(|| profile.default_label().to_owned());
    let server = state.config().server.clone();
    Ok(Json(
        invitation(&state.auth.pairing, &server, profile, &label).await?,
    ))
}

pub(crate) async fn invitation(
    pairing: &crate::api::pairing::PairingManager,
    server: &crate::config::ServerConfig,
    profile: ScopeProfile,
    label: &str,
) -> Result<PairingInvitation, AppError> {
    let link = match profile {
        ScopeProfile::MacApp => None,
        ScopeProfile::Phone => Some(phone_link(server).await?),
    };
    let created = pairing.create_invitation(profile, label)?;
    Ok(PairingInvitation {
        phone: link.map(|link| PhoneInvitation {
            link: link.url(&created.code),
            pin: link.pin,
            origins: link.origins,
        }),
        code: created.code,
        expires_in_seconds: created.expires_in_seconds,
        kind: profile.kind(),
        label: created.label,
    })
}

struct PhoneLink {
    pin: String,
    origins: Vec<String>,
}

impl PhoneLink {
    fn url(&self, code: &str) -> String {
        let mut url = format!("bobe://pair?v=1&c={code}&p={}", self.pin);
        for origin in &self.origins {
            url.push_str("&o=");
            url.push_str(&link_value(origin));
        }
        url
    }
}

async fn phone_link(server: &crate::config::ServerConfig) -> Result<PhoneLink, AppError> {
    if !server.is_networked() {
        return Err(phone_unreachable("it listens only on this computer"));
    }
    let origins = phone_origins(server);
    if origins.is_empty() {
        return Err(phone_unreachable(
            "none of its allowed hosts can be reached from another device",
        ));
    }
    let spki = active_tls_spki(server.tls_cert_path.as_deref()).await?;
    let digest = crate::util::text::hex_decode(&spki)
        .ok_or_else(|| AppError::Internal("TLS key pin is not hexadecimal".into()))?;
    Ok(PhoneLink {
        pin: encode(&digest),
        origins,
    })
}

fn phone_unreachable(reason: &str) -> AppError {
    AppError::NotNetworked(format!(
        "A phone cannot reach this BoBe because {reason}. Turn on remote access with \
         `bobe config remote --host 0.0.0.0 --allowed-host <this computer's address>`, \
         restart BoBe, then pair again."
    ))
}

fn phone_origins(server: &crate::config::ServerConfig) -> Vec<String> {
    let mut origins: Vec<String> = Vec::new();
    for host in server
        .allowed_hosts
        .iter()
        .chain(std::iter::once(&server.host))
    {
        if let Some(origin) = phone_origin(host, server.port)
            && !origins.contains(&origin)
        {
            origins.push(origin);
        }
    }
    origins
}

fn phone_origin(host: &str, port: u16) -> Option<String> {
    let host = host.trim();
    let authority = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let url = url::Url::parse(&format!("https://{authority}:{port}/")).ok()?;
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let reachable = match url.host()? {
        url::Host::Domain(domain) => domain != "localhost" && !domain.ends_with(".localhost"),
        url::Host::Ipv4(address) => !address.is_loopback() && !address.is_unspecified(),
        url::Host::Ipv6(address) => !address.is_loopback() && !address.is_unspecified(),
    };
    let host = url.host_str()?;
    reachable.then(|| format!("{host}:{port}"))
}

fn link_value(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b':') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

fn enrollment_cipher(session_key: &[u8]) -> Result<ChaCha20Poly1305, AppError> {
    let mut key = Zeroizing::new([0_u8; 32]);
    Hkdf::<Sha256>::new(None, session_key)
        .expand(ENROLLMENT_INFO, key.as_mut_slice())
        .map_err(|_| AppError::Internal("derive pairing encryption key".into()))?;
    Ok(ChaCha20Poly1305::new((&*key).into()))
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn decode(value: &str) -> Result<Vec<u8>, AppError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| pairing_unavailable())
}

fn decode_fixed<const N: usize>(value: &str) -> Result<[u8; N], AppError> {
    let mut bytes = [0_u8; N];
    match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode_slice(value, &mut bytes) {
        Ok(written) if written == N => Ok(bytes),
        _ => Err(pairing_unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_root_origins_and_marks_transport() {
        let remote = canonical_pairing_origin("https://EXAMPLE.test:443/").unwrap();
        assert_eq!(remote.value, "https://example.test");
        assert_eq!(remote.transport, TRANSPORT_HTTPS);
        let local = canonical_pairing_origin("http://127.0.0.2:8766/").unwrap();
        assert_eq!(local.value, "http://127.0.0.2:8766");
        assert_eq!(local.transport, TRANSPORT_LOOPBACK_HTTP);
    }

    #[test]
    fn rejects_paths_and_non_loopback_plaintext() {
        assert!(canonical_pairing_origin("https://example.test/api").is_err());
        assert!(canonical_pairing_origin("https://example.test/?x=1").is_err());
        assert!(canonical_pairing_origin("http://example.test").is_err());
    }

    #[test]
    fn the_transcript_binds_the_invitations_scope_profile() {
        let identity = crate::db::runtime_authority_repo::RuntimeIdentity {
            runtime_authority_id: Uuid::from_u128(7),
            runtime_authority_epoch: 3,
            runtime_incarnation_id: Uuid::from_u128(8),
            daemon_version: "test",
            schema_version: 1,
            started_at: chrono::Utc::now(),
        };
        let origin = canonical_pairing_origin("https://bobe.example.net:8766/").unwrap();
        let context = |profile| -> serde_json::Value {
            serde_json::from_slice(
                &pairing_context(&identity, profile, &origin, "ab", &[1; 32], &[2; 32]).unwrap(),
            )
            .unwrap()
        };
        let mac = context(ScopeProfile::MacApp);
        let mut phone = context(ScopeProfile::Phone);
        assert_eq!(mac["scope_profile"], "mac_app.v1");
        assert_eq!(phone["scope_profile"], "phone.v1");
        let mut keys: Vec<&str> = mac
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "authority_epoch",
                "authority_id",
                "client_nonce",
                "client_role",
                "origin",
                "protocol",
                "scope_profile",
                "server_nonce",
                "server_role",
                "tls_spki_sha256",
                "transport",
            ]
        );
        phone["scope_profile"] = mac["scope_profile"].clone();
        assert_eq!(phone, mac);
    }

    #[tokio::test]
    async fn a_phone_invitation_returns_its_code_pin_origins_and_link() {
        let root = std::env::temp_dir().join(format!("bobe-pairing-{}", Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}/data.db", root.display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let identity = crate::util::automatic_tls::ensure(&paths, &["bobe.example.net".into()])
            .await
            .unwrap();
        let server = crate::config::ServerConfig {
            host: "0.0.0.0".into(),
            allowed_hosts: vec!["bobe.example.net".into(), "192.168.1.5".into()],
            tls_cert_path: Some(identity.certificate_path.to_string_lossy().into_owned()),
            tls_key_path: Some(identity.private_key_path.to_string_lossy().into_owned()),
            ..crate::config::ServerConfig::default()
        };
        let pairing = crate::api::pairing::PairingManager::new();
        let phone = serde_json::to_value(
            invitation(&pairing, &server, ScopeProfile::Phone, "iPhone")
                .await
                .unwrap(),
        )
        .unwrap();
        let code = phone["code"].as_str().unwrap();
        let pin = encode(&crate::util::text::hex_decode(&identity.spki_sha256).unwrap());
        assert_eq!(code.len(), 6);
        assert_eq!(pin.len(), 43);
        assert_eq!(
            phone,
            serde_json::json!({
                "code": code,
                "expires_in_seconds": 300,
                "kind": "phone",
                "label": "iPhone",
                "pin": pin,
                "origins": ["bobe.example.net:8766", "192.168.1.5:8766"],
                "link": format!(
                    "bobe://pair?v=1&c={code}&p={pin}&o=bobe.example.net:8766&o=192.168.1.5:8766"
                ),
            })
        );

        let loopback = crate::config::ServerConfig::default();
        assert!(matches!(
            invitation(&pairing, &loopback, ScopeProfile::Phone, "iPhone").await,
            Err(AppError::NotNetworked(_))
        ));
        assert_eq!(pairing.invitation_profile(), Some(ScopeProfile::Phone));
        let mac = serde_json::to_value(
            invitation(&pairing, &loopback, ScopeProfile::MacApp, "Mac")
                .await
                .unwrap(),
        )
        .unwrap();
        let mut fields: Vec<&str> = mac
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, ["code", "expires_in_seconds", "kind", "label"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_phone_is_offered_every_allowed_host_it_could_reach_in_order() {
        let mut server = crate::config::ServerConfig {
            host: "0.0.0.0".into(),
            allowed_hosts: vec![
                "Bobe.Example.NET".into(),
                "192.168.1.5".into(),
                "localhost".into(),
                "fd7a:115c::1".into(),
                "[fd7a:115c::2]".into(),
                "127.0.0.1".into(),
                "bobe.example.net".into(),
                "bobe.example.net:443".into(),
                "https://bobe.example.net".into(),
            ],
            ..crate::config::ServerConfig::default()
        };
        assert_eq!(
            phone_origins(&server),
            [
                "bobe.example.net:8766",
                "192.168.1.5:8766",
                "[fd7a:115c::1]:8766",
                "[fd7a:115c::2]:8766",
            ]
        );
        server.host = "100.101.102.103".into();
        server.port = 443;
        server.allowed_hosts = vec!["bobe.tail1234.ts.net".into()];
        assert_eq!(
            phone_origins(&server),
            ["bobe.tail1234.ts.net:443", "100.101.102.103:443"]
        );
        server.allowed_hosts = vec!["localhost".into()];
        server.host = "127.0.0.1".into();
        assert_eq!(phone_origins(&server), Vec::<String>::new());
    }

    #[test]
    fn the_pairing_link_carries_version_code_pin_and_every_origin() {
        let link = PhoneLink {
            pin: encode(&[0xab; 32]),
            origins: vec!["192.168.1.5:8766".into(), "[fd7a:115c::1]:8766".into()],
        }
        .url("012345");
        assert_eq!(
            link,
            "bobe://pair?v=1&c=012345&p=q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s\
             &o=192.168.1.5:8766&o=%5Bfd7a:115c::1%5D:8766"
        );
        let parsed = url::Url::parse(&link).unwrap();
        assert_eq!(parsed.scheme(), "bobe");
        assert_eq!(parsed.host_str(), Some("pair"));
        let fields: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            fields,
            [
                ("v".into(), "1".into()),
                ("c".into(), "012345".into()),
                ("p".into(), encode(&[0xab; 32])),
                ("o".into(), "192.168.1.5:8766".into()),
                ("o".into(), "[fd7a:115c::1]:8766".into()),
            ]
        );
        assert_eq!(fields[2].1.len(), 43);
    }

    #[test]
    fn a_paired_phone_records_its_profile_and_only_phone_capabilities() {
        let stored: serde_json::Value = serde_json::from_str(
            &enrolled_capabilities(
                ScopeProfile::Phone,
                serde_json::json!({"microphone": true, "playback": true}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(stored["scope_profile"], "phone.v1");
        assert_eq!(stored["microphone"], true);
        assert_eq!(stored["screen_context"], false);
        assert!(
            enrolled_capabilities(
                ScopeProfile::Phone,
                serde_json::json!({"screen_context": true})
            )
            .is_err()
        );
        assert!(
            enrolled_capabilities(ScopeProfile::Phone, serde_json::json!({"telepathy": true}))
                .is_err()
        );
        assert_eq!(
            enrolled_capabilities(
                ScopeProfile::MacApp,
                serde_json::json!({"screen_context": true})
            )
            .unwrap(),
            r#"{"screen_context":true}"#
        );
    }
}
