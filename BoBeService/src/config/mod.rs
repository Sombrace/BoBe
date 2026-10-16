mod body;
pub(crate) mod manager;
pub(crate) mod manager_fields;
pub(crate) mod manager_persistence;
mod mcp;
pub(crate) mod provider_persistence;
mod providers;
mod runtime;
mod server;
mod voice;

use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

pub(crate) use agent::{AgentConfig, AgentTool, ExecutionLimits};
pub(crate) use body::{BodyConfig, BodyTheme};
pub(crate) use mcp::McpConfig;
pub(crate) use runtime::{
    CaptureConfig, CheckinConfig, ConversationConfig, DecisionConfig, GoalsConfig, MemoryConfig,
};
pub(crate) use server::{DatabaseConfig, LoggingConfig, ServerConfig};
pub(crate) use voice::{PauseSensitivity, VoiceConfig};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Config {
    pub(crate) data_dir: String,

    pub(crate) server: ServerConfig,
    pub(crate) body: BodyConfig,
    pub(crate) database: DatabaseConfig,
    pub(crate) capture: CaptureConfig,
    pub(crate) checkin: CheckinConfig,
    pub(crate) conversation: ConversationConfig,
    pub(crate) logging: LoggingConfig,
    pub(crate) decision: DecisionConfig,
    pub(crate) mcp: McpConfig,
    pub(crate) goals: GoalsConfig,
    pub(crate) memory: MemoryConfig,
    pub(crate) providers: crate::provider::ProvidersConfig,
    pub(crate) routes: crate::provider::ProviderRoutes,
    pub(crate) voice: VoiceConfig,
    pub(crate) agent: AgentConfig,

    pub(crate) seed_default_documents: bool,
}

impl Default for Config {
    fn default() -> Self {
        let (providers, routes) = providers::default_profiles_and_routes();
        Self {
            data_dir: String::new(),
            server: ServerConfig::default(),
            body: BodyConfig::default(),
            database: DatabaseConfig::default(),
            capture: CaptureConfig::default(),
            checkin: CheckinConfig::default(),
            conversation: ConversationConfig::default(),
            logging: LoggingConfig::default(),
            decision: DecisionConfig::default(),
            mcp: McpConfig::default(),
            goals: GoalsConfig::default(),
            memory: MemoryConfig::default(),
            providers,
            routes,
            voice: VoiceConfig::default(),
            agent: AgentConfig::default(),
            seed_default_documents: true,
        }
    }
}

pub(crate) struct LoadedConfig {
    pub(crate) config: Config,
    pub(crate) paths: crate::util::paths::DataPaths,
    pub(crate) config_file: std::path::PathBuf,
}

pub(crate) struct ServeConfig {
    pub(crate) loaded: LoadedConfig,
    pub(crate) purged_release_0_2: bool,
    pub(crate) interrupted_restore: crate::ops::backup::Reconciled,
}

fn normalize_optional_path(value: &mut Option<String>, config_root: &std::path::Path) {
    let Some(path) = value.as_deref() else {
        return;
    };
    if path.trim().is_empty() {
        *value = None;
        return;
    }
    let expanded = crate::util::paths::expand_tilde(path);
    let normalized = if expanded.is_absolute() {
        expanded
    } else {
        config_root.join(expanded)
    };
    *value = Some(normalized.to_string_lossy().into_owned());
}

fn validate_persisted_config(source: &str) -> Result<(), crate::error::AppError> {
    if source.is_empty() {
        return Ok(());
    }
    let deserializer = toml_edit::de::Deserializer::parse(source.to_owned())
        .map_err(|error| crate::error::AppError::Config(error.to_string()))?;
    let mut unknown = Vec::new();
    drop(
        serde_ignored::deserialize::<_, _, Config>(deserializer, |path| {
            unknown.push(path.to_string());
        })
        .map_err(|error| crate::error::AppError::Config(error.to_string()))?,
    );
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort();
    unknown.dedup();
    Err(crate::error::AppError::Config(format!(
        "unknown config.toml field{}: {}",
        if unknown.len() == 1 { "" } else { "s" },
        unknown.join(", ")
    )))
}

