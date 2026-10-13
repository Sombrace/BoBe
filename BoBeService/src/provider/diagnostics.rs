use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct ProviderProbe {
    pub(crate) model_execution: bool,
    pub(crate) streaming: bool,
    pub(crate) tool_execution: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

impl ProviderProbe {
    pub(crate) fn passed(&self) -> bool {
        self.model_execution && self.streaming && self.tool_execution && self.error.is_none()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct TokenUsage {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
    pub(crate) total: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct AgentContextStatus {
    pub(crate) session_id: String,
    pub(crate) profile: String,
    pub(crate) model: Option<String>,
    pub(crate) user_messages: u64,
    pub(crate) assistant_messages: u64,
    pub(crate) tool_calls: u64,
    pub(crate) tool_results: u64,
    pub(crate) tokens: TokenUsage,
    pub(crate) last_response_tokens: Option<TokenUsage>,
    pub(crate) context_tokens: Option<u64>,
    pub(crate) context_window: Option<u64>,
    pub(crate) context_percent: Option<f64>,
    pub(crate) compactions: u64,
    pub(crate) active_children: u64,
    pub(crate) execution: ExecutionStatus,
    pub(crate) live_channels: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ExecutionGateStatus {
    pub(crate) active: usize,
    pub(crate) queued: usize,
    pub(crate) background: usize,
    pub(crate) capacity: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ExecutionStatus {
    pub(crate) models: ExecutionGateStatus,
    pub(crate) tools: ExecutionGateStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CompactionResult {
    pub(crate) session_id: String,
    pub(crate) tokens_before: u64,
    pub(crate) estimated_tokens_after: Option<u64>,
    pub(crate) first_kept_entry_id: String,
    pub(crate) summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_execution_requires_stream_tool_and_no_error() {
        let mut probe = ProviderProbe::default();
        assert!(!probe.passed());
        probe.model_execution = true;
        probe.streaming = true;
        assert!(!probe.passed());
        probe.tool_execution = true;
        assert!(probe.passed());
        probe.error = Some("cleanup failed".into());
        assert!(!probe.passed());
    }
}
