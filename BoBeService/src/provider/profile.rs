use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

const SECRET_PREFIX: &str = "bobe-secret://";
const MAX_MODEL_ID_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub(crate) enum ReasoningEffort {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderKind {
    Copilot,
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Deepseek,
    OpenRouter,
    RailRelay,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderAuthMethod {
    #[default]
    ApiKey,
    Oauth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalRuntimeKind {
    Ollama,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ProfileId(String);

impl ProfileId {
    pub(crate) fn parse(value: &str) -> Result<Self, AppError> {
        let valid = !value.is_empty()
            && value.len() <= 32
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
            && value.as_bytes()[0].is_ascii_alphanumeric();
        if !valid {
            return Err(AppError::Config(format!(
                "invalid provider profile id '{value}'; use 1-32 lowercase letters, digits, _ or -"
            )));
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn from_static(value: &'static str) -> Self {
        debug_assert_ne!(value, "");
        Self(value.to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for ProfileId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderProfileConfig {
    #[serde(default)]
    pub(crate) models: ProviderModels,
    pub(crate) kind: ProviderKind,
    #[serde(default)]
    pub(crate) auth_method: ProviderAuthMethod,
    #[serde(default)]
    pub(crate) base_url: Option<String>,
    #[serde(default)]
    pub(crate) credential: Option<String>,
    #[serde(default)]
    pub(crate) organization: Option<String>,
    #[serde(default)]
    pub(crate) offline: bool,
    #[serde(default)]
    pub(crate) local_runtime: Option<LocalRuntimeKind>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ProviderModels {
    pub(crate) chat: Option<String>,
    pub(crate) goals: Option<String>,
    pub(crate) consolidate: Option<String>,
    pub(crate) small: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProvidersConfig {
    #[serde(flatten)]
    pub(crate) profiles: BTreeMap<ProfileId, ProviderProfileConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkloadRoute {
    pub(crate) profile: ProfileId,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<ReasoningEffort>,
}

impl WorkloadRoute {
    fn validate(&self) -> Result<(), AppError> {
        self.model.as_deref().map_or(Ok(()), validate_model_id)
    }
}

pub(crate) fn validate_model_id(model: &str) -> Result<(), AppError> {
    if model.trim().is_empty() || model.len() > MAX_MODEL_ID_BYTES {
        return Err(AppError::Validation(format!(
            "model must be 1-{MAX_MODEL_ID_BYTES} bytes and not blank"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderRoutes {
    pub(crate) chat: WorkloadRoute,
    pub(crate) goals: WorkloadRoute,
    pub(crate) consolidate: WorkloadRoute,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) small: Option<WorkloadRoute>,
}

impl ProviderRoutes {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &WorkloadRoute> {
        [&self.chat, &self.goals, &self.consolidate]
            .into_iter()
            .chain(self.small.iter())
    }
}

impl ProviderProfileConfig {
    pub(crate) fn same_access(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.auth_method == other.auth_method
            && self.base_url == other.base_url
            && self.credential == other.credential
            && self.organization == other.organization
            && self.offline == other.offline
            && self.local_runtime == other.local_runtime
    }

    pub(crate) fn validate_api_key(&self, api_key: &str) -> Result<(), AppError> {
        if self.kind == ProviderKind::Anthropic
            && (api_key.contains("sk-ant-oat") || api_key.contains("sk-ant-ort"))
        {
            return Err(AppError::Validation(
                "configure an Anthropic API key, not a Claude subscription access or refresh token"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate(&self, id: &ProfileId) -> Result<(), AppError> {
        if self.kind == ProviderKind::Anthropic && self.auth_method == ProviderAuthMethod::Oauth {
            return Err(AppError::Validation(
                "Anthropic does not permit Claude subscription login in third-party applications; configure an Anthropic API key".into(),
            ));
        }
        if self.auth_method == ProviderAuthMethod::Oauth
            && !matches!(self.kind, ProviderKind::Copilot | ProviderKind::OpenAi)
        {
            return Err(AppError::Config(format!(
                "provider '{}' does not support subscription login",
                id.as_str()
            )));
        }
        if self.auth_method == ProviderAuthMethod::Oauth && self.credential.is_some() {
            return Err(AppError::Config(
                "subscription login and API-key credentials are separate authentication methods"
                    .into(),
            ));
        }
        if self.auth_method == ProviderAuthMethod::Oauth
            && (self.base_url.is_some() || self.organization.is_some())
        {
            return Err(AppError::Config(
                "subscription login uses the provider's own endpoint and account, not a custom base URL or organization".into(),
            ));
        }
        if self.local_runtime.is_some() && self.kind != ProviderKind::OpenAiCompatible {
            return Err(AppError::Config(
                "local runtimes require an OpenAI-compatible provider profile".into(),
            ));
        }
        if self.kind == ProviderKind::RailRelay && self.offline {
            return Err(AppError::Config(
                "Rail Relay is an external model provider".into(),
            ));
        }
        if self
            .credential
            .as_deref()
            .is_some_and(|value| !value.starts_with(SECRET_PREFIX))
        {
            return Err(AppError::Config(format!(
                "provider '{}' credential must be a {SECRET_PREFIX} reference",
                id.as_str()
            )));
        }
        match self.kind {
            ProviderKind::Copilot => {
                if self.base_url.is_some()
                    || self.credential.is_some()
                    || self.offline
                    || self.local_runtime.is_some()
                    || self.auth_method != ProviderAuthMethod::Oauth
                {
                    return Err(AppError::Config(format!(
                        "Copilot profile '{}' requires OAuth and cannot define base_url, credential, or offline",
                        id.as_str()
                    )));
                }
            }
            ProviderKind::OpenAi | ProviderKind::Deepseek | ProviderKind::OpenRouter => {
                if self.offline || self.local_runtime.is_some() {
                    return Err(AppError::Config(format!(
                        "cloud provider profile '{}' cannot be offline",
                        id.as_str()
                    )));
                }
                validate_base_url(
                    id,
                    self.base_url
                        .as_deref()
                        .unwrap_or("https://api.openai.com/v1"),
                )?;
            }
            ProviderKind::Anthropic => {
                if self.offline || self.local_runtime.is_some() {
                    return Err(AppError::Config(format!(
                        "Anthropic profile '{}' cannot be offline",
                        id.as_str()
                    )));
                }
                validate_base_url(
                    id,
                    self.base_url
                        .as_deref()
                        .unwrap_or("https://api.anthropic.com"),
                )?;
            }
            ProviderKind::OpenAiCompatible | ProviderKind::RailRelay => {
                let url = self.base_url.as_deref().ok_or_else(|| {
                    AppError::Config(format!(
                        "provider profile '{}' requires base_url",
                        id.as_str()
                    ))
                })?;
                validate_base_url(id, url)?;
            }
        }
        Ok(())
    }
}

impl ProvidersConfig {
    pub(crate) fn same_access(&self, other: &Self) -> bool {
        self.profiles.len() == other.profiles.len()
            && self.profiles.iter().all(|(id, profile)| {
                other
                    .profiles
                    .get(id)
                    .is_some_and(|active| profile.same_access(active))
            })
    }

    pub(crate) fn remember_routes(&mut self, routes: &ProviderRoutes) -> Result<(), AppError> {
        self.validate(routes)?;
        for (workload, route) in [
            ("chat", Some(&routes.chat)),
            ("goals", Some(&routes.goals)),
            ("consolidate", Some(&routes.consolidate)),
            ("small", routes.small.as_ref()),
        ] {
            if let Some(route) = route {
                let profile = self
                    .profiles
                    .get_mut(&route.profile)
                    .ok_or_else(|| AppError::Config("model route profile disappeared".into()))?;
                let target = match workload {
                    "chat" => &mut profile.models.chat,
                    "goals" => &mut profile.models.goals,
                    "consolidate" => &mut profile.models.consolidate,
                    _ => &mut profile.models.small,
                };
                target.clone_from(&route.model);
            }
        }
        Ok(())
    }

    pub(crate) fn validate(&self, routes: &ProviderRoutes) -> Result<(), AppError> {
        for (id, profile) in &self.profiles {
            profile.validate(id)?;
        }
        for route in routes.iter() {
            if !self.profiles.contains_key(&route.profile) {
                return Err(AppError::Config(format!(
                    "provider route references missing profile '{}'",
                    route.profile.as_str()
                )));
            }
            route.validate()?;
        }
        Ok(())
    }
}

pub(crate) fn credential_account(profile: &ProfileId) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!("provider-v1-{}-api-key", encoded.encode(profile.as_str()))
}

pub(crate) fn credential_reference(profile: &ProfileId) -> String {
    format!("{SECRET_PREFIX}{}", credential_account(profile))
}

pub(crate) fn resolve_credential(
    profile: &ProfileId,
    reference: Option<&str>,
    store: &dyn crate::secrets::SecretStore,
) -> Result<Option<String>, AppError> {
    let Some(reference) = reference else {
        return Ok(None);
    };
    let account = reference.strip_prefix(SECRET_PREFIX).ok_or_else(|| {
        AppError::Config(format!(
            "provider '{}' has an invalid credential reference",
            profile.as_str()
        ))
    })?;
    store.read(account).map(Some).ok_or_else(|| {
        AppError::Config(format!(
            "provider '{}' credential is missing",
            profile.as_str()
        ))
    })
}

fn validate_base_url(id: &ProfileId, value: &str) -> Result<(), AppError> {
    let url = reqwest::Url::parse(value).map_err(|error| {
        AppError::Config(format!(
            "provider '{}' has invalid base URL: {error}",
            id.as_str()
        ))
    })?;
    let secure = url.scheme() == "https";
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if !matches!(url.scheme(), "http" | "https") || (!secure && !loopback) {
        return Err(AppError::Config(format!(
            "provider '{}' base URL must use HTTPS or loopback HTTP",
            id.as_str()
        )));
    }
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::Config(format!(
            "provider '{}' base URL cannot contain credentials, query, or fragment",
            id.as_str()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_id(value: &str) -> ProfileId {
        ProfileId::parse(value).expect("valid profile id")
    }

    #[test]
    fn remembered_models_do_not_change_provider_access() {
        let config = crate::config::Config::default();
        let original = config.providers;
        let mut changed = original.clone();
        let profile = changed.profiles.values_mut().next().unwrap();
        profile.models.chat = Some("new-chat".into());
        profile.models.goals = Some("new-work-model".into());
        assert!(original.same_access(&changed));
        changed.profiles.values_mut().next().unwrap().base_url =
            Some("https://different.example/v1".into());
        assert!(!original.same_access(&changed));
        assert!(!changed.same_access(&original));
    }

    #[test]
    fn model_choices_survive_switching_an_active_route_to_another_profile() {
        let config = crate::config::Config::default();
        let mut providers = config.providers;
        let mut routes = config.routes;
        routes.chat.model = Some("cloud-chat".into());
        routes.goals.model = Some("cloud-work".into());
        routes.small = Some(WorkloadRoute {
            profile: routes.chat.profile.clone(),
            model: Some("cloud-small".into()),
            reasoning_effort: None,
        });
        providers.remember_routes(&routes).unwrap();
        routes.chat = WorkloadRoute {
            profile: profile_id("local"),
            model: Some("local-chat".into()),
            reasoning_effort: None,
        };
        providers.remember_routes(&routes).unwrap();
        assert_eq!(
            providers.profiles[&profile_id("copilot")]
                .models
                .chat
                .as_deref(),
            Some("cloud-chat")
        );
        assert_eq!(
            providers.profiles[&profile_id("copilot")]
                .models
                .small
                .as_deref(),
            Some("cloud-small")
        );
        assert_eq!(
            providers.profiles[&profile_id("local")]
                .models
                .chat
                .as_deref(),
            Some("local-chat")
        );
    }

    #[test]
    fn provider_secret_accounts_preserve_profile_boundaries() {
        let dash = profile_id("a-b");
        let underscore = profile_id("a_b");
        assert_ne!(credential_account(&dash), credential_account(&underscore));
        assert!(credential_reference(&dash).starts_with(SECRET_PREFIX));
    }

    #[test]
    fn rejects_literal_credentials_and_insecure_remote_urls() {
        let openai = ProviderProfileConfig {
            models: ProviderModels::default(),
            kind: ProviderKind::OpenAi,
            auth_method: ProviderAuthMethod::ApiKey,
            base_url: None,
            credential: Some("sk-secret".into()),
            organization: None,
            offline: false,

            local_runtime: None,
        };
        assert!(openai.validate(&profile_id("openai")).is_err());

        let remote = ProviderProfileConfig {
            models: ProviderModels::default(),
            kind: ProviderKind::OpenAiCompatible,
            auth_method: ProviderAuthMethod::ApiKey,
            base_url: Some("http://example.com/v1".into()),
            credential: None,
            organization: None,
            offline: false,

            local_runtime: None,
        };
        assert!(remote.validate(&profile_id("remote")).is_err());
    }

    #[test]
    fn accepts_openai_default_and_loopback_compatible_profiles() {
        let openai = ProviderProfileConfig {
            models: ProviderModels::default(),
            kind: ProviderKind::OpenAi,
            auth_method: ProviderAuthMethod::ApiKey,
            base_url: None,
            credential: Some("bobe-secret://provider-v1-openai-api-key".into()),
            organization: None,
            offline: false,

            local_runtime: None,
        };
        assert!(openai.validate(&profile_id("openai")).is_ok());

        let local = ProviderProfileConfig {
            models: ProviderModels::default(),
            kind: ProviderKind::OpenAiCompatible,
            auth_method: ProviderAuthMethod::ApiKey,
            base_url: Some("http://127.0.0.1:11434/v1".into()),
            credential: None,
            organization: None,
            offline: true,

            local_runtime: None,
        };
        assert!(local.validate(&profile_id("local")).is_ok());
    }

    #[test]
    fn anthropic_uses_api_keys_not_consumer_subscription_login() {
        let mut profile = ProviderProfileConfig {
            models: ProviderModels::default(),
            kind: ProviderKind::Anthropic,
            auth_method: ProviderAuthMethod::ApiKey,
            base_url: None,
            credential: None,
            organization: None,
            offline: false,
            local_runtime: None,
        };
        assert!(profile.validate(&profile_id("anthropic")).is_ok());
        assert!(profile.validate_api_key("sk-ant-api03-fixture").is_ok());
        for token in ["sk-ant-oat01-fixture", "sk-ant-ort01-fixture"] {
            assert!(profile.validate_api_key(token).is_err());
        }
        profile.auth_method = ProviderAuthMethod::Oauth;
        assert!(
            profile
                .validate(&profile_id("anthropic"))
                .unwrap_err()
                .to_string()
                .contains("Anthropic API key")
        );
    }

    #[test]
    fn routes_must_reference_existing_profiles() {
        let providers = ProvidersConfig::default();
        let route = WorkloadRoute {
            profile: profile_id("missing"),
            model: None,
            reasoning_effort: None,
        };
        let routes = ProviderRoutes {
            small: None,
            chat: route.clone(),
            goals: route.clone(),
            consolidate: route,
        };
        assert!(providers.validate(&routes).is_err());
    }

    #[test]
    fn reasoning_efforts_are_the_worker_thinking_levels() {
        let protocol = include_str!("../../../BoBeAgent/src/protocol.ts");
        let start = protocol
            .find("const thinkingSchema")
            .expect("worker declares its thinking levels");
        let declaration = &protocol[start..];
        let declaration = &declaration[..declaration.find(';').expect("declaration ends")];
        let worker: std::collections::BTreeSet<&str> =
            declaration.split('"').skip(1).step_by(2).collect();
        let rust: std::collections::BTreeSet<String> =
            <ReasoningEffort as clap::ValueEnum>::value_variants()
                .iter()
                .map(|effort| {
                    serde_json::to_value(effort)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect();
        assert_eq!(
            rust,
            worker.into_iter().map(str::to_owned).collect(),
            "Rust reasoning efforts must match the worker's thinking levels"
        );
    }

    #[test]
    fn routes_reject_reasoning_efforts_the_worker_cannot_run() {
        let route = |effort: &str| {
            serde_json::from_value::<WorkloadRoute>(serde_json::json!({
                "profile": "copilot",
                "model": "gpt-test",
                "reasoning_effort": effort,
            }))
        };
        assert_eq!(
            route("xhigh").unwrap().reasoning_effort,
            Some(ReasoningEffort::Xhigh)
        );
        for invalid in ["invented", "", "HIGH", "none"] {
            assert!(route(invalid).is_err(), "{invalid}");
        }
        assert!(
            toml_edit::de::from_str::<WorkloadRoute>(
                "profile = 'copilot'\nreasoning_effort = 'invented'\n"
            )
            .is_err()
        );
    }

    #[test]
    fn route_models_must_be_bounded_and_not_blank_before_they_are_saved() {
        let config = crate::config::Config::default();
        let original = config.providers;
        let mut routes = config.routes;
        for invalid in [
            String::new(),
            "   ".into(),
            "m".repeat(MAX_MODEL_ID_BYTES + 1),
        ] {
            routes.goals.model = Some(invalid.clone());
            let mut providers = original.clone();
            assert!(
                matches!(
                    providers.remember_routes(&routes),
                    Err(AppError::Validation(_))
                ),
                "{invalid:?}"
            );
            assert_eq!(providers, original);
        }
        for valid in [
            "gpt-test".to_owned(),
            "モデル-ä".to_owned(),
            "m".repeat(MAX_MODEL_ID_BYTES),
        ] {
            routes.goals.model = Some(valid);
            assert!(original.clone().remember_routes(&routes).is_ok());
        }
    }
}
