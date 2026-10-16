use std::sync::Arc;

use crate::app_state::AppState;
use crate::error::AppError;

pub(crate) async fn create_live_backup(state: &AppState) -> Result<std::path::PathBuf, AppError> {
    let _turn = state
        .runtime
        .runtime_session
        .try_begin_personal_data_access()
        .map_err(|message| AppError::Conflict(message.into()))?;
    let _personal_data = Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_write_owned()
        .map_err(|_| AppError::Conflict("Cannot back up while background work is active".into()))?;
    let _maintenance = state
        .runtime
        .agent_authority_lifecycle
        .try_begin_maintenance()?;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state.auth.provider_login.cancel_and_wait(),
    )
    .await
    .map_err(|_| AppError::Internal("Provider login did not stop before backup".into()))??;
    state.runtime.workers.quiesce_for_snapshot().await?;
    let identity = &state.infra.identity;
    crate::ops::backup::create_live(
        &state.infra.db,
        &state.infra.paths,
        &state.infra.config_root.join("config.toml"),
        &crate::ops::backup::SettingsLocks {
            config: Arc::clone(&state.infra.config_write_lock),
            mcp: Arc::clone(&state.infra.mcp_config_lock),
        },
        identity.runtime_authority_id,
        identity.runtime_authority_epoch,
        identity.schema_version,
    )
    .await
}
