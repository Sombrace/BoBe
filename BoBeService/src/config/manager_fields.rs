use std::collections::HashMap;

use tracing::warn;

use crate::config::Config;

pub(crate) fn apply(config: &mut Config, changes: &HashMap<String, serde_json::Value>) {
    macro_rules! set_parsed {
        ($field:expr, $value:expr, $key:expr) => {
            match serde::Deserialize::deserialize($value) {
                Ok(v) => $field = v,
                Err(_) => warn!(field = $key, "config_manager.parse_failed"),
            }
        };
    }

    macro_rules! set_opt_string {
        ($field:expr, $value:expr, $key:expr) => {
            match $value {
                serde_json::Value::Null => $field = None,
                serde_json::Value::String(s) if s.is_empty() => $field = None,
                value => match <String as serde::Deserialize>::deserialize(value) {
                    Ok(value) => $field = Some(value),
                    Err(_) => warn!(field = $key, "config_manager.parse_failed"),
                },
            }
        };
    }

    for (key, value) in changes {
        let key = key.as_str();
        match key {
            "agent.enabled_tools" => set_parsed!(config.agent.enabled_tools, value, key),
            "agent.execution" => set_parsed!(config.agent.execution, value, key),
            "server.host" => set_parsed!(config.server.host, value, key),
            "server.port" => set_parsed!(config.server.port, value, key),
            "server.mdns_enabled" => set_parsed!(config.server.mdns_enabled, value, key),
            "server.cors_origins" => set_parsed!(config.server.cors_origins, value, key),
            "server.allowed_hosts" => set_parsed!(config.server.allowed_hosts, value, key),
            "server.tls_cert_path" => set_opt_string!(config.server.tls_cert_path, value, key),
            "server.tls_key_path" => set_opt_string!(config.server.tls_key_path, value, key),
            "database.url" => set_parsed!(config.database.url, value, key),
            "body.theme" => set_parsed!(config.body.theme, value, key),
            "capture.enabled" => set_parsed!(config.capture.enabled, value, key),
            "capture.interval_seconds" => {
                set_parsed!(config.capture.interval_seconds, value, key);
            }
            "checkin.enabled" => set_parsed!(config.checkin.enabled, value, key),
            "checkin.times" => set_parsed!(config.checkin.times, value, key),
            "checkin.jitter_minutes" => {
                set_parsed!(config.checkin.jitter_minutes, value, key);
            }
            "checkin.interval_minutes" => {
                set_parsed!(config.checkin.interval_minutes, value, key);
            }
            "conversation.inactivity_timeout_seconds" => {
                set_parsed!(config.conversation.inactivity_timeout_seconds, value, key);
            }
            "conversation.auto_close_minutes" => {
                set_parsed!(config.conversation.auto_close_minutes, value, key);
            }
            "logging.level" => set_parsed!(config.logging.level, value, key),
            "logging.json" => set_parsed!(config.logging.json, value, key),
            "logging.file" => set_parsed!(config.logging.file, value, key),
            "decision.cooldown_minutes" => {
                set_parsed!(config.decision.cooldown_minutes, value, key);
            }
            "decision.extended_cooldown_minutes" => {
                set_parsed!(config.decision.extended_cooldown_minutes, value, key);
            }
            "mcp.enabled" => set_parsed!(config.mcp.enabled, value, key),
            "mcp.config_file" => set_parsed!(config.mcp.config_file, value, key),
            "mcp.blocked_commands" => set_parsed!(config.mcp.blocked_commands, value, key),
            "mcp.dangerous_env_keys" => set_parsed!(config.mcp.dangerous_env_keys, value, key),
            "goals.check_interval_seconds" => {
                set_parsed!(config.goals.check_interval_seconds, value, key);
            }
            "goals.enabled" => set_parsed!(config.goals.enabled, value, key),
            "memory.enabled" => set_parsed!(config.memory.enabled, value, key),
            "voice.enabled" => set_parsed!(config.voice.enabled, value, key),
            "voice.persona" => set_parsed!(config.voice.persona, value, key),
            "voice.speed" => set_parsed!(config.voice.speed, value, key),
            "voice.stt_language" => set_parsed!(config.voice.stt_language, value, key),
            "voice.pause_sensitivity" => {
                set_parsed!(config.voice.pause_sensitivity, value, key);
            }
            "voice.show_partial_caption" => {
                set_parsed!(config.voice.show_partial_caption, value, key);
            }
            "seed_default_documents" => set_parsed!(config.seed_default_documents, value, key),
            _ => {}
        }
    }
}

