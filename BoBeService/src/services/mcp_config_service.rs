use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

use crate::agent::PiRuntime;
use crate::config::Config;
use crate::error::AppError;
use crate::mcp::config::{self as mcp_config, McpConfigFile, McpParsedServer, McpServerEntry};
use crate::provider::McpRuntimeConfig;
use crate::secrets::SecretStore;

pub(crate) const RELOAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(90);
pub(crate) const REQUEST_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(3 * RELOAD_DEADLINE.as_secs());
const EMPTY_DOCUMENT: &str = r#"{"mcpServers":{}}"#;

#[derive(Clone)]
pub(crate) struct McpConfigDeps {
    pub(crate) config: Arc<ArcSwap<Config>>,
    pub(crate) mcp_config_lock: Arc<Mutex<()>>,
    pub(crate) workers: Arc<PiRuntime>,
    pub(crate) secret_store: Arc<dyn SecretStore>,
}

struct PendingSecret {
    key: String,
    account: String,
    value: String,
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
    deps: &McpConfigDeps,
) -> Result<McpConfigDocumentResponse, AppError> {
    let (_path, file) = load_mcp_file(deps)?;
    let raw_json = redacted_json(&file)?;

    let servers = build_runtime_summaries(&deps.workers, &file).await?;
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
    deps: &McpConfigDeps,
    body: &McpConfigMutationRequest,
) -> Result<McpConfigValidateResponse, AppError> {
    let (blocked_cmds, dangerous_keys) = blocked_and_dangerous(deps);

    let validation = parse_document(&body.raw_json, &blocked_cmds, &dangerous_keys)
        .map(|(file, _)| normalize_secrets(file, &body.secret_keys).0);
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
        AppError::Validation(message) => Ok(McpConfigValidateResponse {
            valid: false,
            normalized_json: String::new(),
            server_count: 0,
            errors: vec![message],
        }),
        error => Err(error),
    }
}

pub(crate) async fn save_document(
    deps: &McpConfigDeps,
    body: McpConfigMutationRequest,
) -> Result<McpConfigSaveResponse, AppError> {
    let file = commit_document(deps, body).await?;
    let servers = match tokio::time::timeout(
        RELOAD_DEADLINE,
        build_runtime_summaries(&deps.workers, &file),
    )
    .await
    {
        Ok(Ok(servers)) => servers,
        Ok(Err(error)) => offline_summaries(&file, &error),
        Err(elapsed) => offline_summaries(&file, &elapsed),
    };
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
    deps: &McpConfigDeps,
) -> Result<McpConfigResetResponse, AppError> {
    let empty = commit_document(
        deps,
        McpConfigMutationRequest {
            raw_json: EMPTY_DOCUMENT.into(),
            secret_keys: HashMap::new(),
        },
    )
    .await?;

    Ok(McpConfigResetResponse {
        message: "MCP config reset".into(),
        raw_json: redacted_json(&empty)?,
        count: 0,
    })
}

async fn commit_document(
    deps: &McpConfigDeps,
    body: McpConfigMutationRequest,
) -> Result<McpConfigFile, AppError> {
    let deps = deps.clone();
    tokio::spawn(async move {
        let workers = Arc::clone(&deps.workers);
        save_locked(
            &deps.mcp_config_lock,
            &deps.config,
            deps.secret_store.as_ref(),
            &body,
            async move |runtime| workers.apply_mcp_config(runtime).await,
        )
        .await
    })
    .await
    .map_err(|error| AppError::Internal(format!("MCP configuration update stopped: {error}")))?
}

async fn save_locked(
    lock: &Mutex<()>,
    config: &ArcSwap<Config>,
    secret_store: &dyn SecretStore,
    body: &McpConfigMutationRequest,
    reload: impl AsyncFnOnce(McpRuntimeConfig) -> Result<(), AppError>,
) -> Result<McpConfigFile, AppError> {
    let _guard = lock.lock().await;
    let current = config.load_full();
    let (file, servers) = parse_document(
        &body.raw_json,
        current.mcp_blocked_commands_vec(),
        current.mcp_dangerous_env_keys_vec(),
    )?;
    let (file, secrets) = normalize_secrets(file, &body.secret_keys);
    let runtime = if current.mcp.enabled {
        mcp_config::to_runtime_mcp_config(servers)
    } else {
        McpRuntimeConfig::default()
    };
    let path = mcp_config::ensure_mcp_config_exists(
        Path::new(&current.data_dir),
        current.mcp.config_file.as_deref(),
    )?;
    replace_document(&path, secret_store, &file, &secrets, reload(runtime)).await?;
    Ok(file)
}

