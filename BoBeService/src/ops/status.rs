use std::time::Duration;

use reqwest::{StatusCode, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::api::readiness::ReadinessStatus;
use crate::config::Config;
use crate::ops::exit::ExitCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadinessLevel {
    Process,
    Operational,
}

impl ReadinessLevel {
    const fn is_reached_by(self, status: ReadinessStatus) -> bool {
        match self {
            Self::Process => status.is_serving(),
            Self::Operational => matches!(status, ReadinessStatus::Ready),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OpsTarget {
    base_url: Url,
    token: SecretString,
    tls_cert_path: Option<std::path::PathBuf>,
}

impl OpsTarget {
    pub(crate) fn resolve(
        config: &Config,
        url: Option<&str>,
        token: Option<&str>,
    ) -> Result<Self, String> {
        let default_host = if config.server.host == "0.0.0.0" || config.server.host == "::" {
            "127.0.0.1"
        } else {
            config.server.host.as_str()
        };
        let automatic_paths = if config.data_dir.trim().is_empty() {
            None
        } else {
            Some(
                crate::util::paths::DataPaths::resolve(&config.data_dir, &config.database.url)
                    .map_err(|error| error.to_string())?,
            )
        };
        let manual_tls =
            config.server.tls_cert_path.is_some() && config.server.tls_key_path.is_some();
        let automatic_tls = automatic_paths.as_ref().is_some_and(|paths| {
            paths.automatic_tls_certificate().is_file()
                && paths.automatic_tls_private_key().is_file()
                && paths.automatic_tls_metadata().is_file()
        });
        let scheme = if manual_tls || automatic_tls {
            "https"
        } else {
            "http"
        };
        let configured_url = format!("{scheme}://{default_host}:{}", config.server.port);
        let raw_url = url.map_or_else(|| configured_url.clone(), str::to_owned);
        let mut base_url =
            Url::parse(&raw_url).map_err(|error| format!("invalid daemon URL: {error}"))?;
        if base_url.query().is_some()
            || base_url.fragment().is_some()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
        {
            return Err("daemon URL must not include credentials, query, or fragment".into());
        }
        if !base_url.path().ends_with('/') {
            base_url
                .path_segments_mut()
                .map_err(|()| "daemon URL must support path segments".to_owned())?
                .push("");
        }
        let host = base_url
            .host_str()
            .ok_or_else(|| "daemon URL must include a host".to_string())?;
        let is_loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        let explicit_url_uses_configured_target = url.is_some_and(|value| {
            value.trim_end_matches('/') == configured_url.trim_end_matches('/')
                || (is_loopback && base_url.port_or_known_default() == Some(config.server.port))
        });
        let token = SecretString::from(match token {
            Some(token) => token.to_owned(),
            None if url.is_none() || explicit_url_uses_configured_target => {
                config.server.api_token.expose_secret().to_owned()
            }
            None => String::new(),
        });
        if !is_loopback && base_url.scheme() != "https" {
            return Err("non-loopback daemon URL must use HTTPS".into());
        }
        if !is_loopback && token.expose_secret().is_empty() {
            return Err("non-loopback daemon requires a bearer token".into());
        }
        let uses_configured_target = url.is_none() || explicit_url_uses_configured_target;
        let tls_cert_path = if uses_configured_target && base_url.scheme() == "https" {
            if manual_tls {
                config.server.tls_cert_path.as_ref().map(Into::into)
            } else if automatic_tls {
                automatic_paths.map(|paths| paths.automatic_tls_certificate())
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            base_url,
            token,
            tls_cert_path,
        })
    }

    pub(crate) fn display(&self) -> String {
        self.base_url.as_str().trim_end_matches('/').to_owned()
    }

    pub(crate) async fn voice_socket(
        &self,
    ) -> Result<tokio_tungstenite::WebSocketStream<reqwest::Upgraded>, crate::error::AppError> {
        use reqwest::header;
        use tokio_tungstenite::tungstenite::{
            handshake::{client::generate_key, derive_accept_key},
            protocol::Role,
        };
        let key = generate_key();
        let protocol = crate::constants::voice_wire::SUBPROTOCOL_V2;
        let response = self
            .request_builder_with_timeout(reqwest::Method::GET, "/voice/preview", None)?
            .version(reqwest::Version::HTTP_11)
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header(header::SEC_WEBSOCKET_KEY, &key)
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .header(header::SEC_WEBSOCKET_PROTOCOL, protocol)
            .send()
            .await?;
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(crate::error::AppError::Validation(format!(
                "Voice connection rejected (HTTP {})",
                response.status()
            )));
        }
        let expected = derive_accept_key(key.as_bytes());
        if response
            .headers()
            .get(header::SEC_WEBSOCKET_ACCEPT)
            .and_then(|v| v.to_str().ok())
            != Some(expected.as_str())
            || response
                .headers()
                .get(header::SEC_WEBSOCKET_PROTOCOL)
                .and_then(|v| v.to_str().ok())
                != Some(protocol)
        {
            return Err(crate::error::AppError::Validation(
                "Invalid voice WebSocket handshake".into(),
            ));
        }
        Ok(tokio_tungstenite::WebSocketStream::from_raw_socket(
            response.upgrade().await?,
            Role::Client,
            None,
        )
        .await)
    }

    pub(crate) async fn patch_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        self.request_builder(reqwest::Method::PATCH, path)?
            .json(body)
            .send()
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn put_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        self.request_builder(reqwest::Method::PUT, path)?
            .json(body)
            .send()
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        self.request_builder(reqwest::Method::POST, path)?
            .json(body)
            .send()
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn post(
        &self,
        path: &str,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        self.request(reqwest::Method::POST, path).await
    }

    pub(crate) async fn delete(
        &self,
        path: &str,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        self.request(reqwest::Method::DELETE, path).await
    }

    pub(crate) async fn get(
        &self,
        path: &str,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        self.request(reqwest::Method::GET, path).await
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        Ok(self.request_builder(method, path)?.send().await?)
    }

    fn request_builder(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, crate::error::AppError> {
        self.request_builder_with_timeout(method, path, Some(read_timeout_for(path)))
    }

    pub(crate) async fn diagnose_provider(
        &self,
    ) -> Result<crate::api::handlers::health::ProviderDiagnosticResponse, crate::error::AppError>
    {
        let response = self
            .request_builder_with_timeout(
                reqwest::Method::POST,
                "/diagnostics/provider",
                Some(Duration::from_secs(95)),
            )?
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(crate::error::AppError::Internal(format!(
                "provider diagnostic returned HTTP {status}: {}",
                response.text().await?,
            )));
        }
        Ok(response.json().await?)
    }

    pub(crate) async fn stream_get(
        &self,
        path: &str,
    ) -> Result<reqwest::Response, crate::error::AppError> {
        self.request_builder_with_timeout(reqwest::Method::GET, path, None)?
            .send()
            .await
            .map_err(Into::into)
    }

    fn request_builder_with_timeout(
        &self,
        method: reqwest::Method,
        path: &str,
        read_timeout: Option<Duration>,
    ) -> Result<reqwest::RequestBuilder, crate::error::AppError> {
        let url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| {
                crate::error::AppError::Internal(format!("invalid request URL: {error}"))
            })?;
        let mut client = reqwest::Client::builder().connect_timeout(Duration::from_secs(3));
        if let Some(read_timeout) = read_timeout {
            client = client.read_timeout(read_timeout);
        }
        if let Some(path) = &self.tls_cert_path {
            let pem = std::fs::read(path).map_err(|error| {
                crate::error::AppError::Config(format!(
                    "could not read daemon TLS certificate '{}': {error}",
                    path.display()
                ))
            })?;
            let certificate = reqwest::Certificate::from_pem(&pem).map_err(|error| {
                crate::error::AppError::Config(format!(
                    "invalid daemon TLS certificate '{}': {error}",
                    path.display()
                ))
            })?;
            client = client.add_root_certificate(certificate);
        }
        let client = client.build()?;
        let mut request = client.request(method, url);
        if !self.token.expose_secret().is_empty() {
            request = request.bearer_auth(self.token.expose_secret());
        }
        Ok(request)
    }
}

const CONFIGURATION_READ_TIMEOUT: Duration =
    Duration::from_secs(crate::services::mcp_config_service::REQUEST_TIMEOUT.as_secs() + 10);

fn read_timeout_for(path: &str) -> Duration {
    if path == "/agent/context/compact" {
        Duration::from_secs(320)
    } else if path == "/config"
        || path.starts_with("/config/")
        || path == "/settings"
        || path == "/tools/mcp/config"
    {
        CONFIGURATION_READ_TIMEOUT
    } else if path == "/initialization" {
        Duration::from_secs(95)
    } else if path == "/agent/context"
        || path.starts_with("/agent/jobs")
        || path == "/agent/tools"
        || path.starts_with("/auth/")
        || path.starts_with("/models")
    {
        Duration::from_secs(35)
    } else {
        Duration::from_secs(5)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ReadinessView {
    pub(crate) status: ReadinessStatus,
    pub(crate) runtime_authority_id: uuid::Uuid,
    pub(crate) runtime_authority_epoch: i64,
    pub(crate) runtime_incarnation_id: uuid::Uuid,
    pub(crate) daemon_version: String,
    pub(crate) schema_version: i64,
    pub(crate) initialization_state: crate::db::runtime_initialization_repo::InitializationState,
    pub(crate) started_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub(crate) blocking_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RuntimeView {
    pub(crate) indicator: String,
    pub(crate) capturing: bool,
    pub(crate) accepting_user_messages: bool,
    pub(crate) version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct HealthView {
    pub(crate) status: String,
    pub(crate) version: Option<String>,
    pub(crate) services: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct StatusReport {
    pub(crate) target: String,
    pub(crate) reachable: bool,
    pub(crate) readiness: Option<ReadinessView>,
    pub(crate) runtime: Option<RuntimeView>,
    pub(crate) liveness: Option<HealthView>,
    pub(crate) error: Option<String>,
}

pub(crate) async fn collect(target: &OpsTarget) -> (ExitCode, StatusReport) {
    let readiness_response = match target.get("/readyz").await {
        Ok(response) => response,
        Err(error) => {
            return (
                ExitCode::NotReady,
                StatusReport {
                    target: target.display(),
                    reachable: false,
                    readiness: None,
                    runtime: None,
                    liveness: None,
                    error: Some(error.to_string()),
                },
            );
        }
    };
    if readiness_response.status() == StatusCode::UNAUTHORIZED {
        return (
            ExitCode::Invalid,
            StatusReport {
                target: target.display(),
                reachable: true,
                readiness: None,
                runtime: None,
                liveness: None,
                error: Some("daemon rejected the bearer token".into()),
            },
        );
    }
    let readiness = readiness_response.json::<ReadinessView>().await.ok();
    let runtime = match target.get("/status").await {
        Ok(response) if response.status().is_success() => response.json().await.ok(),
        _ => None,
    };
    let liveness = match target.get("/health").await {
        Ok(response) if response.status().is_success() => response.json().await.ok(),
        _ => None,
    };
    let ready = readiness
        .as_ref()
        .is_some_and(|view| ReadinessLevel::Operational.is_reached_by(view.status));
    (
        if ready {
            ExitCode::Success
        } else {
            ExitCode::NotReady
        },
        StatusReport {
            target: target.display(),
            reachable: true,
            readiness,
            runtime,
            liveness,
            error: None,
        },
    )
}

pub(crate) async fn wait_ready_for(
    target: &OpsTarget,
    timeout: Duration,
    interval: Duration,
    level: ReadinessLevel,
    authority_id: Option<uuid::Uuid>,
    authority_epoch: Option<i64>,
    daemon_version: Option<&str>,
) -> (ExitCode, StatusReport) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let (exit, report) = collect(target).await;
        if exit == ExitCode::Invalid {
            return (exit, report);
        }
        let reached = report.readiness.as_ref().is_some_and(|readiness| {
            level.is_reached_by(readiness.status)
                && authority_id.is_none_or(|expected| readiness.runtime_authority_id == expected)
                && authority_epoch
                    .is_none_or(|expected| readiness.runtime_authority_epoch == expected)
                && daemon_version.is_none_or(|expected| readiness.daemon_version == expected)
        });
        if reached {
            return (ExitCode::Success, report);
        }
        if tokio::time::Instant::now() >= deadline {
            return (ExitCode::NotReady, report);
        }
        tokio::time::sleep(interval).await;
    }
}

pub(crate) async fn wait_ready(
    target: &OpsTarget,
    timeout: Duration,
    interval: Duration,
) -> (ExitCode, StatusReport) {
    wait_ready_for(
        target,
        timeout,
        interval,
        ReadinessLevel::Operational,
        None,
        None,
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn owner_requests_preserve_remote_prefixes_with_or_without_a_trailing_slash() {
        crate::util::tls::install_crypto_provider().unwrap();
        for base in [
            "https://authority.example/bobe",
            "https://authority.example/bobe/",
            "https://authority.example/bo%20be",
        ] {
            let target =
                OpsTarget::resolve(&Config::default(), Some(base), Some("fixture")).unwrap();
            for path in ["/agent/tools", "/agent/jobs", "/auth/profile/login"] {
                let request = target
                    .request_builder(reqwest::Method::GET, path)
                    .unwrap()
                    .build()
                    .unwrap();
                assert_eq!(
                    request.url().as_str(),
                    format!("{}{path}", base.trim_end_matches('/'))
                );
            }
        }
    }

    use super::*;

    #[test]
    fn configuration_requests_outlast_the_daemon_wait_for_an_mcp_reload() {
        let daemon_limit = crate::services::mcp_config_service::REQUEST_TIMEOUT;
        for path in [
            "/config",
            "/config/mcp.enabled",
            "/settings",
            "/tools/mcp/config",
        ] {
            assert!(read_timeout_for(path) > daemon_limit, "{path}");
            assert!(
                read_timeout_for(path) <= daemon_limit + Duration::from_secs(30),
                "{path}"
            );
        }
        for (path, seconds) in [
            ("/configuration", 5),
            ("/status", 5),
            ("/initialization", 95),
            ("/agent/tools", 35),
            ("/agent/context/compact", 320),
        ] {
            assert_eq!(
                read_timeout_for(path),
                Duration::from_secs(seconds),
                "{path}"
            );
        }
    }

    #[test]
    fn process_readiness_includes_setup_but_operational_readiness_does_not() {
        assert!(ReadinessLevel::Process.is_reached_by(ReadinessStatus::Ready));
        assert!(ReadinessLevel::Process.is_reached_by(ReadinessStatus::Initializing));
        assert!(!ReadinessLevel::Process.is_reached_by(ReadinessStatus::NotReady));
        assert!(ReadinessLevel::Operational.is_reached_by(ReadinessStatus::Ready));
        assert!(!ReadinessLevel::Operational.is_reached_by(ReadinessStatus::Initializing));
        assert!(!ReadinessLevel::Operational.is_reached_by(ReadinessStatus::NotReady));
    }

    #[tokio::test]
    async fn a_runtime_still_setting_up_is_running_but_not_ready() {
        use crate::db::runtime_initialization_repo::InitializationState;

        crate::util::tls::install_crypto_provider().unwrap();
        let identity = crate::db::runtime_authority_repo::RuntimeIdentity {
            runtime_authority_id: uuid::Uuid::new_v4(),
            runtime_authority_epoch: 2,
            runtime_incarnation_id: uuid::Uuid::new_v4(),
            daemon_version: "1.2.3",
            schema_version: 1,
            started_at: chrono::Utc::now(),
        };
        let body = serde_json::to_value(crate::api::handlers::health::ReadinessResponse::new(
            crate::api::readiness::Readiness::evaluate(true, InitializationState::ProviderRequired),
            &identity,
        ))
        .unwrap();
        let app = axum::Router::new().route(
            "/readyz",
            axum::routing::get(async move || axum::Json(body)),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut config = Config::default();
        config.server.host = "127.0.0.1".into();
        config.server.port = address.port();
        let target = OpsTarget::resolve(&config, None, None).unwrap();

        let (exit, report) = collect(&target).await;
        assert_eq!(exit, ExitCode::NotReady);
        let readiness = report.readiness.unwrap();
        assert_eq!(readiness.status, ReadinessStatus::Initializing);
        assert_eq!(readiness.blocking_reasons, ["initialization_incomplete"]);
        assert_eq!(
            readiness.initialization_state,
            InitializationState::ProviderRequired
        );
        assert_eq!(
            readiness.runtime_incarnation_id,
            identity.runtime_incarnation_id
        );
        assert_eq!(readiness.started_at, identity.started_at);

        let (exit, _) = wait_ready_for(
            &target,
            Duration::from_secs(2),
            Duration::from_millis(10),
            ReadinessLevel::Process,
            Some(identity.runtime_authority_id),
            Some(identity.runtime_authority_epoch),
            Some(identity.daemon_version),
        )
        .await;
        assert_eq!(exit, ExitCode::Success);

        let (exit, _) = wait_ready_for(
            &target,
            Duration::from_millis(100),
            Duration::from_millis(10),
            ReadinessLevel::Process,
            Some(identity.runtime_authority_id),
            Some(identity.runtime_authority_epoch + 1),
            None,
        )
        .await;
        assert_eq!(exit, ExitCode::NotReady);

        let (exit, _) = wait_ready(
            &target,
            Duration::from_millis(100),
            Duration::from_millis(10),
        )
        .await;
        assert_eq!(exit, ExitCode::NotReady);
        server.abort();
    }

    #[test]
    fn remote_target_requires_https_and_token() {
        let config = Config::default();
        assert!(OpsTarget::resolve(&config, Some("http://example.com:8766"), Some("x")).is_err());
        assert!(OpsTarget::resolve(&config, Some("https://example.com"), None).is_err());
        assert!(OpsTarget::resolve(&config, Some("https://example.com"), Some("x")).is_ok());
    }

    #[test]
    fn configured_loopback_tls_selects_https_and_pins_certificate() {
        let mut config = Config::default();
        config.server.tls_cert_path = Some("/tmp/bobe-cert.pem".into());
        config.server.tls_key_path = Some("/tmp/bobe-key.pem".into());
        let target = OpsTarget::resolve(&config, None, None).unwrap();
        assert_eq!(target.display(), "https://127.0.0.1:8766");
        assert_eq!(
            target.tls_cert_path.as_deref(),
            Some(std::path::Path::new("/tmp/bobe-cert.pem"))
        );
    }

    #[test]
    fn configured_automatic_tls_selects_https_and_pins_certificate() {
        let root = std::env::temp_dir().join(format!("bobe-ops-tls-{}", uuid::Uuid::new_v4()));
        let tls = root.join("tls");
        std::fs::create_dir_all(&tls).unwrap();
        for name in [
            crate::util::paths::AUTOMATIC_TLS_CERTIFICATE_FILE,
            crate::util::paths::AUTOMATIC_TLS_PRIVATE_KEY_FILE,
            crate::util::paths::AUTOMATIC_TLS_METADATA_FILE,
        ] {
            std::fs::write(tls.join(name), name).unwrap();
        }
        let mut config = Config {
            data_dir: root.to_string_lossy().into_owned(),
            ..Config::default()
        };
        config.server.host = "0.0.0.0".into();
        config.server.allowed_hosts = vec!["127.0.0.1".into()];
        let target = OpsTarget::resolve(&config, None, None).unwrap();
        assert_eq!(target.display(), "https://127.0.0.1:8766");
        assert_eq!(
            target.tls_cert_path.as_deref(),
            Some(
                tls.join(crate::util::paths::AUTOMATIC_TLS_CERTIFICATE_FILE)
                    .as_path()
            )
        );
        let explicit = OpsTarget::resolve(
            &config,
            Some("https://example.com"),
            Some("endpoint-credential"),
        )
        .unwrap();
        assert!(explicit.tls_cert_path.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_remote_https_uses_system_trust_not_local_certificate() {
        let mut config = Config::default();
        config.server.tls_cert_path = Some("/tmp/bobe-cert.pem".into());
        config.server.tls_key_path = Some("/tmp/bobe-key.pem".into());
        let target = OpsTarget::resolve(&config, Some("https://example.com"), Some("x")).unwrap();
        assert!(target.tls_cert_path.is_none());
    }

    #[test]
    fn loopback_target_allows_current_credentialless_mode() {
        assert!(OpsTarget::resolve(&Config::default(), None, None).is_ok());
    }
}
