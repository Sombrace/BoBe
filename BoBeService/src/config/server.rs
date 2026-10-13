use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct ServerConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) mdns_enabled: bool,
    pub(crate) cors_origins: Vec<String>,
    pub(crate) allowed_hosts: Vec<String>,
    #[serde(skip)]
    pub(crate) api_token: secrecy::SecretString,
    pub(crate) tls_cert_path: Option<String>,
    pub(crate) tls_key_path: Option<String>,
}

impl ServerConfig {
    pub(crate) fn is_networked(&self) -> bool {
        self.host
            .parse::<std::net::IpAddr>()
            .ok()
            .is_none_or(|address| !address.is_loopback())
            || !self.allowed_hosts.is_empty()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: crate::constants::DEFAULT_DAEMON_PORT,
            mdns_enabled: false,
            cors_origins: Vec::new(),
            allowed_hosts: Vec::new(),
            api_token: secrecy::SecretString::default(),
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct DatabaseConfig {
    pub(crate) url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct LoggingConfig {
    pub(crate) level: String,
    pub(crate) json: bool,
    pub(crate) file: Option<String>,
    pub(crate) retention_days: u32,
    pub(crate) retention_count: u32,
    pub(crate) retention_total_bytes: u64,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "INFO".into(),
            json: false,
            file: None,
            retention_days: 14,
            retention_count: 14,
            retention_total_bytes: 100 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networked_includes_reverse_proxy_exposure() {
        let mut config = ServerConfig::default();
        assert!(!config.is_networked());
        config.allowed_hosts.push("bobe.example.test".into());
        assert!(config.is_networked());
        config.allowed_hosts.clear();
        config.host = "0.0.0.0".into();
        assert!(config.is_networked());
    }
}
