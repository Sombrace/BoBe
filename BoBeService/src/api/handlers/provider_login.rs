use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};
use tokio_stream::wrappers::WatchStream;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::provider::{ProfileId, ProviderAuthMethod};

pub(crate) async fn start_login(
    State(state): State<Arc<AppState>>,
    Path(profile): Path<String>,
) -> Result<StatusCode, AppError> {
    let profile = configured_profile(&state, &profile)?;
    super::providers::require_live_profile(&state, &profile)?;
    if state.config().providers.profiles[&profile].auth_method != ProviderAuthMethod::Oauth {
        return Err(AppError::Validation(
            "this provider profile uses an API token, not subscription login".into(),
        ));
    }
    state.auth.provider_login.start(profile).await?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn status(
    State(state): State<Arc<AppState>>,
    Path(profile): Path<String>,
) -> Result<Json<crate::agent::auth::LoginPhase>, AppError> {
    let profile = configured_profile(&state, &profile)?;
    let receiver = state
        .auth
        .provider_login
        .subscribe(&profile)
        .await
        .ok_or_else(|| AppError::NotFound("no provider login is active".into()))?;
    let phase = receiver.borrow().clone();
    Ok(Json(phase))
}

pub(crate) async fn cancel_login(
    State(state): State<Arc<AppState>>,
    Path(profile): Path<String>,
) -> Result<StatusCode, AppError> {
    let profile = configured_profile(&state, &profile)?;
    state.auth.provider_login.cancel(&profile).await?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginInput {
    value: String,
}

pub(crate) async fn submit(
    State(state): State<Arc<AppState>>,
    Path(profile): Path<String>,
    Json(input): Json<LoginInput>,
) -> Result<StatusCode, AppError> {
    let profile = configured_profile(&state, &profile)?;
    state
        .auth
        .provider_login
        .submit(&profile, input.value)
        .await?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn auth_status(
    State(state): State<Arc<AppState>>,
    Path(profile): Path<String>,
) -> Result<Json<crate::provider::ProviderAuthStatus>, AppError> {
    let profile = configured_profile(&state, &profile)?;
    super::providers::require_live_profile(&state, &profile)?;
    Ok(Json(state.auth.agent.auth_status(&profile).await?))
}

pub(crate) async fn events(
    State(state): State<Arc<AppState>>,
    Path(profile): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let profile = configured_profile(&state, &profile)?;
    let Some(rx) = state.auth.provider_login.subscribe(&profile).await else {
        return Err(AppError::NotFound("no login session in flight".into()));
    };
    let stream = WatchStream::new(rx).map(|phase| {
        let payload = match serde_json::to_string(&phase) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::error!(%error, "provider.login_event_encode_failed");
                r#"{"phase":"failed","message":"Could not encode provider login state"}"#.into()
            }
        };
        Ok::<_, Infallible>(Event::default().event("login").data(payload))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

fn configured_profile(state: &AppState, value: &str) -> Result<ProfileId, AppError> {
    let profile = ProfileId::parse(value)?;
    if !state.config().providers.profiles.contains_key(&profile) {
        return Err(AppError::NotFound(format!(
            "provider profile '{value}' was not found"
        )));
    }
    Ok(profile)
}

pub(crate) async fn logout(
    State(state): State<Arc<AppState>>,
    Path(profile): Path<String>,
) -> Result<StatusCode, AppError> {
    let profile = configured_profile(&state, &profile)?;
    super::providers::require_live_profile(&state, &profile)?;
    if state.config().providers.profiles[&profile].auth_method != ProviderAuthMethod::Oauth {
        return Err(AppError::Validation(
            "Remove the saved API key to disconnect this provider".into(),
        ));
    }
    state.auth.provider_login.cancel(&profile).await?;
    state.auth.agent.logout(&profile).await?;
    Ok(StatusCode::NO_CONTENT)
}
