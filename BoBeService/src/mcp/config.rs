use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::security::{validate_mcp_command_with_args, validate_mcp_env};
use crate::error::AppError;

const DEFAULT_MCP_CONFIG_JSON: &str = include_str!("../assets/defaults/mcp_servers.json");
const MIN_TIMEOUT_SECONDS: f64 = 0.001;
const MAX_TIMEOUT_SECONDS: f64 = 600.0;

pub(crate) const SECRET_REF_PREFIX: &str = "bobe-secret://";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct McpServerEntry {
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default = "default_timeout")]
    pub(crate) timeout_seconds: f64,
    #[serde(default)]
    pub(crate) excluded_tools: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> f64 {
    30.0
}

pub(crate) fn timeout_millis(timeout_seconds: f64) -> Result<u32, &'static str> {
    if timeout_seconds.is_finite()
        && (MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&timeout_seconds)
    {
        Ok((timeout_seconds * 1000.0).round() as u32)
    } else {
        Err("timeout_seconds must be finite and between 0.001 and 600")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct McpConfigFile {
    #[serde(rename = "mcpServers", alias = "servers", default)]
    pub(crate) mcp_servers: HashMap<String, McpServerEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct McpParsedServer {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) timeout_ms: u32,
    pub(crate) excluded_tools: Vec<String>,
}

pub(crate) fn resolve_mcp_config_path(
    data_root: &Path,
    config_file: Option<&str>,
) -> Result<PathBuf, AppError> {
    if let Some(path) = config_file
        && !path.trim().is_empty()
    {
        return Ok(crate::util::paths::expand_tilde(path));
    }

    Ok(data_root.join("mcp.json"))
}

pub(crate) fn ensure_mcp_config_exists(
    data_root: &Path,
    config_file: Option<&str>,
) -> Result<PathBuf, AppError> {
    let path = resolve_mcp_config_path(data_root, config_file)?;
    if path.exists() {
        return Ok(path);
    }

    let defaults: McpConfigFile = serde_json::from_str(DEFAULT_MCP_CONFIG_JSON)
        .map_err(|e| AppError::Config(format!("Invalid bundled MCP defaults: {e}")))?;
    save_mcp_config_file(&path, &defaults)?;
    Ok(path)
}

pub(crate) fn load_mcp_config_file(path: &Path) -> Result<McpConfigFile, AppError> {
    let content = fs::read_to_string(path)
        .map_err(|e| AppError::Config(format!("Cannot read MCP config {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| AppError::Config(format!("Invalid MCP config JSON: {e}")))
}

pub(crate) fn save_mcp_config_file(path: &Path, config: &McpConfigFile) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }

    let content = serde_json::to_string_pretty(config)?;
    crate::util::durable_fs::atomic_write_sync(path, format!("{content}\n").as_bytes())?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub(crate) fn load_mcp_config(
    path: &Path,
    blocked_commands: &[String],
    dangerous_env_keys: &[String],
) -> Result<Vec<McpParsedServer>, AppError> {
    let file = load_mcp_config_file(path)?;
    parse_enabled_servers(file, blocked_commands, dangerous_env_keys)
}

pub(crate) fn to_runtime_mcp_config(
    mut servers: Vec<McpParsedServer>,
) -> crate::provider::McpRuntimeConfig {
    servers.sort_by(|left, right| left.name.cmp(&right.name));
    let servers = servers
        .into_iter()
        .map(|server| {
            let spec = crate::provider::McpServerSpec {
                command: server.command,
                args: server.args,
                env: server.env,
                timeout_ms: server.timeout_ms,
                excluded_tools: server.excluded_tools,
            };
            (server.name, spec)
        })
        .collect();
    crate::provider::McpRuntimeConfig { servers }
}

pub(crate) fn parse_enabled_servers(
    file: McpConfigFile,
    blocked_commands: &[String],
    dangerous_env_keys: &[String],
) -> Result<Vec<McpParsedServer>, AppError> {
    let mut servers = Vec::new();
    for (name, entry) in file.mcp_servers {
        if !entry.enabled {
            continue;
        }
        let timeout_ms = timeout_millis(entry.timeout_seconds).map_err(|message| {
            AppError::Config(format!("Invalid MCP server '{name}': {message}"))
        })?;

        let command = resolve_value(&entry.command)?;
        let args = entry
            .args
            .iter()
            .map(|arg| resolve_value(arg))
            .collect::<Result<Vec<_>, _>>()?;
        let env = entry
            .env
            .iter()
            .map(|(k, v)| resolve_value(v).map(|resolved| (k.clone(), resolved)))
            .collect::<Result<HashMap<_, _>, _>>()?;

        validate_mcp_env(&env, dangerous_env_keys)?;
        validate_mcp_command_with_args(&command, &args, blocked_commands)?;

        servers.push(McpParsedServer {
            name,
            command,
            args,
            env,
            timeout_ms,
            excluded_tools: entry.excluded_tools,
        });
    }
    Ok(servers)
}

pub(crate) fn is_secret_ref(value: &str) -> bool {
    value.starts_with(SECRET_REF_PREFIX)
}

pub(crate) fn secret_ref(account: &str) -> String {
    format!("{SECRET_REF_PREFIX}{account}")
}

pub(crate) fn secret_account(server_name: &str, env_key: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!(
        "mcp-v2-{}-{}",
        encoded.encode(server_name.as_bytes()),
        encoded.encode(env_key.as_bytes())
    )
}

pub(crate) fn should_treat_as_secret_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.ends_with("API_KEY")
}

fn resolve_value(raw: &str) -> Result<String, AppError> {
    if let Some(account) = raw.strip_prefix(SECRET_REF_PREFIX) {
        return resolve_secret(account);
    }
    expand_env_templates(raw)
}

fn resolve_secret(account: &str) -> Result<String, AppError> {
    if account.trim().is_empty() {
        return Err(AppError::Config(
            "Invalid empty MCP secret reference".into(),
        ));
    }
    crate::secrets::read_secret(account)
        .ok_or_else(|| AppError::Config(format!("Missing MCP secret for account '{account}'")))
}

fn expand_env_templates(input: &str) -> Result<String, AppError> {
    if !input.contains("${") {
        return Ok(input.to_owned());
    }

    let mut out = String::new();
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let token_start = start + 2;
        let after_start = &rest[token_start..];
        let end = after_start.find('}').ok_or_else(|| {
            AppError::Config(format!("Invalid MCP template (missing }}): '{input}'"))
        })?;
        let token = &after_start[..end];
        rest = &after_start[end + 1..];

        let (name, default) = if let Some((n, d)) = token.split_once(":-") {
            (n.trim(), Some(d))
        } else {
            (token.trim(), None)
        };

        if name.is_empty() {
            return Err(AppError::Config(format!(
                "Invalid MCP template variable in '{input}'"
            )));
        }

        let value = match std::env::var(name) {
            Ok(v) => v,
            Err(_) => default.map_or_else(
                || {
                    Err(AppError::Config(format!(
                        "Missing environment variable '{name}' required by MCP config"
                    )))
                },
                |d| Ok(d.to_owned()),
            )?,
        };
        out.push_str(&value);
    }

    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_server(name: &str, excluded_tools: &[&str]) -> McpParsedServer {
        McpParsedServer {
            name: name.to_owned(),
            command: "server".into(),
            args: Vec::new(),
            env: HashMap::new(),
            timeout_ms: 30_000,
            excluded_tools: excluded_tools
                .iter()
                .map(|tool| (*tool).to_owned())
                .collect(),
        }
    }

    #[test]
    fn secret_accounts_preserve_component_boundaries() {
        assert_ne!(secret_account("a_b", "c"), secret_account("a", "b_c"));
        assert_ne!(
            secret_account("a-b", "TOKEN"),
            secret_account("a_b", "TOKEN")
        );
    }

    #[test]
    fn mcp_timeout_is_finite_and_bounded() {
        assert_eq!(timeout_millis(0.001), Ok(1));
        assert_eq!(timeout_millis(30.0), Ok(30_000));
        assert_eq!(timeout_millis(600.0), Ok(600_000));
        for invalid in [
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            600.001,
            f64::INFINITY,
            f64::NAN,
        ] {
            assert!(timeout_millis(invalid).is_err());
        }
    }

    #[test]
    fn invalid_timeout_is_rejected_when_loading_enabled_servers() {
        let file = McpConfigFile {
            mcp_servers: HashMap::from([(
                "broken".into(),
                McpServerEntry {
                    command: "server".into(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    enabled: true,
                    timeout_seconds: -1.0,
                    excluded_tools: Vec::new(),
                },
            )]),
        };

        let error = parse_enabled_servers(file, &[], &[]).expect_err("negative timeout");
        assert!(error.to_string().contains("timeout_seconds"));
    }

    #[test]
    fn runtime_servers_and_exclusions_are_deterministic_and_server_scoped() {
        let servers = vec![
            parsed_server("zeta", &["write", "read"]),
            parsed_server("alpha", &["read"]),
        ];

        let config = to_runtime_mcp_config(servers);

        assert_eq!(
            config
                .servers
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["alpha", "zeta"]
        );
        assert_eq!(config.servers["alpha"].excluded_tools, ["read"]);
        assert_eq!(config.servers["zeta"].excluded_tools, ["write", "read"]);
    }
}
