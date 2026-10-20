use super::manager_fields as fields;
use super::manager_persistence as persistence;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use tracing::info;

use crate::config::Config;

static STATIC_FIELDS: &[&str] = &[
    "agent.enabled_tools",
    "agent.execution",
    "server.host",
    "server.port",
    "server.allowed_hosts",
    "server.tls_cert_path",
    "server.tls_key_path",
    "database.url",
    "server.mdns_enabled",
    "logging.file",
    "logging.level",
    "logging.json",
    "server.cors_origins",
    "mcp.config_file",
    "mcp.blocked_commands",
    "mcp.dangerous_env_keys",
    "memory.enabled",
    "goals.enabled",
    "seed_default_documents",
];

static HOT_SWAP_FIELDS: &[&str] = &[
    "capture.enabled",
    "capture.interval_seconds",
    "decision.cooldown_minutes",
    "decision.extended_cooldown_minutes",
    "checkin.enabled",
    "checkin.times",
    "checkin.interval_minutes",
    "checkin.jitter_minutes",
    "goals.check_interval_seconds",
    "conversation.inactivity_timeout_seconds",
    "conversation.auto_close_minutes",
    "body.theme",
    "mcp.enabled",
    "voice.enabled",
    "voice.persona",
    "voice.speed",
    "voice.stt_language",
    "voice.pause_sensitivity",
    "voice.show_partial_caption",
];

#[derive(Debug)]
pub(crate) struct UpdateResult {
    pub(crate) applied_fields: Vec<String>,
    pub(crate) restart_required_fields: Vec<String>,
    pub(crate) persist_failed: bool,
}

pub(crate) struct ConfigManager {
    config: Arc<ArcSwap<Config>>,
    config_root: std::path::PathBuf,
    write_lock: Arc<std::sync::Mutex<()>>,
}

impl ConfigManager {
    pub(crate) fn new(
        config: Arc<ArcSwap<Config>>,
        config_root: std::path::PathBuf,
        write_lock: Arc<std::sync::Mutex<()>>,
    ) -> Self {
        Self {
            config,
            config_root,
            write_lock,
        }
    }

    pub(crate) fn unset(&self, key: &str) -> Result<UpdateResult, crate::error::AppError> {
        let _write_guard = self.write_lock.lock().map_err(|_| {
            crate::error::AppError::Io(std::io::Error::other("config write lock poisoned"))
        })?;
        let dotted = key.to_owned();
        if !fields::is_known_key(&dotted) {
            return Err(crate::error::AppError::Validation(format!(
                "unknown configuration key: {key}"
            )));
        }
        let static_field = STATIC_FIELDS.contains(&dotted.as_str());
        let hot_field = HOT_SWAP_FIELDS.contains(&dotted.as_str());
        if !static_field && !hot_field {
            return Err(crate::error::AppError::Validation(format!(
                "configuration key cannot be changed at runtime: {dotted}"
            )));
        }
        if !persistence::remove(&self.config_root, &dotted) {
            return Err(crate::error::AppError::Io(std::io::Error::other(
                "failed to remove persisted configuration override",
            )));
        }
        let loaded = Config::load_from(&self.config_root.join("config.toml"), &self.config_root)?;
        if hot_field {
            let value = fields::value(&loaded.config, &dotted).ok_or_else(|| {
                crate::error::AppError::Config(format!(
                    "could not resolve effective configuration field: {dotted}"
                ))
            })?;
            let mut updated = (**self.config.load()).clone();
            fields::apply(&mut updated, &HashMap::from([(dotted.clone(), value)]));
            self.config.store(Arc::new(updated));
        }
        Ok(UpdateResult {
            applied_fields: hot_field.then(|| dotted.clone()).into_iter().collect(),
            restart_required_fields: static_field.then(|| dotted.clone()).into_iter().collect(),
            persist_failed: false,
        })
    }