impl Config {
    pub(crate) fn load_with_paths() -> Result<LoadedConfig, crate::error::AppError> {
        let bootstrap_data_dir = crate::util::paths::bobe_data_dir()?;
        let config_path = bootstrap_data_dir.join("config.toml");
        Self::load_from(&config_path, &bootstrap_data_dir)
    }

    pub(crate) fn load_operational() -> Result<LoadedConfig, crate::error::AppError> {
        let mut loaded = Self::load_with_paths()?;
        if loaded.config.server.api_token.expose_secret().is_empty()
            && let Some(token) = crate::owner_token::read(&loaded.paths)?
        {
            loaded.config.server.api_token = secrecy::SecretString::from(token);
        }
        Ok(loaded)
    }

    pub(crate) async fn load_reconciled() -> Result<LoadedConfig, crate::error::AppError> {
        crate::ops::backup::reconcile_interrupted_restore(&crate::util::paths::bobe_data_dir()?)
            .await?;
        Self::load_operational()
    }

    pub(crate) async fn reconcile_locked(
        loaded: LoadedConfig,
        paths: &crate::util::paths::DataPaths,
    ) -> Result<(LoadedConfig, crate::ops::backup::Reconciled), crate::error::AppError> {
        let reconciled =
            crate::ops::backup::reconcile_restore_journals(paths, &loaded.config_file).await?;
        if !reconciled.changed_live_data() {
            return Ok((loaded, reconciled));
        }
        let reloaded = Self::load_operational()?;
        let reloaded_paths = reloaded.paths.clone().prepare_root()?;
        if reloaded_paths.root() != paths.root()
            || reloaded_paths.database_url() != paths.database_url()
        {
            return Err(crate::error::AppError::Conflict(
                "the reconciled configuration resolves a different BoBe data root".into(),
            ));
        }
        Ok((reloaded, reconciled))
    }

    pub(crate) async fn load_for_serve() -> Result<ServeConfig, crate::error::AppError> {
        let bootstrap_data_dir = crate::util::paths::bobe_data_dir()?;
        let interrupted_restore =
            crate::ops::backup::reconcile_interrupted_restore(&bootstrap_data_dir).await?;
        let purged_release_0_2 = crate::bootstrap::release_0_2::purge(&bootstrap_data_dir).await?;
        Ok(ServeConfig {
            loaded: Self::load_operational()?,
            purged_release_0_2,
            interrupted_restore,
        })
    }