async fn replace_document(
    path: &Path,
    secret_store: &dyn SecretStore,
    next: &McpConfigFile,
    secrets: &[PendingSecret],
    reload: impl Future<Output = Result<(), AppError>>,
) -> Result<(), AppError> {
    let previous_bytes = read_document(path)?;
    let previous = previous_bytes
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<McpConfigFile>(bytes).ok());
    let mut replaced = Vec::with_capacity(secrets.len());
    let outcome = match stage_document(path, secret_store, next, secrets, &mut replaced) {
        Ok(()) => reload.await,
        Err(error) => Err(error),
    };
    if let Err(error) = outcome {
        let restored = restore_document(path, previous_bytes.as_deref(), secret_store, &replaced);
        return Err(match restored {
            Ok(()) => error,
            Err(failures) => AppError::Internal(format!(
                "{error}; restoring the previous MCP configuration also failed: {failures}"
            )),
        });
    }
    if let Some(previous) = previous {
        cleanup_removed_secret_refs(secret_store, &previous, next);
    }
    Ok(())
}

fn stage_document(
    path: &Path,
    secret_store: &dyn SecretStore,
    next: &McpConfigFile,
    secrets: &[PendingSecret],
    replaced: &mut Vec<(String, Option<String>)>,
) -> Result<(), AppError> {
    for secret in secrets {
        replaced.push((secret.account.clone(), secret_store.read(&secret.account)));
        secret_store
            .store(&secret.account, &secret.value)
            .map_err(|error| {
                AppError::Config(format!(
                    "Failed to store MCP secret '{}': {error}",
                    secret.key
                ))
            })?;
    }
    mcp_config::save_mcp_config_file(path, next)
}

fn restore_document(
    path: &Path,
    previous: Option<&[u8]>,
    secret_store: &dyn SecretStore,
    replaced: &[(String, Option<String>)],
) -> Result<(), String> {
    let mut failures = Vec::new();
    let document = match previous {
        Some(bytes) => crate::util::durable_fs::atomic_write_sync_mode(path, bytes, 0o600),
        None => std::fs::remove_file(path).or_else(|error| {
            (error.kind() == std::io::ErrorKind::NotFound).ok_or(AppError::Io(error))
        }),
    };
    if let Err(error) = document {
        failures.push(format!("{}: {error}", path.display()));
    }
    for (account, value) in replaced.iter().rev() {
        if secret_store.read(account) == *value {
            continue;
        }
        let restored = match value {
            Some(value) => secret_store.store(account, value),
            None => secret_store.delete(account),
        };
        if let Err(error) = restored {
            failures.push(format!("MCP secret {account}: {error}"));
        }
    }
    failures.is_empty().ok_or_else(|| failures.join("; "))
}

fn read_document(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Io(error)),
    }
}

fn parse_document(
    raw_json: &str,
    blocked_commands: &[String],
    dangerous_env_keys: &[String],
) -> Result<(McpConfigFile, Vec<McpParsedServer>), AppError> {
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
    }

    let servers =
        mcp_config::parse_enabled_servers(file.clone(), blocked_commands, dangerous_env_keys)
            .map_err(|error| match error {
                AppError::Config(message) => AppError::Validation(message),
                error => error,
            })?;
    Ok((file, servers))
}

fn normalize_secrets(
    mut file: McpConfigFile,
    secret_keys: &HashMap<String, Vec<String>>,
) -> (McpConfigFile, Vec<PendingSecret>) {
    let mut secrets = Vec::new();
    for (server_name, entry) in &mut file.mcp_servers {
        let explicit = secret_keys.get(server_name).map_or(&[][..], Vec::as_slice);
        entry.env = build_env_with_refs(server_name, &entry.env, explicit, &mut secrets);
    }
    (file, secrets)
}

fn build_env_with_refs(
    server_name: &str,
    env: &HashMap<String, String>,
    explicit_secret_keys: &[String],
    secrets: &mut Vec<PendingSecret>,
) -> HashMap<String, String> {
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
            out.insert(key.clone(), mcp_config::secret_ref(&account));
            secrets.push(PendingSecret {
                key: key.clone(),
                account,
                value: value.clone(),
            });
        } else {
            out.insert(key.clone(), value.clone());
        }
    }

    out
}

fn offline_summaries(
    file: &McpConfigFile,
    reason: &dyn std::fmt::Display,
) -> Vec<McpServerSummary> {
    warn!(%reason, "mcp_config.saved_without_live_status");
    sorted_entries(file)
        .into_iter()
        .map(|(name, entry)| build_server_summary(name, entry, None, Vec::new()))
        .collect()
}