    pub(crate) fn update(
        &self,
        changes: &HashMap<String, serde_json::Value>,
    ) -> Result<UpdateResult, crate::error::AppError> {
        let _write_guard = self.write_lock.lock().map_err(|_| {
            crate::error::AppError::Io(std::io::Error::other("config write lock poisoned"))
        })?;
        let mut result = UpdateResult {
            applied_fields: Vec::new(),
            restart_required_fields: Vec::new(),
            persist_failed: false,
        };
        let static_set: HashSet<&str> = STATIC_FIELDS.iter().copied().collect();
        let hot_set: HashSet<&str> = HOT_SWAP_FIELDS.iter().copied().collect();
        let current = self.config.load_full();
        let mut toml_changes = BTreeMap::new();
        let mut hot_changes = HashMap::new();
        let mut candidate = (*current).clone();

        for (key, value) in changes {
            let dotted = key.clone();
            let normalized = dotted.as_str();
            if !static_set.contains(normalized) && !hot_set.contains(normalized) {
                return Err(crate::error::AppError::Validation(format!(
                    "configuration key cannot be changed at runtime: {dotted}"
                )));
            }
            let current_value = fields::value(&candidate, &dotted).ok_or_else(|| {
                crate::error::AppError::Validation(format!("unknown configuration key: {dotted}"))
            })?;
            if !fields::compatible_json_type(&current_value, value) {
                return Err(crate::error::AppError::Validation(format!(
                    "invalid value type for {dotted}"
                )));
            }
            fields::apply(
                &mut candidate,
                &HashMap::from([(dotted.clone(), value.clone())]),
            );
            candidate.agent.execution.validate()?;
            let applied = fields::value(&candidate, &dotted);
            let equivalent = if dotted == "agent.enabled_tools" {
                serde_json::from_value::<std::collections::BTreeSet<crate::config::AgentTool>>(
                    value.clone(),
                )
                .is_ok_and(|selection| selection == candidate.agent.enabled_tools)
            } else {
                applied
                    .as_ref()
                    .is_some_and(|applied| fields::equivalent_json_value(applied, value))
            };
            if !equivalent {
                return Err(crate::error::AppError::Validation(format!(
                    "invalid value for {dotted}: {value}"
                )));
            }
            if static_set.contains(normalized) {
                result.restart_required_fields.push(key.clone());
                toml_changes.insert(
                    dotted,
                    applied.ok_or_else(|| {
                        crate::error::AppError::Config("validated setting disappeared".into())
                    })?,
                );
            } else if hot_set.contains(normalized) {
                hot_changes.insert(dotted.clone(), value.clone());
                result.applied_fields.push(key.clone());
                toml_changes.insert(dotted, value.clone());
            }
        }

        if !toml_changes.is_empty() && !persistence::persist(&self.config_root, &toml_changes) {
            result.persist_failed = true;
            result.applied_fields.clear();
            result.restart_required_fields.clear();
        } else if !hot_changes.is_empty() {
            let mut live = (*current).clone();
            fields::apply(&mut live, &hot_changes);
            self.config.store(Arc::new(live));
            info!("config_manager.config_swapped");
        }
        info!(
            applied = ?result.applied_fields,
            restart_required = ?result.restart_required_fields,
            "config_manager.update_complete"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> ConfigManager {
        let root =
            std::env::temp_dir().join(format!("bobe-config-manager-{}", uuid::Uuid::new_v4()));
        ConfigManager::new(
            Arc::new(ArcSwap::from_pointee(Config::default())),
            root,
            Arc::new(std::sync::Mutex::new(())),
        )
    }

    #[test]
    fn integral_json_updates_float_configuration_fields() {
        let manager = manager();
        let changes =
            HashMap::from([("goals.check_interval_seconds".into(), serde_json::json!(2))]);

        let result = manager.update(&changes).unwrap();

        assert_eq!(result.applied_fields, ["goals.check_interval_seconds"]);
        assert!((manager.config.load().goals.check_interval_seconds - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn tool_selection_normalizes_order_without_changing_active_tools() {
        let manager = manager();
        let changes = HashMap::from([(
            "agent.enabled_tools".into(),
            serde_json::json!(["background_jobs", "read", "bash", "read"]),
        )]);
        let result = manager.update(&changes).unwrap();
        assert!(!result.persist_failed);
        assert_eq!(result.restart_required_fields, ["agent.enabled_tools"]);
        assert_eq!(
            manager.config.load().agent,
            crate::config::AgentConfig::default()
        );
        let saved = Config::load_from(
            &manager.config_root.join("config.toml"),
            &manager.config_root,
        )
        .unwrap();
        assert_eq!(
            saved.config.agent.enabled_tools,
            [
                crate::config::AgentTool::Read,
                crate::config::AgentTool::Bash,
                crate::config::AgentTool::BackgroundJobs,
            ]
            .into_iter()
            .collect()
        );
        std::fs::remove_dir_all(&manager.config_root).unwrap();
    }

    #[test]
    fn hot_fields_apply_without_restart() {
        let manager = manager();
        let changes = HashMap::from([("capture.enabled".into(), serde_json::json!(false))]);
        let result = manager.update(&changes).unwrap();
        assert_eq!(result.applied_fields, ["capture.enabled"]);
        assert_eq!(result.restart_required_fields, Vec::<String>::new());
        assert!(!manager.config.load().capture.enabled);
    }

    #[test]
    fn unsetting_hot_field_does_not_publish_pending_restart_field() {
        let manager = manager();
        std::fs::create_dir_all(&manager.config_root).unwrap();
        std::fs::write(
            manager.config_root.join("config.toml"),
            "[capture]\nenabled = false\n[logging]\nlevel = 'debug'\n",
        )
        .unwrap();
        let before = manager.config.load().logging.level.clone();

        let result = manager.unset("capture.enabled").unwrap();

        assert_eq!(result.applied_fields, ["capture.enabled"]);
        let config = manager.config.load();
        assert!(config.capture.enabled);
        assert_eq!(config.logging.level, before);
        drop(std::fs::remove_dir_all(&manager.config_root));
    }

    #[test]
    fn startup_fields_persist_without_swapping_live_config() {
        let manager = manager();
        let before = manager.config.load().logging.level.clone();
        let changes = HashMap::from([("logging.level".into(), serde_json::json!("debug"))]);
        let result = manager.update(&changes).unwrap();
        assert_eq!(result.applied_fields, Vec::<String>::new());
        assert_eq!(result.restart_required_fields, ["logging.level"]);
        assert_eq!(manager.config.load().logging.level, before);
    }

    #[test]
    fn mixed_update_does_not_publish_restart_bound_fields() {
        let manager = manager();
        let before = manager.config.load().logging.level.clone();
        let changes = HashMap::from([
            ("capture.enabled".into(), serde_json::json!(false)),
            ("logging.level".into(), serde_json::json!("debug")),
        ]);

        let result = manager.update(&changes).unwrap();

        assert_eq!(result.applied_fields, ["capture.enabled"]);
        assert_eq!(result.restart_required_fields, ["logging.level"]);
        let config = manager.config.load();
        assert!(!config.capture.enabled);
        assert_eq!(config.logging.level, before);
    }
}