    pub(crate) fn load_from(
        config_path: &std::path::Path,
        bootstrap_data_dir: &std::path::Path,
    ) -> Result<LoadedConfig, crate::error::AppError> {
        let source = match std::fs::read_to_string(config_path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        Self::load_source(config_path, bootstrap_data_dir, source)
    }

    fn load_source(
        config_path: &std::path::Path,
        bootstrap_data_dir: &std::path::Path,
        source: String,
    ) -> Result<LoadedConfig, crate::error::AppError> {
        validate_persisted_config(&source)?;
        let data_dir_str = bootstrap_data_dir.to_string_lossy().into_owned();

        let defaults = Self {
            data_dir: data_dir_str,
            database: DatabaseConfig { url: String::new() },
            ..Self::default()
        };

        let mut config: Config = Figment::new()
            .merge(Serialized::defaults(&defaults))
            .merge(Toml::string(&source))
            .merge(Env::prefixed("BOBE_").split("__"))
            .extract()
            .map_err(|e| crate::error::AppError::Config(e.to_string()))?;

        config.data_dir = crate::util::paths::expand_tilde(&config.data_dir)
            .to_string_lossy()
            .into_owned();
        let config_root = config_path.parent().ok_or_else(|| {
            crate::error::AppError::Config("config file has no parent directory".into())
        })?;
        normalize_optional_path(&mut config.server.tls_cert_path, config_root);
        normalize_optional_path(&mut config.server.tls_key_path, config_root);
        if config.database.url.trim().is_empty() {
            config.database.url = format!("sqlite:{}/data/bobrust.db", config.data_dir);
        }

        config.providers.remember_routes(&config.routes)?;
        config.agent.execution.validate()?;

        let paths = crate::util::paths::DataPaths::resolve(&config.data_dir, &config.database.url)?;
        Ok(LoadedConfig {
            config,
            paths,
            config_file: config_path.to_path_buf(),
        })
    }

    pub(crate) fn checkin_times_vec(&self) -> &[String] {
        &self.checkin.times
    }

    pub(crate) fn mcp_blocked_commands_vec(&self) -> &[String] {
        &self.mcp.blocked_commands
    }

    pub(crate) fn mcp_dangerous_env_keys_vec(&self) -> &[String] {
        &self.mcp.dangerous_env_keys
    }

    pub(crate) fn cors_origins_vec(&self) -> &[String] {
        &self.server.cors_origins
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_active_routes_are_remembered_before_the_first_provider_switch() {
        let root =
            std::env::temp_dir().join(format!("bobe-model-defaults-{}", uuid::Uuid::new_v4()));
        let mut loaded = Config::load_source(
            &root.join("config.toml"),
            &root,
            concat!(
                "[routes.chat]\nprofile='local'\nmodel='previous-local-model'\n",
                "[routes.goals]\nprofile='local'\nmodel='previous-work-model'\n",
                "[routes.consolidate]\nprofile='local'\nmodel='previous-memory-model'\n",
            )
            .into(),
        )
        .unwrap();
        let local = crate::provider::ProfileId::from_static("local");
        assert_eq!(
            loaded.config.providers.profiles[&local]
                .models
                .chat
                .as_deref(),
            Some("previous-local-model")
        );
        let mut routes = loaded.config.routes;
        routes.chat = crate::provider::WorkloadRoute {
            profile: crate::provider::ProfileId::from_static("copilot"),
            model: Some("next-model".into()),
            reasoning_effort: None,
        };
        loaded.config.providers.remember_routes(&routes).unwrap();
        assert_eq!(
            loaded.config.providers.profiles[&local]
                .models
                .chat
                .as_deref(),
            Some("previous-local-model")
        );
    }

    #[test]
    fn unknown_config_is_rejected_without_rewriting_data() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-readonly-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let config_file = root.join("config.toml");
        let source = "[engine]\nengine = 'local'\nprovider_chat_model = 'test'\n";
        std::fs::write(&config_file, source).unwrap();
        assert!(Config::load_from(&config_file, &root).is_err());
        assert_eq!(std::fs::read_to_string(&config_file).unwrap(), source);
        drop(std::fs::remove_dir_all(root));
    }

    #[test]
    fn loading_preserves_valid_settings() {
        let root = std::env::temp_dir().join(format!(
            "bobe-config-authoritative-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_file = root.join("config.toml");
        let source = "# keep\n[voice]\nenabled = true\n";
        std::fs::write(&config_file, source).unwrap();

        let loaded = Config::load_from(&config_file, &root).unwrap();

        assert!(loaded.config.voice.enabled);
        assert_eq!(std::fs::read_to_string(&config_file).unwrap(), source);
        drop(std::fs::remove_dir_all(root));
    }

    #[test]
    fn invalid_settings_are_not_rewritten() {
        let root = std::env::temp_dir().join(format!(
            "bobe-config-invalid-authoritative-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_file = root.join("config.toml");
        let source = "[server]\nport = 'invalid'\n";
        std::fs::write(&config_file, source).unwrap();

        assert!(Config::load_from(&config_file, &root).is_err());
        assert_eq!(std::fs::read_to_string(&config_file).unwrap(), source);
        drop(std::fs::remove_dir_all(root));
    }

    #[test]
    fn persisted_config_rejects_unknown_fields() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-validation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let config_file = root.join("config.toml");
        std::fs::write(
            &config_file,
            "[server]\nprot = 9443\n[capture]\nenabld = true\n",
        )
        .unwrap();
        let Err(error) = Config::load_from(&config_file, &root) else {
            panic!("unknown fields must fail");
        };
        let message = error.to_string();
        assert!(message.contains("server.prot"));
        assert!(message.contains("capture.enabld"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_optional_tls_paths_load_as_unset() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-empty-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let config_file = root.join("config.toml");
        std::fs::write(
            &config_file,
            "[server]\ntls_cert_path = ''\ntls_key_path = ''\n",
        )
        .unwrap();
        let loaded = Config::load_from(&config_file, &root).unwrap();
        assert!(loaded.config.server.tls_cert_path.is_none());
        assert!(loaded.config.server.tls_key_path.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
mod agent;
