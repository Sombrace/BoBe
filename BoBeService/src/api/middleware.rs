use std::collections::HashSet;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use secrecy::{ExposeSecret, SecretString};

use crate::api::principal::Principal;
use crate::api::scope::Scope;
use crate::constants::MILLIS_PER_SECOND;
use crate::util::text::constant_time_eq;
use crate::{app_state::AppState, error::AppError};

pub(crate) async fn personal_data_exclusion(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    if !is_personal_data_mutation(request.method(), request.uri().path()) {
        return Ok(next.run(request).await);
    }
    let _guard = state
        .runtime
        .runtime_session
        .try_begin_personal_data_access()
        .map_err(|message| AppError::Conflict(message.into()))?;
    Ok(next.run(request).await)
}

fn is_personal_data_mutation(method: &Method, path: &str) -> bool {
    *method != Method::GET
        && [
            "/goals",
            "/memory",
            "/souls",
            "/user-profiles",
            "/skills",
            "/workspace",
            "/tools/mcp/config",
        ]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

#[cfg(test)]
mod personal_data_tests {
    use super::*;

    #[test]
    fn gates_personal_mutations_but_not_reads_or_operational_settings() {
        assert!(is_personal_data_mutation(&Method::PUT, "/memory"));
        assert!(is_personal_data_mutation(
            &Method::DELETE,
            "/goals/00000000-0000-0000-0000-000000000000"
        ));
        assert!(is_personal_data_mutation(
            &Method::POST,
            "/tools/mcp/config/validate"
        ));
        assert!(!is_personal_data_mutation(&Method::GET, "/memory"));
        assert!(!is_personal_data_mutation(&Method::PATCH, "/settings"));
        assert!(!is_personal_data_mutation(&Method::POST, "/message"));
    }
}

#[derive(Clone)]
pub(crate) struct AllowedHosts {
    hosts: Arc<HashSet<String>>,
}

impl AllowedHosts {
    pub(crate) fn new(host: &str, port: u16, configured: &[String]) -> Self {
        let mut set = HashSet::new();
        for value in [host, "localhost", "127.0.0.1", "[::1]"]
            .into_iter()
            .chain(configured.iter().map(String::as_str))
        {
            set.insert(value.to_lowercase());
            set.insert(format!("{}:{port}", value.to_lowercase()));
        }

        Self {
            hosts: Arc::new(set),
        }
    }
}

#[derive(Clone)]
pub(crate) struct AllowedOrigins {
    origins: Arc<HashSet<HeaderValue>>,
}

impl AllowedOrigins {
    pub(crate) fn new(configured: &[String]) -> Self {
        Self {
            origins: Arc::new(
                configured
                    .iter()
                    .filter_map(|value| value.parse().ok())
                    .collect(),
            ),
        }
    }

    pub(crate) fn contains(&self, origin: &HeaderValue) -> bool {
        self.origins.contains(origin)
    }
}

#[derive(Clone)]
pub(crate) struct ApiToken {
    token: Arc<SecretString>,
    principal: Principal,
}

impl ApiToken {
    pub(crate) fn new(token: SecretString) -> Self {
        Self {
            token: Arc::new(token),
            principal: Principal::owner(),
        }
    }

    pub(crate) fn adapter(token: SecretString) -> Self {
        Self {
            token: Arc::new(token),
            principal: Principal::adapter(),
        }
    }
}

pub(crate) async fn fixed_bearer_auth(
    axum::extract::Extension(token): axum::extract::Extension<ApiToken>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let expected = token.token.expose_secret().as_bytes();
    let supplied = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::as_bytes);
    if !supplied.is_some_and(|value| constant_time_eq(value, expected)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    req.extensions_mut().insert(token.principal);
    next.run(req).await
}

pub(crate) async fn bearer_auth(
    axum::extract::Extension(token): axum::extract::Extension<ApiToken>,
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let supplied = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    if let Some(supplied) = supplied
        && supplied.starts_with("bobe_v2_")
    {
        match state
            .body
            .trust
            .authenticate_bearer(supplied, state.infra.identity.runtime_authority_epoch)
            .await
        {
            Ok(Some(authenticated)) => {
                req.extensions_mut().insert(Principal::endpoint(
                    authenticated.endpoint_id,
                    authenticated.credential_id,
                    authenticated.issued_authority_epoch,
                    authenticated.scopes,
                ));
                return next.run(req).await;
            }
            Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
            Err(error) => {
                tracing::error!(%error, "security.endpoint_auth_failed");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    let expected = token.token.expose_secret().as_bytes();
    if expected.is_empty() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let supplied = supplied.map(str::as_bytes);
    if !supplied.is_some_and(|value| constant_time_eq(value, expected)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if token.principal.kind == crate::api::principal::PrincipalKind::Owner
        && !owner_token_allowed(&state.config().server, connected_peer(&req))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    req.extensions_mut().insert(token.principal);
    next.run(req).await
}

fn connected_peer(req: &Request<Body>) -> Option<std::net::SocketAddr> {
    req.extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|axum::extract::ConnectInfo(peer)| *peer)
}

fn owner_token_allowed(
    server: &crate::config::ServerConfig,
    peer: Option<std::net::SocketAddr>,
) -> bool {
    if !server.is_networked() {
        return true;
    }
    let listens_directly = server
        .host
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| !address.is_loopback());
    listens_directly && peer.is_some_and(|peer| peer.ip().to_canonical().is_loopback())
}

pub(crate) async fn scope_enforcement(
    axum::extract::Extension(principal): axum::extract::Extension<Principal>,
    request: Request<Body>,
    next: Next,
) -> Response {
    match required_scope(request.method(), request.uri().path()) {
        RouteScope::Public => next.run(request).await,
        RouteScope::Required(scope) if principal.scopes.contains(scope) => next.run(request).await,
        RouteScope::Required(scope) => {
            tracing::warn!(
                principal_kind = ?principal.kind,
                endpoint_id = principal.endpoint_id.as_deref(),
                credential_id = principal.credential_id.as_deref(),
                issued_authority_epoch = principal.issued_authority_epoch,
                required_scope = ?scope,
                method = %request.method(),
                path = request.uri().path(),
                "security.scope_denied"
            );
            StatusCode::FORBIDDEN.into_response()
        }
        RouteScope::Unknown => {
            tracing::error!(
                method = %request.method(),
                path = request.uri().path(),
                "security.route_scope_missing"
            );
            StatusCode::FORBIDDEN.into_response()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteScope {
    Public,
    Required(Scope),
    Unknown,
}

fn required_scope(method: &Method, path: &str) -> RouteScope {
    use Scope::{
        BodyAdmin, CaptureSubmit, ConversationRead, ConversationWrite, EventsSubscribe, GoalsRead,
        GoalsWrite, MemoryRead, MemoryWrite, ProviderAdmin, SettingsRead, SettingsWrite,
        VoiceSession,
    };

    if matches!(path, "/health" | "/readyz" | "/v1/hello")
        || (*method == Method::DELETE && path == "/endpoints/self")
    {
        return RouteScope::Public;
    }
    if path == "/events" {
        return RouteScope::Required(EventsSubscribe);
    }
    if matches!(path, "/voice/stream" | "/voice/preview") {
        return RouteScope::Required(VoiceSession);
    }
    if path == "/conversation/current"
        || (path.starts_with("/conversation/") && path.ends_with("/presented"))
    {
        return RouteScope::Required(ConversationRead);
    }
    if path == "/runs" {
        return RouteScope::Required(ConversationRead);
    }
    if path == "/owner-input" || path.starts_with("/owner-input/") {
        return RouteScope::Required(if *method == Method::GET {
            ConversationRead
        } else {
            ConversationWrite
        });
    }
    if path.starts_with("/runs/") && path.contains("/inputs/") {
        return RouteScope::Required(ProviderAdmin);
    }
    if path == "/message" || path.starts_with("/runs/") {
        return RouteScope::Required(ConversationWrite);
    }
    if path == "/privacy/data" {
        return RouteScope::Required(SettingsWrite);
    }
    if *method == Method::GET
        && ["/agent/jobs", "/agent/schedules"]
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
    {
        return RouteScope::Required(GoalsRead);
    }
    if path == "/memory"
        || path.starts_with("/souls")
        || path.starts_with("/user-profiles")
        || path == "/skills"
        || path.starts_with("/skills/")
        || path == "/workspace"
        || path.starts_with("/workspace/")
    {
        return RouteScope::Required(if *method == Method::GET {
            MemoryRead
        } else {
            MemoryWrite
        });
    }
    if path == "/goals" || path.starts_with("/goals/") {
        return RouteScope::Required(if *method == Method::GET {
            GoalsRead
        } else {
            GoalsWrite
        });
    }
    if path == "/settings" {
        return RouteScope::Required(if *method == Method::GET {
            SettingsRead
        } else {
            SettingsWrite
        });
    }
    if path == "/status" || path == "/initialization" {
        return RouteScope::Required(if *method == Method::GET {
            SettingsRead
        } else {
            SettingsWrite
        });
    }
    if path == "/config" || path.starts_with("/config/") {
        return RouteScope::Required(SettingsWrite);
    }
    if path.starts_with("/body/requests/") {
        return RouteScope::Required(CaptureSubmit);
    }
    if path == "/endpoints" || path.starts_with("/endpoints/") || path == "/pairing/invitations" {
        return RouteScope::Required(BodyAdmin);
    }
    if path == "/diagnostics/provider" {
        return RouteScope::Required(ProviderAdmin);
    }
    if path == "/providers"
        || path == "/agent/jobs"
        || path.starts_with("/agent/jobs/")
        || path == "/agent/schedules"
        || path.starts_with("/agent/schedules/")
        || path == "/agent/context"
        || path == "/agent/tools"
        || path == "/agent/context/compact"
        || path.starts_with("/providers/")
        || path == "/provider-routes"
        || path.starts_with("/auth/")
        || path == "/models"
        || path.starts_with("/local-runtime/")
        || path.starts_with("/tools/mcp/")
        || path.starts_with("/voice/install/")
    {
        return RouteScope::Required(ProviderAdmin);
    }
    RouteScope::Unknown
}

fn request_host(req: &Request<Body>) -> String {
    req.headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            req.uri()
                .authority()
                .map(axum::http::uri::Authority::as_str)
        })
        .unwrap_or("")
        .to_lowercase()
}

pub(crate) async fn host_validation(
    axum::extract::Extension(allowed): axum::extract::Extension<AllowedHosts>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let host = request_host(&req);

    if !allowed.hosts.contains(&host) {
        tracing::warn!(host = %host, "security.dns_rebinding_blocked");
        return StatusCode::FORBIDDEN.into_response();
    }

    next.run(req).await
}

pub(crate) async fn request_logging(req: Request<Body>, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string()[..12].to_string();
    let method = req.method().clone();
    let uri = req.uri().path().to_owned();
    let start = std::time::Instant::now();

    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %uri,
        "http.request_start"
    );

    let response = next.run(req).await;

    let duration_ms = start.elapsed().as_secs_f64() * MILLIS_PER_SECOND;
    let status = response.status().as_u16();

    if status >= 500 {
        tracing::error!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status,
            duration_ms = format!("{duration_ms:.1}"),
            "http.request_error"
        );
    } else if status >= 400 {
        tracing::warn!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status,
            duration_ms = format!("{duration_ms:.1}"),
            "http.request_client_error"
        );
    } else {
        tracing::info!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status,
            duration_ms = format!("{duration_ms:.1}"),
            "http.request_complete"
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_api_route_has_an_explicit_scope_policy() {
        use RouteScope::{Public, Required};

        let cases = [
            (Method::POST, "/v1/hello", Public),
            (Method::GET, "/health", Public),
            (Method::GET, "/readyz", Public),
            (Method::GET, "/status", Required(Scope::SettingsRead)),
            (
                Method::POST,
                "/diagnostics/provider",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::DELETE,
                "/privacy/data",
                Required(Scope::SettingsWrite),
            ),
            (Method::PATCH, "/config", Required(Scope::SettingsWrite)),
            (
                Method::POST,
                "/initialization",
                Required(Scope::SettingsWrite),
            ),
            (
                Method::GET,
                "/conversation/current",
                Required(Scope::ConversationRead),
            ),
            (Method::POST, "/message", Required(Scope::ConversationWrite)),
            (
                Method::POST,
                "/runs/id/cancel",
                Required(Scope::ConversationWrite),
            ),
            (
                Method::PUT,
                "/runs/id/inputs/input",
                Required(Scope::ProviderAdmin),
            ),
            (Method::GET, "/providers", Required(Scope::ProviderAdmin)),
            (
                Method::PUT,
                "/providers/id/credential",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::PUT,
                "/provider-routes",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::PATCH,
                "/provider-routes",
                Required(Scope::ProviderAdmin),
            ),
            (Method::GET, "/endpoints", Required(Scope::BodyAdmin)),
            (Method::DELETE, "/endpoints/id", Required(Scope::BodyAdmin)),
            (Method::DELETE, "/endpoints/self", Public),
            (Method::PUT, "/endpoints/self", Required(Scope::BodyAdmin)),
            (
                Method::POST,
                "/pairing/invitations",
                Required(Scope::BodyAdmin),
            ),
            (
                Method::POST,
                "/body/requests/breq_1",
                Required(Scope::CaptureSubmit),
            ),
            (Method::GET, "/goals", Required(Scope::GoalsRead)),
            (Method::PATCH, "/goals/id", Required(Scope::GoalsWrite)),
            (Method::GET, "/memory", Required(Scope::MemoryRead)),
            (Method::PUT, "/memory", Required(Scope::MemoryWrite)),
            (Method::GET, "/souls", Required(Scope::MemoryRead)),
            (Method::POST, "/user-profiles", Required(Scope::MemoryWrite)),
            (Method::GET, "/skills", Required(Scope::MemoryRead)),
            (Method::POST, "/skills/reload", Required(Scope::MemoryWrite)),
            (Method::GET, "/workspace", Required(Scope::MemoryRead)),
            (Method::PUT, "/workspace/file", Required(Scope::MemoryWrite)),
            (Method::GET, "/settings", Required(Scope::SettingsRead)),
            (Method::PATCH, "/settings", Required(Scope::SettingsWrite)),
            (Method::GET, "/auth/status", Required(Scope::ProviderAdmin)),
            (Method::GET, "/models", Required(Scope::ProviderAdmin)),
            (Method::GET, "/agent/tools", Required(Scope::ProviderAdmin)),
            (Method::PUT, "/agent/tools", Required(Scope::ProviderAdmin)),
            (Method::GET, "/agent/jobs", Required(Scope::GoalsRead)),
            (Method::GET, "/agent/jobs/id", Required(Scope::GoalsRead)),
            (Method::POST, "/agent/jobs", Required(Scope::ProviderAdmin)),
            (
                Method::POST,
                "/agent/jobs/id/cancel",
                Required(Scope::ProviderAdmin),
            ),
            (Method::GET, "/agent/schedules", Required(Scope::GoalsRead)),
            (
                Method::PUT,
                "/agent/schedules/id/time",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::GET,
                "/agent/context",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::POST,
                "/agent/context/compact",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::POST,
                "/local-runtime/install",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::PUT,
                "/tools/mcp/config",
                Required(Scope::ProviderAdmin),
            ),
            (
                Method::GET,
                "/voice/install/status",
                Required(Scope::ProviderAdmin),
            ),
            (Method::GET, "/events", Required(Scope::EventsSubscribe)),
            (Method::GET, "/voice/stream", Required(Scope::VoiceSession)),
            (Method::GET, "/voice/preview", Required(Scope::VoiceSession)),
        ];

        for (method, path, expected) in cases {
            assert_eq!(required_scope(&method, path), expected, "{method} {path}");
        }
        assert_eq!(
            required_scope(&Method::GET, "/unclassified"),
            RouteScope::Unknown
        );
    }

    async fn enrolled_principal(profile: crate::api::scope::ScopeProfile) -> Principal {
        let trust = crate::db::device_trust_repo::DeviceTrustRepo::new(
            crate::db::test_helpers::in_memory_pool().await,
        );
        let issued = trust
            .enroll_endpoint_with_bearer(
                &crate::db::endpoint_repo::EndpointUpsert {
                    id: "6f2a3f8e-0b7d-4f5e-9a51-3c1d2e4f5a6b".into(),
                    kind: profile.endpoint_kind().into(),
                    label: profile.default_label().into(),
                    room: None,
                    certificate_sha256: None,
                    protocol_min: 1,
                    protocol_max: 1,
                    capabilities_json: "{}".into(),
                },
                profile.scopes(),
                1,
                "pairing",
                None,
            )
            .await
            .unwrap();
        let authenticated = trust
            .authenticate_bearer(&issued.token, 1)
            .await
            .unwrap()
            .unwrap();
        Principal::endpoint(
            authenticated.endpoint_id,
            authenticated.credential_id,
            authenticated.issued_authority_epoch,
            authenticated.scopes,
        )
    }

    async fn status_for(principal: &Principal, method: &Method, path: &str) -> StatusCode {
        use tower::ServiceExt;

        axum::Router::new()
            .fallback(|| async { StatusCode::OK })
            .layer(axum::middleware::from_fn(scope_enforcement))
            .layer(axum::Extension(principal.clone()))
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[test]
    fn the_owner_token_works_from_this_computer_only_when_bound_directly() {
        let local: std::net::SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let mapped: std::net::SocketAddr = "[::ffff:127.0.0.1]:50000".parse().unwrap();
        let remote: std::net::SocketAddr = "192.168.1.20:50000".parse().unwrap();

        let loopback_only = crate::config::ServerConfig::default();
        assert!(owner_token_allowed(&loopback_only, Some(local)));
        assert!(owner_token_allowed(&loopback_only, None));

        let direct = crate::config::ServerConfig {
            host: "0.0.0.0".into(),
            allowed_hosts: vec!["192.168.1.20".into()],
            ..crate::config::ServerConfig::default()
        };
        assert!(owner_token_allowed(&direct, Some(local)));
        assert!(owner_token_allowed(&direct, Some(mapped)));
        assert!(!owner_token_allowed(&direct, Some(remote)));
        assert!(!owner_token_allowed(&direct, None));

        let behind_proxy = crate::config::ServerConfig {
            allowed_hosts: vec!["bobe.example.test".into()],
            ..crate::config::ServerConfig::default()
        };
        assert!(!owner_token_allowed(&behind_proxy, Some(local)));
    }

    #[tokio::test]
    async fn a_phone_credential_reaches_its_body_routes_and_nothing_administrative() {
        use crate::api::scope::ScopeProfile;

        let phone = enrolled_principal(ScopeProfile::Phone).await;
        let mac = enrolled_principal(ScopeProfile::MacApp).await;
        let allowed = [
            (Method::POST, "/v1/hello"),
            (Method::GET, "/health"),
            (Method::GET, "/readyz"),
            (Method::GET, "/events"),
            (Method::GET, "/voice/stream"),
            (Method::GET, "/conversation/current"),
            (Method::POST, "/conversation/turn/presented"),
            (Method::POST, "/message"),
            (Method::GET, "/runs"),
            (Method::POST, "/runs/run/cancel"),
            (Method::GET, "/owner-input"),
            (Method::GET, "/owner-input/input"),
            (Method::POST, "/owner-input/input/presented"),
            (Method::POST, "/owner-input/input/answer"),
            (Method::POST, "/owner-input/input/cancel"),
            (Method::GET, "/goals"),
            (Method::POST, "/goals"),
            (Method::PATCH, "/goals/goal"),
            (Method::DELETE, "/goals/goal"),
            (Method::POST, "/goals/goal/complete"),
            (Method::POST, "/goals/goal/archive"),
            (Method::GET, "/agent/jobs"),
            (Method::GET, "/agent/jobs/job"),
            (Method::GET, "/agent/schedules"),
            (Method::GET, "/agent/schedules/schedule"),
            (Method::GET, "/memory"),
            (Method::PUT, "/memory"),
            (Method::GET, "/souls"),
            (Method::POST, "/souls"),
            (Method::PATCH, "/souls/soul"),
            (Method::DELETE, "/souls/soul"),
            (Method::POST, "/souls/soul/enable"),
            (Method::POST, "/souls/soul/disable"),
            (Method::GET, "/user-profiles"),
            (Method::POST, "/user-profiles"),
            (Method::PATCH, "/user-profiles/profile"),
            (Method::DELETE, "/user-profiles/profile"),
            (Method::POST, "/user-profiles/profile/enable"),
            (Method::POST, "/user-profiles/profile/disable"),
            (Method::GET, "/skills"),
            (Method::POST, "/skills/reload"),
            (Method::GET, "/skills/skill"),
            (Method::GET, "/workspace"),
            (Method::GET, "/workspace/file"),
            (Method::PUT, "/workspace/file"),
            (Method::GET, "/settings"),
            (Method::GET, "/status"),
            (Method::GET, "/initialization"),
            (Method::DELETE, "/endpoints/self"),
        ];
        let refused = [
            (Method::PUT, "/runs/run/inputs/input"),
            (Method::POST, "/agent/jobs"),
            (Method::POST, "/agent/jobs/job/cancel"),
            (Method::POST, "/agent/jobs/job/resume"),
            (Method::POST, "/agent/schedules"),
            (Method::DELETE, "/agent/schedules/schedule"),
            (Method::PUT, "/agent/schedules/schedule/enabled"),
            (Method::PUT, "/agent/schedules/schedule/time"),
            (Method::GET, "/agent/tools"),
            (Method::PUT, "/agent/tools"),
            (Method::GET, "/agent/context"),
            (Method::POST, "/agent/context/compact"),
            (Method::PATCH, "/settings"),
            (Method::PATCH, "/config"),
            (Method::DELETE, "/config/server.host"),
            (Method::POST, "/initialization"),
            (Method::DELETE, "/privacy/data"),
            (Method::GET, "/providers"),
            (Method::PUT, "/providers/profile"),
            (Method::DELETE, "/providers/profile"),
            (Method::PUT, "/providers/profile/activation"),
            (Method::PUT, "/providers/profile/credential"),
            (Method::DELETE, "/providers/profile/credential"),
            (Method::POST, "/providers/profile/probe"),
            (Method::PUT, "/provider-routes"),
            (Method::PATCH, "/provider-routes"),
            (Method::POST, "/diagnostics/provider"),
            (Method::GET, "/auth/profile/status"),
            (Method::POST, "/auth/profile/login/start"),
            (Method::POST, "/auth/profile/login/cancel"),
            (Method::GET, "/auth/profile/login/status"),
            (Method::POST, "/auth/profile/login/submit"),
            (Method::GET, "/auth/profile/login/events"),
            (Method::POST, "/auth/profile/logout"),
            (Method::GET, "/models"),
            (Method::POST, "/local-runtime/install"),
            (Method::POST, "/local-runtime/cancel"),
            (Method::GET, "/local-runtime/status"),
            (Method::GET, "/tools/mcp/config"),
            (Method::PUT, "/tools/mcp/config"),
            (Method::DELETE, "/tools/mcp/config"),
            (Method::POST, "/tools/mcp/config/validate"),
            (Method::GET, "/voice/install/status"),
            (Method::POST, "/voice/install/start"),
            (Method::POST, "/voice/install/start/tts"),
            (Method::POST, "/voice/install/start/stt"),
            (Method::POST, "/voice/install/cancel"),
            (Method::GET, "/endpoints"),
            (Method::PUT, "/endpoints/endpoint"),
            (Method::DELETE, "/endpoints/endpoint"),
            (Method::POST, "/endpoints/endpoint/display"),
            (Method::GET, "/endpoints/endpoint/sensors"),
            (Method::POST, "/pairing/invitations"),
            (Method::POST, "/body/requests/breq_1"),
        ];
        for (method, path) in &allowed {
            assert_eq!(
                status_for(&phone, method, path).await,
                StatusCode::OK,
                "{method} {path}"
            );
        }
        for (method, path) in &refused {
            assert_eq!(
                status_for(&phone, method, path).await,
                StatusCode::FORBIDDEN,
                "{method} {path}"
            );
        }
        for (method, path) in allowed.iter().chain(&refused) {
            assert_eq!(
                status_for(&mac, method, path).await,
                StatusCode::OK,
                "{method} {path}"
            );
        }
    }

    #[test]
    fn host_validation_uses_http2_uri_authority_without_host_header() {
        let request = Request::builder()
            .uri("https://bobe.example.test:8443/health")
            .body(Body::empty())
            .unwrap();
        assert_eq!(request_host(&request), "bobe.example.test:8443");
    }

    #[test]
    fn host_header_takes_precedence_over_uri_authority() {
        let request = Request::builder()
            .uri("https://bobe.example.test/health")
            .header(header::HOST, "proxy.example.test")
            .body(Body::empty())
            .unwrap();
        assert_eq!(request_host(&request), "proxy.example.test");
    }

    #[test]
    fn allowed_origins_accept_exact_configured_origin() {
        let allowed = AllowedOrigins::new(&["https://bobe.example.test".into()]);
        assert!(allowed.contains(&HeaderValue::from_static("https://bobe.example.test")));
        assert!(!allowed.contains(&HeaderValue::from_static("https://evil.example.test")));
    }
}
