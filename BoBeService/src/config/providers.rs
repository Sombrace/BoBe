use std::collections::BTreeMap;

use crate::constants::DEFAULT_OLLAMA_V1_URL;
use crate::provider::{
    LocalRuntimeKind, ProfileId, ProviderAuthMethod, ProviderKind, ProviderProfileConfig,
    ProviderRoutes, ProvidersConfig, WorkloadRoute,
};

pub(super) fn default_profiles_and_routes() -> (ProvidersConfig, ProviderRoutes) {
    let copilot = ProfileId::from_static("copilot");
    let profiles = BTreeMap::from([
        (
            copilot.clone(),
            ProviderProfileConfig {
                models: crate::provider::profile::ProviderModels::default(),
                kind: ProviderKind::Copilot,
                auth_method: ProviderAuthMethod::Oauth,
                base_url: None,
                credential: None,
                organization: None,
                offline: false,
                local_runtime: None,
            },
        ),
        (
            ProfileId::from_static("local"),
            ProviderProfileConfig {
                models: crate::provider::profile::ProviderModels::default(),
                kind: ProviderKind::OpenAiCompatible,
                auth_method: ProviderAuthMethod::ApiKey,
                base_url: Some(DEFAULT_OLLAMA_V1_URL.into()),
                credential: None,
                organization: None,
                offline: true,
                local_runtime: Some(LocalRuntimeKind::Ollama),
            },
        ),
    ]);
    let route = WorkloadRoute {
        profile: copilot,
        model: None,
        reasoning_effort: None,
    };
    (
        ProvidersConfig { profiles },
        ProviderRoutes {
            small: None,
            chat: route.clone(),
            goals: route.clone(),
            consolidate: route,
        },
    )
}
