#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::{Command as StdCommand, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::routing::get;
use axum::{Router, middleware as axum_middleware};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, UnixTime};
use rustls::server::WebPkiClientVerifier;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, DigitallySignedStruct, DistinguishedName, Error, RootCertStore,
    SignatureScheme,
};
use secrecy::ExposeSecret;
use sha2::Digest;
#[cfg(target_os = "macos")]
use tokio::process::{Child, Command as TokioCommand};
#[cfg(target_os = "macos")]
use tokio_util::sync::{CancellationToken, DropGuard};
#[cfg(target_os = "macos")]
use tokio_util::task::AbortOnDropHandle;
use tracing::{info, warn};

use super::gateway::BodyGateway;
use super::handlers::{adapter_stream, body_stream};
use super::identity::BodyIdentityIndex;
use super::protocol::BODY_MDNS_AUTH;
use crate::api::middleware::{ApiToken, fixed_bearer_auth};
use crate::config::BodyConfig;

struct RegistryClientCertVerifier {
    inner: Arc<dyn ClientCertVerifier>,
    identities: Arc<BodyIdentityIndex>,
}

impl fmt::Debug for RegistryClientCertVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryClientCertVerifier")
            .finish_non_exhaustive()
    }
}

impl ClientCertVerifier for RegistryClientCertVerifier {
    fn offer_client_auth(&self) -> bool {
        self.inner.offer_client_auth()
    }

    fn client_auth_mandatory(&self) -> bool {
        self.inner.client_auth_mandatory()
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.inner.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        let verified = self
            .inner
            .verify_client_cert(end_entity, intermediates, now)?;
        let digest: [u8; 32] = sha2::Sha256::digest(end_entity.as_ref()).into();
        if self.identities.by_certificate(&digest).is_none() {
            return Err(CertificateError::ApplicationVerificationFailure.into());
        }
        Ok(verified)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, Error> {
        self.inner
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, Error> {
        self.inner
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }
}

fn load_certificates(path: &Path) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    CertificateDer::pem_file_iter(path)
        .with_context(|| format!("open certificate {}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parse certificate {}", path.display()))
}

fn load_private_key(path: &Path) -> anyhow::Result<PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_file(path)
        .with_context(|| format!("parse private key {}", path.display()))
}

fn tls_config(
    config: &BodyConfig,
    identities: Arc<BodyIdentityIndex>,
) -> anyhow::Result<axum_server::tls_rustls::RustlsConfig> {
    let cert_path = config
        .tls_cert_path
        .as_deref()
        .context("BodyLink requires tls_cert_path")?;
    let key_path = config
        .tls_key_path
        .as_deref()
        .context("BodyLink requires tls_key_path")?;
    let client_ca_path = config
        .client_ca_path
        .as_deref()
        .context("BodyLink requires client_ca_path")?;

    let server_chain = load_certificates(Path::new(cert_path))?;
    anyhow::ensure!(!server_chain.is_empty(), "BodyLink server chain is empty");
    let private_key = load_private_key(Path::new(key_path))?;
    let mut client_roots = RootCertStore::empty();
    for certificate in load_certificates(Path::new(client_ca_path))? {
        client_roots
            .add(certificate)
            .context("add BodyLink client CA certificate")?;
    }
    anyhow::ensure!(!client_roots.is_empty(), "BodyLink client CA file is empty");

    let webpki: Arc<dyn ClientCertVerifier> = WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .context("build BodyLink client certificate verifier")?;
    let pinned: Arc<dyn ClientCertVerifier> = Arc::new(RegistryClientCertVerifier {
        inner: webpki,
        identities,
    });
    let mut server_config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(pinned)
        .with_single_cert(server_chain, private_key)
        .context("build BodyLink TLS server configuration")?;
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(axum_server::tls_rustls::RustlsConfig::from_config(
        Arc::new(server_config),
    ))
}

#[cfg(any(not(target_os = "macos"), test))]
fn body_mdns_service(config: &BodyConfig) -> mdns_sd::Result<mdns_sd::ServiceInfo> {
    let hostname = config
        .advertised_hostname
        .trim_end_matches('.')
        .trim_end_matches(".local");
    let fqdn = format!("{hostname}.local.");
    let properties = HashMap::from([
        ("pv".to_string(), "1".to_string()),
        ("path".to_string(), "/body/v1".to_string()),
        ("tls".to_string(), "1".to_string()),
        ("auth".to_string(), BODY_MDNS_AUTH.to_string()),
        ("kid".to_string(), config.trust_key_id.clone()),
        ("scope".to_string(), "local".to_string()),
    ]);
    mdns_sd::ServiceInfo::new(
        "_bobe-body-hub._tcp.local.",
        &format!("BoBe Body Hub on {hostname}"),
        &fqdn,
        "",
        config.port,
        properties,
    )
    .map(mdns_sd::ServiceInfo::enable_addr_auto)
}