fn sorted_entries(file: &McpConfigFile) -> Vec<(&String, &McpServerEntry)> {
    let mut entries: Vec<(&String, &McpServerEntry)> = file.mcp_servers.iter().collect();
    entries.sort_by_key(|(name, _)| *name);
    entries
}

async fn build_runtime_summaries(
    workers: &Arc<PiRuntime>,
    file: &McpConfigFile,
) -> Result<Vec<McpServerSummary>, AppError> {
    let entries = sorted_entries(file);

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

fn load_mcp_file(deps: &McpConfigDeps) -> Result<(PathBuf, McpConfigFile), AppError> {
    let path = resolve_config_path(deps)?;
    let file = mcp_config::load_mcp_config_file(&path)?;
    Ok((path, file))
}

fn resolve_config_path(deps: &McpConfigDeps) -> Result<PathBuf, AppError> {
    let cfg = deps.config.load();
    mcp_config::ensure_mcp_config_exists(Path::new(&cfg.data_dir), cfg.mcp.config_file.as_deref())
}

fn blocked_and_dangerous(deps: &McpConfigDeps) -> (Vec<String>, Vec<String>) {
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
    use std::assert_matches;

    #[test]
    fn validate_only_reports_mcp_policy_errors_as_invalid_documents() {
        let response = invalid_document_response(AppError::Validation("blocked command".into()))
            .expect("validation response");
        assert!(!response.valid);
        assert_eq!(response.errors, ["blocked command"]);
    }

    #[test]
    fn validate_only_propagates_operational_errors() {
        let error = invalid_document_response(AppError::Io(std::io::Error::other("disk")))
            .expect_err("I/O errors must propagate");
        assert_matches!(error, AppError::Io(_));
    }

    #[test]
    fn owner_config_rejects_unbounded_mcp_timeouts() {
        for timeout in ["-1", "0", "600.001", "1e100"] {
            let raw = r#"{"mcpServers":{"test":{"command":"server","timeout_seconds":$TIMEOUT}}}"#
                .replace("$TIMEOUT", timeout);
            let error = parse_document(&raw, &[], &[]).expect_err("invalid timeout");
            assert_matches!(error, AppError::Validation(_));
            assert!(error.to_string().contains("timeout_seconds"));
        }
    }

    #[test]
    fn documents_are_checked_with_the_values_the_runtime_will_load() {
        let variable = format!("BOBE_TEST_UNSET_{}", uuid::Uuid::new_v4().simple()).to_uppercase();
        let raw = r#"{"mcpServers":{"test":{"command":"server","args":["${VARIABLE}"],"enabled":$ENABLED}}}"#
            .replace("VARIABLE", &variable);

        let error = parse_document(&raw.replace("$ENABLED", "true"), &[], &[])
            .expect_err("unresolvable enabled server");
        assert_matches!(error, AppError::Validation(message) if message.contains(&variable));

        let (file, servers) = parse_document(&raw.replace("$ENABLED", "false"), &[], &[])
            .expect("disabled servers are not resolved");
        assert_eq!(file.mcp_servers.len(), 1);
        assert!(servers.is_empty());
    }

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
        previous: String,
        kept: String,
        added: String,
        secrets: crate::secrets::memory::MemorySecretStore,
        next: McpConfigFile,
        pending: Vec<PendingSecret>,
    }

    fn fixture(writes_before_failure: usize) -> Fixture {
        let root = std::env::temp_dir().join(format!("bobe-mcp-commit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("mcp.json");
        let kept = mcp_config::secret_account("kept", "API_TOKEN");
        let previous = format!(
            r#"{{"mcpServers":{{
            "kept":{{"command":"server","env":{{"API_TOKEN":"bobe-secret://{kept}"}}}},
            "removed":{{"command":"server","env":{{"API_TOKEN":"bobe-secret://acct-removed"}}}}
        }}}}"#
        );
        std::fs::write(&path, &previous).unwrap();
        let (next, pending) = normalize_secrets(
            serde_json::from_str(
                r#"{"mcpServers":{"kept":{"command":"server","env":{"API_TOKEN":"new-kept","NEW_TOKEN":"brand-new"}}}}"#,
            )
            .unwrap(),
            &HashMap::new(),
        );
        let secrets = crate::secrets::memory::MemorySecretStore::holding([
            (kept.as_str(), "old-kept"),
            ("acct-removed", "old-removed"),
        ])
        .failing_after(writes_before_failure);
        Fixture {
            root,
            path,
            previous,
            added: mcp_config::secret_account("kept", "NEW_TOKEN"),
            kept,
            secrets,
            next,
            pending,
        }
    }

    #[tokio::test]
    async fn a_failed_worker_reload_restores_the_previous_document_and_secrets() {
        let fixture = fixture(usize::MAX);

        let error = replace_document(
            &fixture.path,
            &fixture.secrets,
            &fixture.next,
            &fixture.pending,
            async {
                Err(AppError::Internal(
                    "MCP reload was rolled back: server failed".into(),
                ))
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("rolled back"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&fixture.path).unwrap(),
            fixture.previous
        );
        assert_eq!(
            fixture.secrets.read(&fixture.kept).as_deref(),
            Some("old-kept")
        );
        assert_eq!(fixture.secrets.read(&fixture.added), None);
        assert_eq!(
            fixture.secrets.read("acct-removed").as_deref(),
            Some("old-removed")
        );
        std::fs::remove_dir_all(&fixture.root).unwrap();
    }

    #[tokio::test]
    async fn a_failed_restore_is_reported_with_the_reload_failure() {
        let fixture = fixture(2);

        let error = replace_document(
            &fixture.path,
            &fixture.secrets,
            &fixture.next,
            &fixture.pending,
            async { Err(AppError::Internal("MCP reload was rolled back".into())) },
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("rolled back"), "{error}");
        assert!(error.contains("injected secret store failure"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&fixture.path).unwrap(),
            fixture.previous
        );
        std::fs::remove_dir_all(&fixture.root).unwrap();
    }

    #[tokio::test]
    async fn a_confirmed_reload_commits_the_document_and_retires_removed_secrets() {
        let fixture = fixture(usize::MAX);

        replace_document(
            &fixture.path,
            &fixture.secrets,
            &fixture.next,
            &fixture.pending,
            async { Ok(()) },
        )
        .await
        .unwrap();

        let saved = mcp_config::load_mcp_config_file(&fixture.path).unwrap();
        assert_eq!(
            saved.mcp_servers.keys().collect::<Vec<_>>(),
            [&"kept".to_owned()]
        );
        assert_eq!(
            fixture.secrets.read(&fixture.kept).as_deref(),
            Some("new-kept")
        );
        assert_eq!(
            fixture.secrets.read(&fixture.added).as_deref(),
            Some("brand-new")
        );
        assert_eq!(fixture.secrets.read("acct-removed"), None);
        std::fs::remove_dir_all(&fixture.root).unwrap();
    }

    #[tokio::test]
    async fn a_save_queued_behind_an_mcp_switch_applies_the_switched_state() {
        let root = std::env::temp_dir().join(format!("bobe-mcp-switch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut initial = Config {
            data_dir: root.to_string_lossy().into_owned(),
            ..Config::default()
        };
        initial.mcp.enabled = true;
        let config = Arc::new(ArcSwap::from_pointee(initial));
        let lock = Arc::new(Mutex::new(()));
        let reloaded = Arc::new(std::sync::Mutex::new(None));
        let switch = Arc::clone(&lock).lock_owned().await;
        let save = tokio::spawn({
            let (lock, config, reloaded) = (
                Arc::clone(&lock),
                Arc::clone(&config),
                Arc::clone(&reloaded),
            );
            async move {
                let body = McpConfigMutationRequest {
                    raw_json: r#"{"mcpServers":{"tools":{"command":"server"}}}"#.into(),
                    secret_keys: HashMap::new(),
                };
                save_locked(
                    &lock,
                    &config,
                    &crate::secrets::memory::MemorySecretStore::holding([]),
                    &body,
                    async move |runtime: McpRuntimeConfig| {
                        *reloaded.lock().unwrap() = Some(runtime.servers.len());
                        Ok(())
                    },
                )
                .await
            }
        });
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        let mut disabled = (*config.load_full()).clone();
        disabled.mcp.enabled = false;
        config.store(Arc::new(disabled));
        drop(switch);

        let saved = save.await.unwrap().unwrap();

        assert_eq!(saved.mcp_servers.len(), 1);
        assert_eq!(*reloaded.lock().unwrap(), Some(0));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_resolved_blocked_command_is_rejected_before_saving() {
        let raw = r#"{"mcpServers":{"test":{"command":"${BOBE_TEST_COMMAND:-rm}"}}}"#;
        let error = parse_document(raw, &[], &[]).expect_err("blocked after resolution");
        assert_matches!(error, AppError::Validation(_));
    }
}
