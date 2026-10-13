use std::path::PathBuf;

use serde::Serialize;

use crate::config::LoadedConfig;

#[derive(Debug, Serialize)]
pub(crate) struct ConfigPathsReport {
    pub(crate) config_file: PathBuf,
    pub(crate) config_file_exists: bool,
    pub(crate) data_root: PathBuf,
    pub(crate) database_url: String,
    pub(crate) runtime_lock: PathBuf,
    pub(crate) models_dir: PathBuf,
    pub(crate) secrets_file: PathBuf,
}

pub(crate) fn paths(loaded: &LoadedConfig) -> ConfigPathsReport {
    ConfigPathsReport {
        config_file_exists: loaded.config_file.is_file(),
        config_file: loaded.config_file.clone(),
        data_root: loaded.paths.root().to_path_buf(),
        database_url: loaded.paths.database_url().to_owned(),
        runtime_lock: loaded.paths.runtime_lock(),
        models_dir: loaded.paths.models_dir(),
        secrets_file: loaded.paths.secrets_file(),
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ValidationFinding {
    pub(crate) id: &'static str,
    pub(crate) severity: &'static str,
    pub(crate) message: String,
    pub(crate) remediation: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ValidationReport {
    pub(crate) valid: bool,
    pub(crate) findings: Vec<ValidationFinding>,
}

pub(crate) fn validate(loaded: &LoadedConfig) -> ValidationReport {
    let config = &loaded.config;
    let mut findings = Vec::new();
    if config.server.host.parse::<std::net::IpAddr>().is_err() {
        findings.push(error(
            "server.bind_host_invalid",
            format!("server.host is not an IP address: {}", config.server.host),
            "Set server.host to an IPv4 or IPv6 address.",
        ));
    }
    let host_is_loopback = config
        .server
        .host
        .parse::<std::net::IpAddr>()
        .is_ok_and(|host| host.is_loopback());
    let remotely_exposed = !host_is_loopback || !config.server.allowed_hosts.is_empty();
    if remotely_exposed && config.server.allowed_hosts.is_empty() {
        findings.push(error(
            "server.remote_requires_allowed_hosts",
            "remote exposure requires at least one allowed host".into(),
            "Set server.allowed_hosts before binding remotely.",
        ));
    }
    if config.server.tls_cert_path.is_some() != config.server.tls_key_path.is_some() {
        findings.push(error(
            "server.remote_tls_pair_incomplete",
            "manual TLS configuration requires both certificate and private key".into(),
            "Set both server.tls_cert_path and server.tls_key_path, or unset both to use automatic TLS.",
        ));
    }
    for (id, label, path) in [
        (
            "server.tls_cert_missing",
            "TLS certificate",
            config.server.tls_cert_path.as_deref(),
        ),
        (
            "server.tls_key_missing",
            "TLS private key",
            config.server.tls_key_path.as_deref(),
        ),
    ] {
        if let Some(path) = path
            && !std::path::Path::new(path).is_file()
        {
            findings.push(error(
                id,
                format!("{label} does not exist: {path}"),
                "Correct the configured path before starting the daemon.",
            ));
        }
    }
    if !loaded.paths.root().is_absolute() {
        findings.push(error(
            "paths.data_root_absolute",
            "data root must be absolute".into(),
            "Set BOBE_DATA_DIR to an absolute path.",
        ));
    }
    if !loaded.paths.database_url().starts_with("sqlite:") {
        findings.push(ValidationFinding {
            id: "database.unsupported_url",
            severity: "warning",
            message: "only SQLite database URLs are currently supported".into(),
            remediation: Some("Use a sqlite: database URL.".into()),
        });
    }
    ValidationReport {
        valid: !findings.iter().any(|finding| finding.severity == "error"),
        findings,
    }
}

fn error(id: &'static str, message: String, remediation: &str) -> ValidationFinding {
    ValidationFinding {
        id,
        severity: "error",
        message,
        remediation: Some(remediation.into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn serialized_config_does_not_expose_tokens() {
        let mut config = crate::config::Config::default();
        config.server.api_token = secrecy::SecretString::from("server-secret".to_owned());
        config.body.adapter_token = secrecy::SecretString::from("adapter-secret".to_owned());
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("server-secret"));
        assert!(!json.contains("adapter-secret"));
        assert!(!json.contains("api_token"));
        assert!(!json.contains("adapter_token"));
    }
}