pub(crate) fn equivalent_json_value(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    if left == right {
        return true;
    }
    match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => left.total_cmp(&right).is_eq(),
        _ => false,
    }
}

pub(crate) fn compatible_json_type(
    expected: &serde_json::Value,
    received: &serde_json::Value,
) -> bool {
    expected.is_null()
        || received.is_null()
        || matches!(
            (expected, received),
            (serde_json::Value::Bool(_), serde_json::Value::Bool(_))
                | (serde_json::Value::Number(_), serde_json::Value::Number(_))
                | (serde_json::Value::String(_), serde_json::Value::String(_))
                | (serde_json::Value::Array(_), serde_json::Value::Array(_))
                | (serde_json::Value::Object(_), serde_json::Value::Object(_))
        )
}

pub(crate) fn value(config: &Config, key: &str) -> Option<serde_json::Value> {
    let serialized = serde_json::to_value(config).ok()?;
    key.split('.')
        .try_fold(&serialized, |value, component| value.get(component))
        .cloned()
}

pub(crate) fn is_known_key(key: &str) -> bool {
    matches!(
        key,
        "server.host"
            | "agent.enabled_tools"
            | "agent.execution"
            | "server.port"
            | "server.mdns_enabled"
            | "server.cors_origins"
            | "server.allowed_hosts"
            | "server.tls_cert_path"
            | "server.tls_key_path"
            | "database.url"
            | "body.theme"
            | "capture.enabled"
            | "capture.interval_seconds"
            | "checkin.enabled"
            | "checkin.times"
            | "checkin.jitter_minutes"
            | "checkin.interval_minutes"
            | "conversation.inactivity_timeout_seconds"
            | "conversation.auto_close_minutes"
            | "logging.level"
            | "logging.json"
            | "logging.file"
            | "decision.cooldown_minutes"
            | "decision.extended_cooldown_minutes"
            | "mcp.enabled"
            | "mcp.config_file"
            | "mcp.blocked_commands"
            | "mcp.dangerous_env_keys"
            | "goals.check_interval_seconds"
            | "goals.enabled"
            | "memory.enabled"
            | "voice.enabled"
            | "voice.persona"
            | "voice.speed"
            | "voice.stt_language"
            | "voice.pause_sensitivity"
            | "voice.show_partial_caption"
            | "seed_default_documents"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BodyTheme;
    use serde_json::json;

    fn changes(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn numeric_values_compare_across_json_integer_and_float_forms() {
        assert!(equivalent_json_value(&json!(2.0), &json!(2)));
        assert!(equivalent_json_value(&json!(-3), &json!(-3.0)));
        assert!(!equivalent_json_value(&json!(2.5), &json!(2)));
        assert!(!equivalent_json_value(&json!("2"), &json!(2)));
    }

    #[test]
    fn known_field_happy_path_mutates() {
        let mut config = Config::default();
        apply(
            &mut config,
            &changes(&[("capture.interval_seconds", json!(42))]),
        );
        assert_eq!(config.capture.interval_seconds, 42);
    }

    #[test]
    fn unknown_field_is_noop() {
        let mut config = Config::default();
        let before = config.capture.interval_seconds;
        apply(
            &mut config,
            &changes(&[("nonexistent.field", json!("anything"))]),
        );
        assert_eq!(config.capture.interval_seconds, before);
    }

    #[test]
    fn wrong_type_is_skipped() {
        let mut config = Config::default();
        let before = config.capture.interval_seconds;
        apply(
            &mut config,
            &changes(&[("capture.interval_seconds", json!("not a number"))]),
        );
        assert_eq!(config.capture.interval_seconds, before);
    }

    #[test]
    fn body_theme_key_updates_config() {
        let mut config = Config::default();
        apply(
            &mut config,
            &changes(&[("body.theme", json!("solar-wide-awake"))]),
        );
        assert_eq!(config.body.theme, BodyTheme::SolarWideAwake);
    }
}
