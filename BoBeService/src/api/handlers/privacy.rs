use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::mcp::config::{self as mcp_config, McpConfigFile};

const RETAINED_OPERATIONAL_DATA: &[&str] = &[
    "settings and provider credentials",
    "installed models and runtimes",
    "owner-created backups",
    "content-free request replay guards",
];

#[derive(Serialize)]
pub(crate) struct PrivacyPurgeResponse {
    message: &'static str,
    deleted: Deleted,
    retained: Vec<&'static str>,
    skill_runtime_reloaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reload_warning: Option<String>,
}

#[derive(Serialize)]
struct Deleted {
    conversations: u64,
    turns: u64,
    message_requests: u64,
    custom_souls: u64,
    custom_profiles: u64,
    goals: usize,
    learned_skills: usize,
    workspace_files: usize,
    mcp_secrets: usize,
}

pub(crate) async fn purge(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PrivacyPurgeResponse>, AppError> {
    tokio::time::timeout(
        crate::constants::privacy::PURGE_DEADLINE,
        purge_inner(state),
    )
    .await
    .map_err(|_| {
        AppError::Internal(
            "Privacy purge timed out; some data may already be deleted. Retry to finish.".into(),
        )
    })?
}

async fn purge_inner(state: Arc<AppState>) -> Result<Json<PrivacyPurgeResponse>, AppError> {
    let _maintenance = state
        .runtime
        .agent_authority_lifecycle
        .try_begin_maintenance()?;
    let _purge_guard = state
        .runtime
        .runtime_session
        .try_begin_privacy_purge()
        .map_err(|message| AppError::Conflict(message.into()))?;
    let _personal_data = Arc::clone(&state.runtime.personal_data_lifecycle)
        .try_write_owned()
        .map_err(|_| {
            AppError::Conflict("Cannot purge data while background work is active".into())
        })?;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state.auth.provider_login.cancel_and_wait(),
    )
    .await
    .map_err(|_| AppError::Internal("Provider login did not stop before privacy purge".into()))??;
    state.runtime.workers.quiesce_for_snapshot().await?;

    state
        .runtime
        .memory_file
        .replace_all(crate::provider::memory_file::DEFAULT_BODY.into())
        .await?;
    state.body.gateway.clear_sensor_samples();
    let goals = state.services.goals_service.delete_all().await?;
    let skills = state.runtime.skill_store.purge_personal_state().await?;
    let workspace_files = state.runtime.agent_workspace.purge_files().await?;
    let skill_reload = state.runtime.workers.reload_skills().await;
    if let Err(error) = &skill_reload {
        tracing::warn!(%error, "privacy purge reset skills but runtime reload failed");
    }

    let mut transaction = state.infra.db.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("INSERT OR IGNORE INTO agent_work_tombstones (source_key) SELECT source_key FROM agent_jobs UNION ALL SELECT 'schedule:'||source_key FROM agent_schedules")
        .execute(&mut *transaction).await?;
    sqlx::query("DELETE FROM agent_schedules")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM runtime_cadences")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO agent_work_tombstones (source_key) SELECT 'input:'||source_key FROM owner_inputs").execute(&mut *transaction).await?;
    sqlx::query("DELETE FROM owner_inputs")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM agent_jobs")
        .execute(&mut *transaction)
        .await?;
    let turns = sqlx::query("DELETE FROM conversation_turns")
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    sqlx::query(
        "INSERT OR IGNORE INTO message_request_tombstones (request_id) \
         SELECT request_id FROM message_requests",
    )
    .execute(&mut *transaction)
    .await?;
    let message_requests = sqlx::query("DELETE FROM message_requests")
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    let conversations = sqlx::query("DELETE FROM conversations")
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM cooldown_state")
        .execute(&mut *transaction)
        .await?;
    let custom_souls = crate::db::SqliteSoulRepo::reset_to_shipped(&mut transaction).await?;
    let custom_profiles =
        crate::db::SqliteUserProfileRepo::reset_to_shipped(&mut transaction).await?;
    let _registry_guard = state.body.registry_lock.lock().await;
    let endpoints = state.body.endpoints.list().await?;
    sqlx::query("DELETE FROM endpoints")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    state.body.identities.refresh(&state.body.trust).await?;
    for endpoint in endpoints {
        state.disconnect_endpoint(&endpoint.id).await;
    }

    let data_root = std::path::PathBuf::from(&state.config().data_dir);
    crate::util::durable_fs::durable_remove_dir_all(&data_root.join("tool-output")).await?;
    state.runtime.workers.purge_sessions().await?;
    let mcp_secrets = purge_mcp(&state).await?;

    let direct_cloud_providers = state
        .config()
        .providers
        .profiles
        .values()
        .filter_map(|profile| match profile.kind {
            crate::provider::ProviderKind::OpenAi => Some("OpenAI"),
            crate::provider::ProviderKind::Anthropic => Some("Anthropic"),
            crate::provider::ProviderKind::Copilot
            | crate::provider::ProviderKind::OpenAiCompatible
            | crate::provider::ProviderKind::Deepseek
            | crate::provider::ProviderKind::OpenRouter
            | crate::provider::ProviderKind::RailRelay => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut retained = RETAINED_OPERATIONAL_DATA.to_vec();
    retained.push("provider subscription logins");
    if direct_cloud_providers.contains("OpenAI") {
        retained
            .push("OpenAI request logs and abuse-monitoring retention are controlled by OpenAI");
    }
    if direct_cloud_providers.contains("Anthropic") {
        retained.push("Anthropic API retention and abuse-monitoring are controlled by Anthropic");
    }

    Ok(Json(PrivacyPurgeResponse {
        message: "Personal data deleted; operational data retained",
        deleted: Deleted {
            conversations,
            turns,
            message_requests,
            custom_souls,
            custom_profiles,
            goals,
            learned_skills: skills,
            workspace_files,
            mcp_secrets,
        },
        retained,
        skill_runtime_reloaded: skill_reload.is_ok(),
        reload_warning: skill_reload.err().map(|error| {
            format!(
                "personal skills were reset, but the active runtime did not reload them before its sessions were purged: {error}"
            )
        }),
    }))
}

async fn purge_mcp(state: &AppState) -> Result<usize, AppError> {
    let _guard = state.infra.mcp_config_lock.lock().await;
    let config = state.config();
    let path = mcp_config::resolve_mcp_config_path(
        std::path::Path::new(&config.data_dir),
        config.mcp.config_file.as_deref(),
    )?;
    let previous = mcp_config::load_mcp_config_file(&path).unwrap_or_else(|_| McpConfigFile {
        mcp_servers: std::collections::HashMap::new(),
    });
    let mut accounts = HashSet::new();
    for (server, entry) in &previous.mcp_servers {
        for (key, value) in &entry.env {
            if let Some(account) = value.strip_prefix(mcp_config::SECRET_REF_PREFIX) {
                accounts.insert(account.to_owned());
            }

            accounts.insert(mcp_config::secret_account(server, key));
        }
    }
    for account in &accounts {
        state.infra.secret_store.delete(account).map_err(|error| {
            AppError::Config(format!("Failed to delete MCP secret '{account}': {error}"))
        })?;
    }
    mcp_config::save_mcp_config_file(
        &path,
        &McpConfigFile {
            mcp_servers: std::collections::HashMap::new(),
        },
    )?;
    state
        .runtime
        .workers
        .apply_mcp_config(crate::provider::McpRuntimeConfig::default())
        .await?;
    Ok(accounts.len())
}

#[cfg(test)]
mod tests {
    #[test]
    fn agent_maintenance_is_single_flight_and_releases_on_drop() {
        use std::sync::atomic::Ordering;

        let lifecycle = crate::provider::AgentAuthorityLifecycle::default();
        let flag = lifecycle.maintenance_flag();
        let guard = lifecycle.try_begin_maintenance().unwrap();
        assert!(flag.load(Ordering::Acquire));
        assert!(lifecycle.try_begin_maintenance().is_err());
        drop(guard);
        assert!(!flag.load(Ordering::Acquire));
    }
}
