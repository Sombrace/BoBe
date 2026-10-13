use tracing::{info, warn};

use crate::config::Config;
use crate::provider::McpRuntimeConfig;

pub(crate) fn load_mcp_servers(config: &Config) -> McpRuntimeConfig {
    if !config.mcp.enabled {
        return McpRuntimeConfig::default();
    }

    let path = match crate::mcp::config::resolve_mcp_config_path(
        std::path::Path::new(&config.data_dir),
        config.mcp.config_file.as_deref(),
    ) {
        Ok(path) => path,
        Err(error) => {
            warn!(%error, "bootstrap.mcp_config_path_resolution_failed");
            return McpRuntimeConfig::default();
        }
    };

    let parsed = match crate::mcp::config::load_mcp_config(
        &path,
        &config.mcp.blocked_commands,
        &config.mcp.dangerous_env_keys,
    ) {
        Ok(servers) => servers,
        Err(error) => {
            warn!(%error, path = %path.display(), "bootstrap.mcp_config_parse_failed");
            return McpRuntimeConfig::default();
        }
    };

    let count = parsed.len();
    let runtime = crate::mcp::config::to_runtime_mcp_config(parsed);
    info!(count, "bootstrap.mcp_servers_loaded");
    runtime
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_mcp_produces_an_empty_runtime_config() {
        let mut config = Config::default();
        config.mcp.enabled = false;
        config.mcp.config_file = Some("/path/that/must/not/be/read.json".to_owned());

        let loaded = load_mcp_servers(&config);

        assert!(loaded.servers.is_empty());
    }
}
