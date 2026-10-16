use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BodyTheme {
    #[default]
    WarmAperture,
    WarmLintel,
    WarmLedge,
    KettleTallBright,
    KettleSoftBowl,
    SolarWideAwake,
    SolarCrown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct BodyConfig {
    pub(crate) enabled: bool,
    pub(crate) host: String,
    pub(crate) advertised_hostname: String,
    pub(crate) port: u16,
    pub(crate) adapter_port: u16,
    #[serde(skip_serializing)]
    pub(crate) adapter_token: SecretString,
    pub(crate) tls_cert_path: Option<String>,
    pub(crate) tls_key_path: Option<String>,
    pub(crate) client_ca_path: Option<String>,
    pub(crate) trust_key_id: String,
    pub(crate) controller_epoch: u64,
    pub(crate) mdns_enabled: bool,
    pub(crate) theme: BodyTheme,
    pub(crate) update_offer_path: Option<String>,
}

impl Default for BodyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            advertised_hostname: "bobe-body-hub.local".into(),
            port: 8_767,
            adapter_port: 8_768,
            adapter_token: SecretString::default(),
            tls_cert_path: None,
            tls_key_path: None,
            client_ca_path: None,
            trust_key_id: String::new(),
            controller_epoch: 1,
            mdns_enabled: true,
            theme: BodyTheme::default(),
            update_offer_path: None,
        }
    }
}

const MAX_CONTROLLER_EPOCH: u64 = 9_007_199_254_740_991;

impl BodyConfig {
    pub(crate) fn validation_errors(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut errors = Vec::new();
        if !(1..=MAX_CONTROLLER_EPOCH).contains(&self.controller_epoch) {
            errors.push("BodyLink controller_epoch must be a positive JSON-safe integer".into());
        }
        if self.trust_key_id.trim().is_empty() {
            errors.push("BodyLink trust_key_id is required".into());
        }
        if !self.adapter_token.expose_secret().is_empty() && self.port == self.adapter_port {
            errors.push("BodyLink body and adapter ports must differ".into());
        }
        if self.host.parse::<std::net::IpAddr>().is_err() {
            errors.push(format!("BodyLink host is not an IP address: {}", self.host));
        }
        if let Err(error) = self.validate_tls() {
            errors.push(error);
        }
        errors
    }

    fn validate_tls(&self) -> Result<(), String> {
        let certificate = self
            .tls_cert_path
            .as_deref()
            .ok_or("BodyLink requires tls_cert_path")?;
        let key = self
            .tls_key_path
            .as_deref()
            .ok_or("BodyLink requires tls_key_path")?;
        let client_ca = self
            .client_ca_path
            .as_deref()
            .ok_or("BodyLink requires client_ca_path")?;
        let chain = load_certificates(Path::new(certificate))?;
        if chain.is_empty() {
            return Err(format!("BodyLink server chain is empty: {certificate}"));
        }
        let private_key = PrivateKeyDer::from_pem_file(key)
            .map_err(|error| format!("parse BodyLink private key {key}: {error}"))?;
        let mut roots = rustls::RootCertStore::empty();
        for root in load_certificates(Path::new(client_ca))? {
            roots
                .add(root)
                .map_err(|error| format!("add BodyLink client CA certificate: {error}"))?;
        }
        if roots.is_empty() {
            return Err(format!("BodyLink client CA file is empty: {client_ca}"));
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::clone(&provider),
        )
        .build()
        .map_err(|error| format!("build BodyLink client certificate verifier: {error}"))?;
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|error| format!("BodyLink TLS protocol configuration: {error}"))?
            .with_client_cert_verifier(verifier)
            .with_single_cert(chain, private_key)
            .map_err(|error| {
                format!("BodyLink certificate and private key do not match: {error}")
            })?;
        Ok(())
    }
}

fn load_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    CertificateDer::pem_file_iter(path)
        .map_err(|error| format!("open certificate {}: {error}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("parse certificate {}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on serialization failures")]
mod tests {
    use super::*;

    fn certificate_pair(name: &str) -> (String, String) {
        let key = rcgen::KeyPair::generate().expect("key");
        let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).expect("params");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let certificate = params.self_signed(&key).expect("certificate");
        (certificate.pem(), key.serialize_pem())
    }

    #[test]
    fn an_enabled_gateway_reports_every_invalid_setting_before_it_starts() {
        let root = std::env::temp_dir().join(format!("bobe-body-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");
        assert!(BodyConfig::default().validation_errors().is_empty());

        let broken = BodyConfig {
            enabled: true,
            controller_epoch: 0,
            adapter_token: SecretString::from("adapter".to_owned()),
            adapter_port: 8_767,
            host: "body.local".into(),
            tls_cert_path: Some(root.join("missing.pem").to_string_lossy().into_owned()),
            tls_key_path: Some(root.join("missing-key.pem").to_string_lossy().into_owned()),
            client_ca_path: Some(root.join("missing-ca.pem").to_string_lossy().into_owned()),
            ..BodyConfig::default()
        };
        let errors = broken.validation_errors();
        for expected in [
            "controller_epoch",
            "trust_key_id",
            "ports must differ",
            "host is not an IP address",
            "missing.pem",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "{expected}: {errors:?}"
            );
        }

        let (server, server_key) = certificate_pair("bobe-body-hub.local");
        let (client_ca, _) = certificate_pair("bobe-body-ca");
        let (_, other_key) = certificate_pair("other");
        for (name, content) in [
            ("server.pem", server.as_str()),
            ("server-key.pem", server_key.as_str()),
            ("client-ca.pem", client_ca.as_str()),
            ("other-key.pem", other_key.as_str()),
        ] {
            std::fs::write(root.join(name), content).expect("write PEM");
        }
        let path = |name: &str| Some(root.join(name).to_string_lossy().into_owned());
        let valid = BodyConfig {
            enabled: true,
            trust_key_id: "test-key".into(),
            tls_cert_path: path("server.pem"),
            tls_key_path: path("server-key.pem"),
            client_ca_path: path("client-ca.pem"),
            ..BodyConfig::default()
        };
        assert!(
            valid.validation_errors().is_empty(),
            "{:?}",
            valid.validation_errors()
        );

        let mismatched = BodyConfig {
            tls_key_path: path("other-key.pem"),
            ..valid.clone()
        };
        assert!(
            mismatched
                .validation_errors()
                .iter()
                .any(|error| error.contains("do not match")),
            "{:?}",
            mismatched.validation_errors()
        );
        std::fs::remove_dir_all(root).expect("remove root");
    }

    #[test]
    fn body_theme_accepts_exact_ids_and_rejects_unknown_values() {
        let expected = [
            ("warm-aperture", BodyTheme::WarmAperture),
            ("warm-lintel", BodyTheme::WarmLintel),
            ("warm-ledge", BodyTheme::WarmLedge),
            ("kettle-tall-bright", BodyTheme::KettleTallBright),
            ("kettle-soft-bowl", BodyTheme::KettleSoftBowl),
            ("solar-wide-awake", BodyTheme::SolarWideAwake),
            ("solar-crown", BodyTheme::SolarCrown),
        ];

        for (id, theme) in expected {
            let decoded =
                serde_json::from_str::<BodyTheme>(&format!("\"{id}\"")).expect("decode body theme");
            assert_eq!(decoded, theme);
            assert_eq!(
                serde_json::to_string(&theme).expect("encode body theme"),
                format!("\"{id}\"")
            );
        }
        assert!(serde_json::from_str::<BodyTheme>("\"warm_aperture\"").is_err());
        assert!(serde_json::from_str::<BodyTheme>("\"other\"").is_err());
    }
}