enum MdnsRegistration {
    #[cfg(not(target_os = "macos"))]
    Library(mdns_sd::ServiceDaemon),
    #[cfg(target_os = "macos")]
    Native(NativeMdnsRegistration),
}

impl MdnsRegistration {
    #[cfg_attr(
        not(target_os = "macos"),
        allow(
            clippy::unused_async_trait_impl,
            reason = "macOS shutdown awaits native mDNS task completion"
        )
    )]
    async fn shutdown(self) {
        match self {
            #[cfg(not(target_os = "macos"))]
            Self::Library(daemon) => drop(daemon.shutdown()),
            #[cfg(target_os = "macos")]
            Self::Native(registration) => registration.shutdown().await,
        }
    }
}

#[cfg(target_os = "macos")]
fn primary_ipv4() -> anyhow::Result<Ipv4Addr> {
    fn interface_address(interface: &str) -> Option<Ipv4Addr> {
        let output = StdCommand::new("/usr/sbin/ipconfig")
            .args(["getifaddr", interface])
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
            .flatten()
            .filter(|address: &Ipv4Addr| !address.is_loopback() && !address.is_unspecified())
    }

    let route = StdCommand::new("/sbin/route")
        .args(["-n", "get", "default"])
        .output()
        .context("inspect default network route")?;
    if route.status.success() {
        let text = String::from_utf8_lossy(&route.stdout);
        if let Some(interface) = text
            .lines()
            .find_map(|line| line.trim().strip_prefix("interface:").map(str::trim))
            && let Some(address) = interface_address(interface)
        {
            return Ok(address);
        }
    }

    let interfaces = StdCommand::new("/sbin/ifconfig")
        .arg("-l")
        .output()
        .context("enumerate network interfaces")?;
    String::from_utf8_lossy(&interfaces.stdout)
        .split_whitespace()
        .filter(|interface| *interface != "lo0")
        .find_map(interface_address)
        .context("BodyLink has no routable IPv4 address")
}

#[cfg(target_os = "macos")]
fn native_mdns_command(config: &BodyConfig, address: Ipv4Addr) -> TokioCommand {
    let mut command = TokioCommand::new("/usr/bin/dns-sd");
    command.args([
        "-P".to_string(),
        "BoBe Body Hub".to_string(),
        "_bobe-body-hub._tcp".to_string(),
        "local".to_string(),
        config.port.to_string(),
        config.advertised_hostname.trim_end_matches('.').to_string(),
        address.to_string(),
        "pv=1".to_string(),
        "path=/body/v1".to_string(),
        "tls=1".to_string(),
        format!("auth={BODY_MDNS_AUTH}"),
        format!("kid={}", config.trust_key_id),
        "scope=local".to_string(),
    ]);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command.kill_on_drop(true);
    command
}

#[cfg(target_os = "macos")]
struct NativeMdnsRegistration {
    cancel: DropGuard,
    task: AbortOnDropHandle<()>,
}

#[cfg(target_os = "macos")]
impl NativeMdnsRegistration {
    async fn shutdown(self) {
        let Self { cancel, task } = self;
        drop(cancel);
        drop(task.await);
    }
}

#[cfg(target_os = "macos")]
async fn supervise_native_mdns(config: BodyConfig, cancel: CancellationToken) {
    let mut child: Option<Child> = None;
    let mut registered_address = None;
    loop {
        match primary_ipv4() {
            Ok(address) => {
                let stopped = child
                    .as_mut()
                    .is_none_or(|process| process.try_wait().ok().flatten().is_some());
                if registered_address != Some(address) || stopped {
                    if let Some(mut process) = child.take() {
                        drop(process.kill().await);
                        drop(process.wait().await);
                    }
                    match native_mdns_command(&config, address).spawn() {
                        Ok(process) => {
                            info!(%address, "body.mdns_native_registered");
                            child = Some(process);
                            registered_address = Some(address);
                        }
                        Err(error) => {
                            warn!(%error, "body.mdns_register_failed");
                            registered_address = None;
                        }
                    }
                }
            }
            Err(error) => {
                warn!(%error, "body.mdns_address_failed");
            }
        }

        if cancel
            .run_until_cancelled(tokio::time::sleep(Duration::from_secs(5)))
            .await
            .is_none()
        {
            break;
        }
    }
    if let Some(mut process) = child {
        drop(process.kill().await);
        drop(process.wait().await);
    }
}

