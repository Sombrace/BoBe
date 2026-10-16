use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::error::AppError;
use crate::provider::{
    ProfileId, ProviderAuthMethod, ProviderKind, ProviderProfileConfig, ProviderRoutes,
};

#[derive(Debug, Serialize)]
pub(crate) struct ProviderProfileResponse {
    models: crate::provider::profile::ProviderModels,
    id: String,
    kind: ProviderKind,
    auth_method: ProviderAuthMethod,
    base_url: Option<String>,
    organization: Option<String>,
    offline: bool,
    local_runtime: Option<crate::provider::LocalRuntimeKind>,
    credential_present: bool,
    restart_required: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProvidersResponse {
    profiles: Vec<ProviderProfileResponse>,
    routes: ProviderRoutes,
    active_profiles: Vec<ProviderProfileResponse>,
    active_routes: ProviderRoutes,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderUpsertRequest {
    kind: ProviderKind,
    auth_method: ProviderAuthMethod,
    base_url: Option<String>,
    organization: Option<String>,
    #[serde(default)]
    offline: bool,
    #[serde(default)]
    local_runtime: Option<crate::provider::LocalRuntimeKind>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderActivationRequest {
    kind: ProviderKind,
    auth_method: ProviderAuthMethod,
    base_url: Option<String>,
    organization: Option<String>,
    #[serde(default)]
    offline: bool,
    #[serde(default)]
    local_runtime: Option<crate::provider::LocalRuntimeKind>,
    api_key: Option<String>,
    chat_model: Option<String>,
    goals_model: Option<String>,
    consolidate_model: Option<String>,
    small_model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCredentialRequest {
    api_key: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProviderMutationResponse {
    updated: bool,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderRoutesPatchRequest {
    chat: Option<crate::provider::WorkloadRoute>,
    goals: Option<crate::provider::WorkloadRoute>,
    consolidate: Option<crate::provider::WorkloadRoute>,
    small: Option<crate::provider::WorkloadRoute>,
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProvidersResponse>, AppError> {
    let config = state.config();
    let routes = config.routes.clone();
    let profiles = config
        .providers
        .profiles
        .iter()
        .map(|(id, profile)| {
            profile_response(
                id,
                profile,
                state.infra.headless_secret_store.as_ref(),
                profile_restart_required(&state, id),
            )
        })
        .collect();
    Ok(Json(ProvidersResponse {
        profiles,
        routes,
        active_profiles: state
            .runtime
            .active_providers
            .profiles
            .iter()
            .map(|(id, profile)| {
                profile_response(
                    id,
                    profile,
                    state.infra.headless_secret_store.as_ref(),
                    false,
                )
            })
            .collect(),
        active_routes: state.runtime.active_routes.clone(),
        restart_required: restart_required(&state),
    }))
}

pub(crate) async fn upsert(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<ProviderUpsertRequest>,
) -> Result<Json<ProviderMutationResponse>, AppError> {
    let id = ProfileId::parse(&id)?;
    let _config_write_guard = lock_config(&state)?;
    let config = state.config();
    let mut providers = config.providers.clone();
    let routes = config.routes.clone();
    let existing_credential = providers
        .profiles
        .get(&id)
        .and_then(|profile| profile.credential.clone());
    let profile = ProviderProfileConfig {
        models: providers
            .profiles
            .get(&id)
            .filter(|profile| {
                profile.kind == request.kind && profile.auth_method == request.auth_method
            })
            .map(|profile| profile.models.clone())
            .unwrap_or_default(),
        kind: request.kind,
        auth_method: request.auth_method,
        base_url: request.base_url,
        credential: if request.auth_method == ProviderAuthMethod::Oauth {
            None
        } else {
            existing_credential
        },
        organization: request.organization,
        offline: request.offline,
        local_runtime: request.local_runtime,
    };
    profile.validate(&id)?;
    if let Some(key) = crate::provider::resolve_credential(
        &id,
        profile.credential.as_deref(),
        state.infra.headless_secret_store.as_ref(),
    )? {
        profile.validate_api_key(&key)?;
    }
    providers.profiles.insert(id, profile);
    providers.validate(&routes)?;
    crate::config::provider_persistence::persist(
        state.infra.config_root.as_path(),
        &providers,
        &routes,
    )?;
    let mut updated = (**config).clone();
    updated.providers = providers;
    updated.routes = routes;
    drop(config);
    state.infra.config.store(Arc::new(updated));
    Ok(Json(ProviderMutationResponse {
        updated: true,
        restart_required: restart_required(&state),
    }))
}

pub(crate) async fn activate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<ProviderActivationRequest>,
) -> Result<Json<ProviderMutationResponse>, AppError> {
    let id = ProfileId::parse(&id)?;
    let _config_write_guard = lock_config(&state)?;
    let config = state.config();
    let mut providers = config.providers.clone();
    let existing_credential = providers
        .profiles
        .get(&id)
        .and_then(|profile| profile.credential.clone());
    let new_api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|api_key| !api_key.is_empty());
    if request.api_key.is_some() && new_api_key.is_none() {
        return Err(AppError::Validation(
            "provider API key must be 1-1024 characters".into(),
        ));
    }
    if new_api_key.is_some_and(|api_key| api_key.len() > 1024) {
        return Err(AppError::Validation(
            "provider API key must be 1-1024 characters".into(),
        ));
    }
    if new_api_key.is_some() && request.auth_method == ProviderAuthMethod::Oauth {
        return Err(AppError::Validation(
            "subscription login does not accept an API-key credential".into(),
        ));
    }
    let credential = if request.auth_method == ProviderAuthMethod::Oauth {
        None
    } else if new_api_key.is_some() {
        Some(crate::provider::credential_reference(&id))
    } else {
        existing_credential
    };
    let profile = ProviderProfileConfig {
        models: providers
            .profiles
            .get(&id)
            .filter(|profile| {
                profile.kind == request.kind && profile.auth_method == request.auth_method
            })
            .map(|profile| profile.models.clone())
            .unwrap_or_default(),
        kind: request.kind,
        auth_method: request.auth_method,
        base_url: request.base_url,
        credential,
        organization: request.organization,
        offline: request.offline,
        local_runtime: request.local_runtime,
    };
    profile.validate(&id)?;
    let checked_key = match new_api_key {
        Some(key) => Some(key.to_owned()),
        None => crate::provider::resolve_credential(
            &id,
            profile.credential.as_deref(),
            state.infra.headless_secret_store.as_ref(),
        )?,
    };
    if let Some(key) = checked_key.as_deref() {
        profile.validate_api_key(key)?;
    }
    let selected = crate::provider::profile::ProviderModels {
        chat: request.chat_model.or(profile.models.chat.clone()),
        goals: request.goals_model.or(profile.models.goals.clone()),
        consolidate: request
            .consolidate_model
            .or(profile.models.consolidate.clone()),
        small: request.small_model.or(profile.models.small.clone()),
    };
    providers.profiles.insert(id.clone(), profile);
    let route = |model: Option<String>| crate::provider::WorkloadRoute {
        profile: id.clone(),
        model,
        reasoning_effort: None,
    };
    let routes = ProviderRoutes {
        small: selected.small.map(|model| route(Some(model))),
        chat: route(selected.chat),
        goals: route(selected.goals),
        consolidate: route(selected.consolidate),
    };
    providers.remember_routes(&routes)?;
    let changed = providers != config.providers || routes != config.routes;
    let change = new_api_key.map_or(CredentialChange::Keep, CredentialChange::Store);
    let committed = commit(&state, &config, &id, change, providers, routes);
    drop(config);
    if committed.secret_changed {
        state
            .infra
            .provider_restart_pending
            .store(true, std::sync::atomic::Ordering::Release);
    }
    committed.result?;
    Ok(Json(ProviderMutationResponse {
        updated: changed || new_api_key.is_some(),
        restart_required: restart_required(&state),
    }))
}

pub(crate) async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProviderMutationResponse>, AppError> {
    let id = ProfileId::parse(&id)?;
    {
        let config = state.config();
        if config
            .routes
            .iter()
            .chain(state.runtime.active_routes.iter())
            .any(|route| route.profile == id)
        {
            return Err(AppError::Conflict(
                "Choose and apply replacement models before removing this provider".into(),
            ));
        }
        let profile = config.providers.profiles.get(&id).ok_or_else(|| {
            AppError::NotFound(format!("provider profile '{}' was not found", id.as_str()))
        })?;
        if profile.auth_method == ProviderAuthMethod::Oauth {
            require_live_profile(&state, &id)?;
            drop(config);
            state.auth.provider_login.cancel(&id).await?;
            state.auth.agent.logout(&id).await?;
        }
    }
    let _config_write_guard = lock_config(&state)?;
    let config = state.config();
    let routes = config.routes.clone();
    if routes
        .iter()
        .chain(state.runtime.active_routes.iter())
        .any(|route| route.profile == id)
    {
        return Err(AppError::Conflict(format!(
            "provider profile '{}' is still assigned to a workload route",
            id.as_str()
        )));
    }
    let mut providers = config.providers.clone();
    let profile = providers.profiles.remove(&id).ok_or_else(|| {
        AppError::NotFound(format!("provider profile '{}' was not found", id.as_str()))
    })?;
    let change = if profile.credential.is_some() {
        CredentialChange::Delete
    } else {
        CredentialChange::Keep
    };
    let committed = commit(&state, &config, &id, change, providers, routes);
    drop(config);
    committed.result?;
    Ok(Json(ProviderMutationResponse {
        updated: true,
        restart_required: restart_required(&state),
    }))
}

pub(crate) async fn set_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<ProviderCredentialRequest>,
) -> Result<Json<ProviderMutationResponse>, AppError> {
    let id = ProfileId::parse(&id)?;
    let api_key = request.api_key.trim();
    if api_key.is_empty() || api_key.len() > 1024 {
        return Err(AppError::Validation(
            "provider API key must be 1-1024 characters".into(),
        ));
    }
    let _config_write_guard = lock_config(&state)?;
    let config = state.config();
    let mut providers = config.providers.clone();
    let routes = config.routes.clone();
    let profile = providers.profiles.get_mut(&id).ok_or_else(|| {
        AppError::NotFound(format!("provider profile '{}' was not found", id.as_str()))
    })?;
    if profile.auth_method == ProviderAuthMethod::Oauth {
        return Err(AppError::Validation(
            "this profile uses subscription login, not an API key".into(),
        ));
    }
    profile.validate_api_key(api_key)?;
    profile.credential = Some(crate::provider::credential_reference(&id));
    let committed = commit(
        &state,
        &config,
        &id,
        CredentialChange::Store(api_key),
        providers,
        routes,
    );
    drop(config);
    if committed.secret_changed {
        state
            .infra
            .provider_restart_pending
            .store(true, std::sync::atomic::Ordering::Release);
    }
    committed.result?;
    Ok(Json(ProviderMutationResponse {
        updated: true,
        restart_required: restart_required(&state),
    }))
}

pub(crate) async fn clear_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProviderMutationResponse>, AppError> {
    let id = ProfileId::parse(&id)?;
    let _config_write_guard = lock_config(&state)?;
    let config = state.config();
    let mut providers = config.providers.clone();
    let routes = config.routes.clone();
    let profile = providers.profiles.get_mut(&id).ok_or_else(|| {
        AppError::NotFound(format!("provider profile '{}' was not found", id.as_str()))
    })?;
    if profile.auth_method == ProviderAuthMethod::Oauth {
        return Err(AppError::Validation(
            "this profile uses subscription login, not an API key".into(),
        ));
    }
    profile.credential = None;
    let committed = commit(
        &state,
        &config,
        &id,
        CredentialChange::Delete,
        providers,
        routes,
    );
    drop(config);
    if committed.secret_changed {
        state
            .infra
            .provider_restart_pending
            .store(true, std::sync::atomic::Ordering::Release);
    }
    committed.result?;
    Ok(Json(ProviderMutationResponse {
        updated: true,
        restart_required: restart_required(&state),
    }))
}

pub(crate) async fn update_routes(
    State(state): State<Arc<AppState>>,
    Json(routes): Json<ProviderRoutes>,
) -> Result<Json<ProviderMutationResponse>, AppError> {
    let _config_write_guard = lock_config(&state)?;
    let config = state.config();
    let mut providers = config.providers.clone();
    providers.remember_routes(&routes)?;
    crate::config::provider_persistence::persist(
        state.infra.config_root.as_path(),
        &providers,
        &routes,
    )?;
    let mut updated = (**config).clone();
    updated.providers = providers;
    updated.routes = routes;
    drop(config);
    state.infra.config.store(Arc::new(updated));
    Ok(Json(ProviderMutationResponse {
        updated: true,
        restart_required: restart_required(&state),
    }))
}

pub(crate) async fn patch_routes(
    State(state): State<Arc<AppState>>,
    Json(patch): Json<ProviderRoutesPatchRequest>,
) -> Result<Json<ProviderMutationResponse>, AppError> {
    if patch.chat.is_none()
        && patch.goals.is_none()
        && patch.consolidate.is_none()
        && patch.small.is_none()
    {
        return Err(AppError::Validation(
            "provider route patch must contain at least one workload".into(),
        ));
    }
    let _config_write_guard = lock_config(&state)?;
    let config = state.config();
    let mut routes = config.routes.clone();
    if let Some(route) = patch.chat {
        routes.chat = route;
    }
    if let Some(route) = patch.goals {
        routes.goals = route;
    }
    if let Some(route) = patch.consolidate {
        routes.consolidate = route;
    }
    if let Some(route) = patch.small {
        routes.small = Some(route);
    }
    let mut providers = config.providers.clone();
    providers.remember_routes(&routes)?;
    let changed = routes != config.routes;
    crate::config::provider_persistence::persist(
        state.infra.config_root.as_path(),
        &providers,
        &routes,
    )?;
    let mut updated = (**config).clone();
    updated.providers = providers;
    updated.routes = routes;
    drop(config);
    state.infra.config.store(Arc::new(updated));
    Ok(Json(ProviderMutationResponse {
        updated: changed,
        restart_required: restart_required(&state),
    }))
}

#[derive(Clone, Copy)]
enum CredentialChange<'a> {
    Keep,
    Store(&'a str),
    Delete,
}

struct Committed {
    result: Result<(), AppError>,
    secret_changed: bool,
}

struct CommitFailure {
    error: AppError,
    config_committed: bool,
}

fn commit(
    state: &AppState,
    config: &crate::config::Config,
    id: &ProfileId,
    change: CredentialChange<'_>,
    providers: crate::provider::ProvidersConfig,
    routes: ProviderRoutes,
) -> Committed {
    let secrets = state.infra.headless_secret_store.as_ref();
    let account = crate::provider::credential_account(id);
    let previous = secrets.read(&account);
    let root = state.infra.config_root.as_path();
    let outcome = commit_provider_change(
        secrets,
        &account,
        change,
        || crate::config::provider_persistence::persist(root, &providers, &routes),
        || crate::config::provider_persistence::persist(root, &config.providers, &config.routes),
    );
    let secret_changed = secrets.read(&account) != previous;
    let config_committed = match &outcome {
        Ok(()) => true,
        Err(failure) => failure.config_committed,
    };
    if config_committed {
        let mut updated = config.clone();
        updated.providers = providers;
        updated.routes = routes;
        state.infra.config.store(Arc::new(updated));
    }
    Committed {
        result: outcome.map_err(|failure| failure.error),
        secret_changed,
    }
}

fn commit_provider_change(
    secrets: &dyn crate::secrets::SecretStore,
    account: &str,
    change: CredentialChange<'_>,
    save_next: impl FnOnce() -> Result<(), AppError>,
    restore_previous: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), CommitFailure> {
    let previous = secrets.read(account);
    let uncommitted = |error| CommitFailure {
        error,
        config_committed: false,
    };
    match change {
        CredentialChange::Keep => save_next().map_err(uncommitted),
        CredentialChange::Store(api_key) => secrets
            .store(account, api_key)
            .map_err(|error| credential_error("store", &error))
            .and_then(|()| save_next())
            .map_err(|error| {
                uncommitted(with_rollback(
                    error,
                    restore_secret(secrets, account, previous.as_deref()),
                ))
            }),
        CredentialChange::Delete => {
            save_next().map_err(uncommitted)?;
            let Err(error) = secrets.delete(account) else {
                return Ok(());
            };
            let error = credential_error("delete", &error);
            if let Err(restore) = restore_secret(secrets, account, previous.as_deref()) {
                return Err(CommitFailure {
                    error: with_rollback(error, Err(restore)),
                    config_committed: true,
                });
            }
            let restored = restore_previous();
            Err(CommitFailure {
                config_committed: restored.is_err(),
                error: with_rollback(error, restored),
            })
        }
    }
}

fn restore_secret(
    secrets: &dyn crate::secrets::SecretStore,
    account: &str,
    previous: Option<&str>,
) -> Result<(), crate::secrets::SecretError> {
    if secrets.read(account).as_deref() == previous {
        return Ok(());
    }
    match previous {
        Some(value) => secrets.store(account, value),
        None => secrets.delete(account),
    }
}

fn credential_error(action: &str, error: &crate::secrets::SecretError) -> AppError {
    AppError::Config(format!("failed to {action} provider credential: {error}"))
}

fn with_rollback(error: AppError, rollback: Result<(), impl std::fmt::Display>) -> AppError {
    match rollback {
        Ok(()) => error,
        Err(rollback) => AppError::Internal(format!(
            "{error}; restoring the previous provider state also failed: {rollback}"
        )),
    }
}

fn lock_config(state: &AppState) -> Result<std::sync::MutexGuard<'_, ()>, AppError> {
    state
        .infra
        .config_write_lock
        .lock()
        .map_err(|_| AppError::Io(std::io::Error::other("config write lock poisoned")))
}

pub(crate) fn restart_required(state: &AppState) -> bool {
    use std::sync::atomic::Ordering;
    let desired = state.config();
    state.infra.provider_restart_pending.load(Ordering::Acquire)
        || desired.routes != state.runtime.active_routes
        || !desired
            .providers
            .same_access(&state.runtime.active_providers)
}

fn profile_restart_required(state: &AppState, id: &ProfileId) -> bool {
    state
        .infra
        .provider_restart_pending
        .load(std::sync::atomic::Ordering::Acquire)
        || match (
            state.config().providers.profiles.get(id),
            state.runtime.active_providers.profiles.get(id),
        ) {
            (Some(saved), Some(active)) => !saved.same_access(active),
            _ => true,
        }
}

pub(super) fn require_live_profile(state: &AppState, id: &ProfileId) -> Result<(), AppError> {
    if profile_restart_required(state, id) {
        return Err(AppError::Conflict(
            "Restart the BoBe host to apply this provider's access changes".into(),
        ));
    }
    Ok(())
}

fn profile_response(
    id: &ProfileId,
    profile: &ProviderProfileConfig,
    secrets: &dyn crate::secrets::SecretStore,
    restart_required: bool,
) -> ProviderProfileResponse {
    let credential_present = profile
        .credential
        .as_deref()
        .and_then(|reference| reference.strip_prefix("bobe-secret://"))
        .is_some_and(|account| secrets.read(account).is_some());
    ProviderProfileResponse {
        models: profile.models.clone(),
        id: id.as_str().to_owned(),
        kind: profile.kind,
        auth_method: profile.auth_method,
        base_url: profile.base_url.clone(),
        organization: profile.organization.clone(),
        offline: profile.offline,
        local_runtime: profile.local_runtime,
        credential_present,
        restart_required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretStore;
    use crate::secrets::memory::MemorySecretStore;

    fn disk_full() -> Result<(), AppError> {
        Err(AppError::Io(std::io::Error::other("disk full")))
    }

    #[test]
    fn a_new_key_is_withdrawn_when_the_provider_configuration_cannot_be_saved() {
        let root =
            std::env::temp_dir().join(format!("bobe-provider-commit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("config.toml")).unwrap();
        let secrets_path = root.join("headless-secrets.json");
        let secrets = crate::secrets::file::FileSecretStore::at_path(secrets_path.clone()).unwrap();
        secrets.store("acct", "old-key").unwrap();
        let config = crate::config::Config::default();

        let failure = commit_provider_change(
            &secrets,
            "acct",
            CredentialChange::Store("new-key"),
            || {
                crate::config::provider_persistence::persist(
                    &root,
                    &config.providers,
                    &config.routes,
                )
            },
            || panic!("the previous configuration was never replaced"),
        )
        .unwrap_err();

        assert!(!failure.config_committed);
        assert!(matches!(failure.error, AppError::Io(_)));
        assert_eq!(secrets.read("acct").as_deref(), Some("old-key"));
        let reopened = crate::secrets::file::FileSecretStore::at_path(secrets_path).unwrap();
        assert_eq!(reopened.read("acct").as_deref(), Some("old-key"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_key_rollback_is_reported_with_the_configuration_failure() {
        let secrets = MemorySecretStore::holding([("acct", "old-key")]).failing_after(1);

        let failure = commit_provider_change(
            &secrets,
            "acct",
            CredentialChange::Store("new-key"),
            disk_full,
            || panic!("the previous configuration was never replaced"),
        )
        .unwrap_err();

        assert!(!failure.config_committed);
        let message = failure.error.to_string();
        assert!(message.contains("disk full"), "{message}");
        assert!(
            message.contains("injected secret store failure"),
            "{message}"
        );
        assert_eq!(secrets.read("acct").as_deref(), Some("new-key"));
    }

    #[test]
    fn a_key_is_deleted_only_after_the_configuration_stops_using_it() {
        let secrets = MemorySecretStore::holding([("acct", "old-key")]);
        let failure = commit_provider_change(
            &secrets,
            "acct",
            CredentialChange::Delete,
            disk_full,
            || panic!("nothing was saved"),
        )
        .unwrap_err();
        assert!(!failure.config_committed);
        assert_eq!(secrets.read("acct").as_deref(), Some("old-key"));

        let restored = std::cell::Cell::new(false);
        let failing = MemorySecretStore::holding([("acct", "old-key")]).failing_after(0);
        let failure = commit_provider_change(
            &failing,
            "acct",
            CredentialChange::Delete,
            || Ok(()),
            || {
                restored.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(restored.get());
        assert!(!failure.config_committed);
        assert_eq!(failing.read("acct").as_deref(), Some("old-key"));

        let failure = commit_provider_change(
            &MemorySecretStore::holding([("acct", "old-key")]).failing_after(0),
            "acct",
            CredentialChange::Delete,
            || Ok(()),
            disk_full,
        )
        .unwrap_err();
        assert!(failure.config_committed);
        let message = failure.error.to_string();
        assert!(
            message.contains("injected secret store failure"),
            "{message}"
        );
        assert!(message.contains("disk full"), "{message}");

        assert!(
            commit_provider_change(
                &secrets,
                "acct",
                CredentialChange::Delete,
                || Ok(()),
                || panic!("nothing failed"),
            )
            .is_ok()
        );
        assert_eq!(secrets.read("acct"), None);
    }
}
