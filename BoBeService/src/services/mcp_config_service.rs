use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

use crate::config::Config;
use crate::error::AppError;
use crate::mcp::config::{self as mcp_config, McpConfigFile, McpServerEntry, timeout_millis};
use crate::mcp::security::{validate_mcp_command_with_args, validate_mcp_env};
use crate::provider::AgentRuntime;
use crate::secrets::SecretStore;

pub(crate) struct McpConfigDeps<'a> {
    pub(crate) config: &'a ArcSwap<Config>,
    pub(crate) mcp_config_lock: &'a Mutex<()>,
    pub(crate) workers: &'a Arc<dyn AgentRuntime>,
    pub(crate) secret_store: &'a Arc<dyn SecretStore>,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpToolMetadata {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) excluded: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpServerSummary {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) enabled: bool,
    pub(crate) connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    pub(crate) tool_count: usize,
    pub(crate) tools: Vec<McpToolMetadata>,
    pub(crate) excluded_tools: Vec<String>,
    pub(crate) env_keys: Vec<String>,
    pub(crate) secret_env_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpConfigDocumentResponse {
    pub(crate) raw_json: String,
    pub(crate) servers: Vec<McpServerSummary>,
    pub(crate) count: usize,
    pub(crate) connected_count: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpConfigMutationRequest {
    pub(crate) raw_json: String,
    #[serde(default)]
    pub(crate) secret_keys: HashMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpConfigValidateResponse {
    pub(crate) valid: bool,
    pub(crate) normalized_json: String,
    pub(crate) server_count: usize,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpConfigSaveResponse {
    pub(crate) message: String,
    pub(crate) raw_json: String,
    pub(crate) servers: Vec<McpServerSummary>,
    pub(crate) count: usize,
    pub(crate) connected_count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpConfigResetResponse {
    pub(crate) message: String,
    pub(crate) raw_json: String,
    pub(crate) count: usize,
}

pub(crate) async fn get_document(
    deps: &McpConfigDeps<'_>,
) -> Result<McpConfigDocumentResponse, AppError> {
    let (_path, file) = load_mcp_file(deps)?;
    let raw_json = redacted_json(&file)?;

    let servers = build_runtime_summaries(deps.workers, &file).await?;
    let count = servers.len();
    let connected_count = servers.iter().filter(|s| s.connected).count();

    Ok(McpConfigDocumentResponse {
        raw_json,
        servers,
        count,
        connected_count,
    })
}

pub(crate) fn validate_document(
    deps: &McpConfigDeps<'_>,
    body: &McpConfigMutationRequest,
) -> Result<McpConfigValidateResponse, AppError> {
    let (blocked_cmds, dangerous_keys) = blocked_and_dangerous(deps);

    let validation = parse_and_validate(&body.raw_json, &blocked_cmds, &dangerous_keys)
        .and_then(|file| normalize_secrets(file, &body.secret_keys, None));
    match validation {
        Ok(normalized) => Ok(McpConfigValidateResponse {
            valid: true,
            normalized_json: redacted_json(&normalized)?,
            server_count: normalized.mcp_servers.len(),
            errors: Vec::new(),
        }),
        Err(error) => invalid_document_response(error),
    }
}

fn invalid_document_response(error: AppError) -> Result<McpConfigValidateResponse, AppError> {
    match error {
        AppError::Validation(message) | AppError::Mcp(message) => Ok(McpConfigValidateResponse {
            valid: false,
            normalized_json: String::new(),
            server_count: 0,
            errors: vec![message],
        }),
        error => Err(error),
    }
}

pub(crate) async fn save_document(
    deps: &McpConfigDeps<'_>,
    body: McpConfigMutationRequest,
) -> Result<McpConfigSaveResponse, AppError> {
    let (blocked_cmds, dangerous_keys) = blocked_and_dangerous(deps);
    let file = parse_and_validate(&body.raw_json, &blocked_cmds, &dangerous_keys)?;

    let guard = deps.mcp_config_lock.lock().await;
    let file = normalize_secrets(file, &body.secret_keys, Some(deps.secret_store.as_ref()))?;
    let path = resolve_config_path(deps)?;
    let previous = mcp_config::load_mcp_config_file(&path).ok();

    mcp_config::save_mcp_config_file(&path, &file)?;

    if let Some(ref prev) = previous {
        cleanup_removed_secret_refs(deps.secret_store.as_ref(), prev, &file);
    }

    let runtime = if deps.config.load().mcp.enabled {
        let parsed =
            mcp_config::parse_enabled_servers(file.clone(), &blocked_cmds, &dangerous_keys)?;
        mcp_config::to_runtime_mcp_config(parsed)
    } else {
        crate::provider::McpRuntimeConfig::default()
    };
    deps.workers.apply_mcp_config(runtime).await?;
    drop(guard);

    let servers = build_runtime_summaries(deps.workers, &file).await?;
    let count = servers.len();
    let connected_count = servers.iter().filter(|s| s.connected).count();

    Ok(McpConfigSaveResponse {
        message: "MCP config saved".into(),
        raw_json: redacted_json(&file)?,
        servers,
        count,
        connected_count,
    })
}

pub(crate) async fn reset_document(
    deps: &McpConfigDeps<'_>,
) -> Result<McpConfigResetResponse, AppError> {
    let _guard = deps.mcp_config_lock.lock().await;
    let path = resolve_config_path(deps)?;
    let previous = mcp_config::load_mcp_config_file(&path).ok();

    let empty = McpConfigFile {
        mcp_servers: HashMap::new(),
    };
    mcp_config::save_mcp_config_file(&path, &empty)?;

    if let Some(ref prev) = previous {
        cleanup_removed_secret_refs(deps.secret_store.as_ref(), prev, &empty);
    }

    deps.workers
        .apply_mcp_config(crate::provider::McpRuntimeConfig::default())
        .await?;

    Ok(McpConfigResetResponse {
        message: "MCP config reset".into(),
        raw_json: redacted_json(&empty)?,
        count: 0,
    })
}

fn parse_and_validate(
    raw_json: &str,
    blocked_commands: &[String],
    dangerous_env_keys: &[String],
) -> Result<McpConfigFile, AppError> {
    if raw_json.trim().is_empty() {
        return Err(AppError::Validation("raw_json must not be empty".into()));
    }

    let file: McpConfigFile = serde_json::from_str(raw_json)
        .map_err(|e| AppError::Validation(format!("Invalid MCP JSON: {e}")))?;

    for (name, entry) in &file.mcp_servers {
        if name.trim().is_empty() {
            return Err(AppError::Validation(
                "MCP server name must not be empty".into(),
            ));
        }
        if entry.command.trim().is_empty() {
            return Err(AppError::Validation(format!(
                "MCP server '{name}' command must not be empty"
            )));
        }
        timeout_millis(entry.timeout_seconds)
            .map_err(|message| AppError::Validation(format!("MCP server '{name}' {message}")))?;
        validate_mcp_command_with_args(&entry.command, &entry.args, blocked_commands)?;
        validate_mcp_env(&entry.env, dangerous_env_keys)?;
    }

    Ok(file)
}

fn normalize_secrets(
    mut file: McpConfigFile,
    secret_keys: &HashMap<String, Vec<String>>,
    secret_store: Option<&dyn SecretStore>,
) -> Result<McpConfigFile, AppError> {
    for (server_name, entry) in &mut file.mcp_servers {
        let explicit = secret_keys.get(server_name).cloned().unwrap_or_default();
        entry.env = build_env_with_refs(server_name, &entry.env, &explicit, secret_store)?;
    }
    Ok(file)
}

fn build_env_with_refs(
    server_name: &str,
    env: &HashMap<String, String>,
    explicit_secret_keys: &[String],
    secret_store: Option<&dyn SecretStore>,
) -> Result<HashMap<String, String>, AppError> {
    let explicit: HashSet<&str> = explicit_secret_keys.iter().map(String::as_str).collect();
    let mut out = HashMap::with_capacity(env.len());

    for (key, value) in env {
        if mcp_config::is_secret_ref(value) {
            out.insert(key.clone(), value.clone());
            continue;
        }
        if value.trim().is_empty() || value.contains("${") {
            out.insert(key.clone(), value.clone());
            continue;
        }

        if explicit.contains(key.as_str()) || mcp_config::should_treat_as_secret_key(key) {
            let account = mcp_config::secret_account(server_name, key);
            if let Some(store) = secret_store {
                store.store(&account, value).map_err(|e| {
                    AppError::Config(format!("Failed to store MCP secret '{key}': {e}"))
                })?;
            }
            out.insert(key.clone(), mcp_config::secret_ref(&account));
        } else {
            out.insert(key.clone(), value.clone());
        }
    }

    Ok(out)
}

async fn build_runtime_summaries(
    workers: &Arc<dyn AgentRuntime>,
    file: &McpConfigFile,
) -> Result<Vec<McpServerSummary>, AppError> {
    let mut entries: Vec<(&String, &McpServerEntry)> = file.mcp_servers.iter().collect();
    entries.sort_by_key(|(name, _)| *name);

    let live_map: HashMap<String, crate::provider::McpServerStatus> = workers
        .live_mcp_servers()
        .await?
        .into_iter()
        .map(|server| (server.name.clone(), server))
        .collect();

    let mut summaries = Vec::with_capacity(entries.len());
    for (name, entry) in entries {
        let live = live_map.get(name);
        let tools = if live.is_some_and(|server| server.connected) {
            workers.live_mcp_tools(name).await?
        } else {
            Vec::new()
        };
        summaries.push(build_server_summary(name, entry, live, tools));
    }
    Ok(summaries)
}

fn build_server_summary(
    name: &str,
    entry: &McpServerEntry,
    live: Option<&crate::provider::McpServerStatus>,
    tools: Vec<crate::provider::McpToolDescriptor>,
) -> McpServerSummary {
    let (mut env_keys, mut secret_env_keys) = env_metadata(entry);
    env_keys.sort();
    secret_env_keys.sort();

    let (connected, status, error) = match live {
        Some(s) => (s.connected, Some(s.status.clone()), s.error.clone()),
        None => (false, None, None),
    };
    let tools: Vec<McpToolMetadata> = tools
        .into_iter()
        .map(|tool| McpToolMetadata {
            excluded: entry.excluded_tools.contains(&tool.name),
            name: tool.name,
            description: tool.description,
        })
        .collect();
    let tool_count = tools.len();

    McpServerSummary {
        name: name.to_owned(),
        command: entry.command.clone(),
        args: entry.args.clone(),
        enabled: entry.enabled,
        connected,
        status,
        tool_count,
        tools,
        excluded_tools: entry.excluded_tools.clone(),
        env_keys,
        secret_env_keys,
        error,
    }
}

fn load_mcp_file(deps: &McpConfigDeps<'_>) -> Result<(PathBuf, McpConfigFile), AppError> {
    let path = resolve_config_path(deps)?;
    let file = mcp_config::load_mcp_config_file(&path)?;
    Ok((path, file))
}

fn resolve_config_path(deps: &McpConfigDeps<'_>) -> Result<PathBuf, AppError> {
    let cfg = deps.config.load();
    mcp_config::ensure_mcp_config_exists(
        std::path::Path::new(&cfg.data_dir),
        cfg.mcp.config_file.as_deref(),
    )
}

fn blocked_and_dangerous(deps: &McpConfigDeps<'_>) -> (Vec<String>, Vec<String>) {
    let cfg = deps.config.load();
    (
        cfg.mcp_blocked_commands_vec().to_vec(),
        cfg.mcp_dangerous_env_keys_vec().to_vec(),
    )
}

fn redacted_json(file: &McpConfigFile) -> Result<String, AppError> {
    Ok(format!("{}\n", serde_json::to_string_pretty(file)?))
}

fn env_metadata(entry: &McpServerEntry) -> (Vec<String>, Vec<String>) {
    let env_keys: Vec<String> = entry.env.keys().cloned().collect();
    let secret_env_keys: Vec<String> = entry
        .env
        .iter()
        .filter(|(_, v)| mcp_config::is_secret_ref(v))
        .map(|(k, _)| k.clone())
        .collect();
    (env_keys, secret_env_keys)
}

fn cleanup_removed_secret_refs(
    secret_store: &dyn SecretStore,
    previous: &McpConfigFile,
    current: &McpConfigFile,
) {
    let prev = secret_accounts_in(previous);
    let curr = secret_accounts_in(current);

    for account in prev.difference(&curr) {
        if let Err(e) = secret_store.delete(account) {
            warn!(account, error = %e, "mcp_config.cleanup_secret_failed");
        }
    }
}

fn secret_accounts_in(file: &McpConfigFile) -> HashSet<String> {
    file.mcp_servers
        .values()
        .flat_map(|e| e.env.values())
        .filter_map(|v| {
            v.strip_prefix(mcp_config::SECRET_REF_PREFIX)
                .map(ToOwned::to_owned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_only_reports_mcp_policy_errors_as_invalid_documents() {
        let response = invalid_document_response(AppError::Mcp("blocked command".into()))
            .expect("validation response");
        assert!(!response.valid);
        assert_eq!(response.errors, ["blocked command"]);
    }

    #[test]
    fn validate_only_propagates_operational_errors() {
        let error = invalid_document_response(AppError::Io(std::io::Error::other("disk")))
            .expect_err("I/O errors must propagate");
        assert!(matches!(error, AppError::Io(_)));
    }

    #[test]
    fn owner_config_rejects_unbounded_mcp_timeouts() {
        for timeout in ["-1", "0", "600.001", "1e100"] {
            let raw = r#"{"mcpServers":{"test":{"command":"server","timeout_seconds":$TIMEOUT}}}"#
                .replace("$TIMEOUT", timeout);
            let error = parse_and_validate(&raw, &[], &[]).expect_err("invalid timeout");
            assert!(error.to_string().contains("timeout_seconds"));
        }
    }
}
