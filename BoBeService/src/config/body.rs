use secrecy::SecretString;
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

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on serialization failures")]
mod tests {
    use super::*;

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
