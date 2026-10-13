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
    let account = crate::provider::credential_account(&id);
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
    let previous_key = state.infra.headless_secret_store.read(&account);
    if let Some(api_key) = new_api_key {
        state
            .infra
            .headless_secret_store
            .store(&account, api_key)
            .map_err(|error| {
                AppError::Config(format!("failed to store provider credential: {error}"))
            })?;
    }
    if let Err(error) = crate::config::provider_persistence::persist(
        state.infra.config_root.as_path(),
        &providers,
        &routes,
    ) {
        if new_api_key.is_some() {
            match previous_key {
                Some(previous) => {
                    drop(state.infra.headless_secret_store.store(&account, &previous));
                }
                None => drop(state.infra.headless_secret_store.delete(&account)),
            }
        }
        return Err(error);
    }
    let changed = providers != config.providers || routes != config.routes;
    let mut updated = (**config).clone();
    updated.providers = providers;
    updated.routes = routes;
    drop(config);
    state.infra.config.store(Arc::new(updated));
    if new_api_key.is_some() {
        state
            .infra
            .provider_restart_pending
            .store(true, std::sync::atomic::Ordering::Release);
    }
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
    let previous_key = profile.credential.as_ref().and_then(|_| {
        state
            .infra
            .headless_secret_store
            .read(&crate::provider::credential_account(&id))
    });
    if profile.credential.is_some() {
        state
            .infra
            .headless_secret_store
            .delete(&crate::provider::credential_account(&id))
            .map_err(|error| {
                AppError::Config(format!("failed to delete provider credential: {error}"))
            })?;
    }
    if let Err(error) = crate::config::provider_persistence::persist(
        state.infra.config_root.as_path(),
        &providers,
        &routes,
    ) {
        if let Some(previous) = previous_key {
            drop(
                state
                    .infra
                    .headless_secret_store
                    .store(&crate::provider::credential_account(&id), &previous),
            );
        }
        return Err(error);
    }
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
    let account = crate::provider::credential_account(&id);
    let previous_key = state.infra.headless_secret_store.read(&account);
    state
        .infra
        .headless_secret_store
        .store(&account, api_key)
        .map_err(|error| {
            AppError::Config(format!("failed to store provider credential: {error}"))
        })?;
    profile.credential = Some(format!("bobe-secret://{account}"));
    if let Err(error) = crate::config::provider_persistence::persist(
        state.infra.config_root.as_path(),
        &providers,
        &routes,
    ) {
        match previous_key {
            Some(previous) => {
                drop(state.infra.headless_secret_store.store(&account, &previous));
            }
            None => drop(state.infra.headless_secret_store.delete(&account)),
        }
        return Err(error);
    }
    let mut updated = (**config).clone();
    updated.providers = providers;
    updated.routes = routes;
    drop(config);
    state.infra.config.store(Arc::new(updated));
    state
        .infra
        .provider_restart_pending
        .store(true, std::sync::atomic::Ordering::Release);
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
    let account = crate::provider::credential_account(&id);
    let previous_key = state.infra.headless_secret_store.read(&account);
    state
        .infra
        .headless_secret_store
        .delete(&account)
        .map_err(|error| {
            AppError::Config(format!("failed to delete provider credential: {error}"))
        })?;
    profile.credential = None;
    if let Err(error) = crate::config::provider_persistence::persist(
        state.infra.config_root.as_path(),
        &providers,
        &routes,
    ) {
        if let Some(previous) = previous_key {
            drop(state.infra.headless_secret_store.store(&account, &previous));
        }
        return Err(error);
    }
    let mut updated = (**config).clone();
    updated.providers = providers;
    updated.routes = routes;
    drop(config);
    state.infra.config.store(Arc::new(updated));
    state
        .infra
        .provider_restart_pending
        .store(true, std::sync::atomic::Ordering::Release);
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