#[cfg(target_os = "macos")]
fn start_mdns(config: &BodyConfig) -> Option<MdnsRegistration> {
    let cancel = CancellationToken::new();
    let task = AbortOnDropHandle::new(tokio::spawn(supervise_native_mdns(
        config.clone(),
        cancel.clone(),
    )));
    Some(MdnsRegistration::Native(NativeMdnsRegistration {
        cancel: cancel.drop_guard(),
        task,
    }))
}

#[cfg(not(target_os = "macos"))]
fn start_mdns(config: &BodyConfig) -> Option<MdnsRegistration> {
    let daemon = mdns_sd::ServiceDaemon::new().ok()?;
    let service = match body_mdns_service(config) {
        Ok(service) => service,
        Err(error) => {
            warn!(%error, "body.mdns_service_failed");
            return None;
        }
    };
    if let Err(error) = daemon.register(service) {
        warn!(%error, "body.mdns_register_failed");
        return None;
    }
    Some(MdnsRegistration::Library(daemon))
}

pub(crate) async fn run(
    gateway: Arc<BodyGateway>,
    identities: Arc<BodyIdentityIndex>,
    endpoints: Arc<crate::db::endpoint_repo::EndpointRepo>,
    registry_lock: Arc<tokio::sync::Mutex<()>>,
    config: BodyConfig,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    anyhow::ensure!(config.enabled, "BodyLink gateway is disabled");
    if let Some(error) = config.validation_errors().into_iter().next() {
        anyhow::bail!(error);
    }
    let host = config
        .host
        .parse::<IpAddr>()
        .context("parse BodyLink host")?;
    let address = SocketAddr::new(host, config.port);
    let adapter_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.adapter_port);
    let adapter_enabled = !config.adapter_token.expose_secret().is_empty();
    let tls = tls_config(&config, Arc::clone(&identities))?;
    let mdns = (!host.is_loopback() && config.mdns_enabled)
        .then(|| start_mdns(&config))
        .flatten();

    let body_app = Router::new()
        .route("/body/v1", get(body_stream))
        .layer(axum::Extension(endpoints))
        .layer(axum::Extension(registry_lock))
        .with_state(Arc::clone(&gateway));
    let adapter_app = Router::new()
        .route("/body/adapter", get(adapter_stream))
        .layer(axum_middleware::from_fn(fixed_bearer_auth))
        .layer(axum::Extension(ApiToken::adapter(
            config.adapter_token.clone(),
        )))
        .with_state(Arc::clone(&gateway));

    let body_handle = axum_server::Handle::new();
    let adapter_handle = axum_server::Handle::new();
    let body_shutdown = body_handle.clone();
    let adapter_shutdown = adapter_handle.clone();
    tokio::spawn(async move {
        shutdown.cancelled().await;
        body_shutdown.graceful_shutdown(Some(Duration::from_secs(5)));
        adapter_shutdown.graceful_shutdown(Some(Duration::from_secs(5)));
    });

    info!(%address, adapter = ?adapter_enabled.then_some(adapter_address), "body.gateway_listening");
    let body_server = axum_server::Server::bind(address)
        .acceptor(crate::body::tls_identity::BodyIdentityAcceptor::new(
            axum_server::tls_rustls::RustlsAcceptor::new(tls),
            Arc::clone(&identities),
        ))
        .handle(body_handle)
        .serve(body_app.into_make_service());
    let adapter_server = axum_server::bind(adapter_address)
        .handle(adapter_handle)
        .serve(adapter_app.into_make_service());
    gateway.mark_running();
    let result = if adapter_enabled {
        tokio::try_join!(body_server, adapter_server).map(|_| ())
    } else {
        body_server.await
    };
    gateway.mark_stopped();
    if let Some(mdns) = mdns {
        mdns.shutdown().await;
    }
    result.context("BodyLink gateway server")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_mdns_service_tracks_host_addresses() {
        let config = BodyConfig {
            trust_key_id: "test-kid".into(),
            ..BodyConfig::default()
        };
        let service = body_mdns_service(&config).unwrap();

        assert!(service.is_addr_auto());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_mdns_registers_the_pinned_hostname_and_protocol() {
        let config = BodyConfig {
            advertised_hostname: "bobe-body-hub.local".into(),
            trust_key_id: "test-kid".into(),
            ..BodyConfig::default()
        };
        let command = native_mdns_command(&config, Ipv4Addr::new(192, 0, 2, 10));
        let arguments = command
            .as_std()
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(
            arguments
                .windows(2)
                .any(|pair| { pair == ["bobe-body-hub.local", "192.0.2.10"] })
        );
        assert!(arguments.iter().any(|value| value == "auth=mtls-pinned"));
        assert!(arguments.iter().any(|value| value == "kid=test-kid"));
    }
}
