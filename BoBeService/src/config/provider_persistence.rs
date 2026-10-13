use std::path::Path;

use crate::error::AppError;
use crate::provider::{ProviderRoutes, ProvidersConfig};

#[derive(serde::Serialize)]
struct ProviderDocument<'a> {
    providers: &'a ProvidersConfig,
    routes: &'a ProviderRoutes,
}

pub(crate) fn merge_into(
    document: &mut toml_edit::DocumentMut,
    providers: &ProvidersConfig,
    routes: &ProviderRoutes,
) -> Result<(), AppError> {
    let nested = toml_edit::ser::to_document(&ProviderDocument { providers, routes })
        .map_err(|error| AppError::Config(format!("provider config serialization: {error}")))?;
    document["providers"] = nested["providers"].clone();
    document["routes"] = nested["routes"].clone();
    Ok(())
}

pub(crate) fn persist(
    config_root: &Path,
    providers: &ProvidersConfig,
    routes: &ProviderRoutes,
) -> Result<(), AppError> {
    std::fs::create_dir_all(config_root)?;
    let path = config_root.join("config.toml");
    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut document: toml_edit::DocumentMut = existing
        .parse()
        .map_err(|error| AppError::Config(format!("invalid config.toml: {error}")))?;
    merge_into(&mut document, providers, routes)?;
    let content = super::manager_persistence::render_config(&existing, &document);
    crate::util::durable_fs::atomic_write_sync(&path, content.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::provider::{ProfileId, ProviderKind, ProviderProfileConfig, WorkloadRoute};

    #[test]
    fn nested_persistence_preserves_unrelated_config() {
        let root =
            std::env::temp_dir().join(format!("bobe-provider-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("config.toml"),
            "# keep\n[voice]\nenabled = true\n",
        )
        .unwrap();
        let id = ProfileId::from_static("openai");
        let providers = ProvidersConfig {
            profiles: BTreeMap::from([(
                id.clone(),
                ProviderProfileConfig {
                    models: crate::provider::profile::ProviderModels::default(),
                    kind: ProviderKind::OpenAi,
                    auth_method: crate::provider::ProviderAuthMethod::ApiKey,
                    base_url: None,
                    credential: None,
                    organization: None,
                    offline: false,
                    local_runtime: None,
                },
            )]),
        };
        let route = WorkloadRoute {
            profile: id,
            model: Some("gpt-test".into()),
            reasoning_effort: None,
        };
        let routes = ProviderRoutes {
            small: None,
            chat: route.clone(),
            goals: route.clone(),
            consolidate: route,
        };
        persist(&root, &providers, &routes).unwrap();
        let text = std::fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(text.contains("# keep"));
        assert!(text.contains("[voice]"));
        #[derive(serde::Deserialize)]
        struct Persisted {
            providers: ProvidersConfig,
            routes: ProviderRoutes,
        }
        let parsed: Persisted = toml_edit::de::from_str(&text).unwrap();
        assert_eq!(
            parsed
                .providers
                .profiles
                .get(&ProfileId::from_static("openai"))
                .map(|profile| profile.kind),
            Some(ProviderKind::OpenAi)
        );
        assert_eq!(parsed.routes.chat.profile.as_str(), "openai");
        assert!(std::fs::remove_dir_all(root).is_ok());
    }
}
