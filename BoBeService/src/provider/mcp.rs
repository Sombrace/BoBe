use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct McpServerSpec {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) timeout_ms: u32,
    pub(crate) excluded_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct McpRuntimeConfig {
    pub(crate) servers: BTreeMap<String, McpServerSpec>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct McpServerStatus {
    pub(crate) name: String,
    pub(crate) connected: bool,
    #[serde(deserialize_with = "deserialize_status")]
    pub(crate) status: String,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct McpToolDescriptor {
    pub(crate) name: String,
    pub(crate) description: String,
}

fn deserialize_status<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    use crate::constants::mcp_status::{CONNECTED, FAILED, PENDING, STOPPED};
    use serde::Deserialize;
    let status = String::deserialize(deserializer)?;
    if matches!(status.as_str(), CONNECTED | FAILED | PENDING | STOPPED) {
        Ok(status)
    } else {
        Err(serde::de::Error::custom("unknown pi MCP connection state"))
    }
}
