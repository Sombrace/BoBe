use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::error::AppError;
use crate::provider::{ProfileId, ProviderModelDescriptor};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListModelsQuery {
    pub(crate) profile: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ListModelsResponse {
    profile: String,
    models: Vec<ProviderModelDescriptor>,
    source: &'static str,
}

pub(crate) async fn list_models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListModelsQuery>,
) -> Result<Json<ListModelsResponse>, AppError> {
    let profile = match query.profile {
        Some(value) => ProfileId::parse(&value)?,
        None => state.config().routes.chat.profile.clone(),
    };
    if !state.config().providers.profiles.contains_key(&profile) {
        return Err(AppError::NotFound(format!(
            "provider profile '{}' was not found",
            profile.as_str(),
        )));
    }
    super::providers::require_live_profile(&state, &profile)?;
    let models = state.auth.agent.list_models(&profile).await?;
    Ok(Json(ListModelsResponse {
        profile: profile.as_str().into(),
        models,
        source: "upstream",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_request_is_profile_scoped() {
        let query: ListModelsQuery = serde_json::from_str(r#"{"profile":"local"}"#).unwrap();
        assert_eq!(query.profile.as_deref(), Some("local"));
        assert!(serde_json::from_str::<ListModelsQuery>(r#"{"engine":"local"}"#).is_err());
    }
}
