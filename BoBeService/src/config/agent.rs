use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub(crate) enum AgentTool {
    Read,
    Bash,
    Edit,
    Write,
    Grep,
    Find,
    Ls,
    Subagent,
    BackgroundJobs,
}

impl AgentTool {
    pub(crate) const ALL: [Self; 9] = [
        Self::Read,
        Self::Bash,
        Self::Edit,
        Self::Write,
        Self::Grep,
        Self::Find,
        Self::Ls,
        Self::Subagent,
        Self::BackgroundJobs,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Bash => "bash",
            Self::Edit => "edit",
            Self::Write => "write",
            Self::Grep => "grep",
            Self::Find => "find",
            Self::Ls => "ls",
            Self::Subagent => "subagent",
            Self::BackgroundJobs => "background_jobs",
        }
    }

    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::Read => "Read files and images",
            Self::Bash => "Run shell commands, including Git",
            Self::Edit => "Apply edits to existing files",
            Self::Write => "Create and replace files",
            Self::Grep => "Search file contents",
            Self::Find => "Find files by name",
            Self::Ls => "List directory contents",
            Self::Subagent => "Delegate work and await its result",
            Self::BackgroundJobs => "Start and manage durable background work",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AgentConfig {
    pub(crate) enabled_tools: BTreeSet<AgentTool>,
    pub(crate) execution: ExecutionLimits,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled_tools: AgentTool::ALL.into_iter().collect(),
            execution: ExecutionLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ExecutionLimits {
    pub(crate) model_concurrency: usize,
    pub(crate) foreground_reserve: usize,
    pub(crate) process_concurrency: usize,
    pub(crate) tool_concurrency: usize,
    pub(crate) live_channels: usize,
    pub(crate) idle_channel_seconds: u64,
    pub(crate) task_timeout_seconds: u32,
    pub(crate) task_max_turns: u32,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            model_concurrency: 4,
            foreground_reserve: 1,
            process_concurrency: 64,
            tool_concurrency: 8,
            live_channels: 8,
            idle_channel_seconds: 900,
            task_timeout_seconds: 1800,
            task_max_turns: 128,
        }
    }
}

impl ExecutionLimits {
    pub(crate) fn validate(&self) -> Result<(), crate::error::AppError> {
        if !(2..=32).contains(&self.model_concurrency)
            || self.foreground_reserve == 0
            || self.foreground_reserve >= self.model_concurrency
            || !(4..=256).contains(&self.process_concurrency)
            || !(1..=64).contains(&self.tool_concurrency)
            || !(1..=32).contains(&self.live_channels)
            || !(30..=86400).contains(&self.idle_channel_seconds)
            || !(1..=86400).contains(&self.task_timeout_seconds)
            || !(1..=512).contains(&self.task_max_turns)
        {
            return Err(crate::error::AppError::Validation(
                "invalid agent execution ceilings".into(),
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_selection_rejects_unknown_names_and_allows_no_general_tools() {
        let empty: AgentConfig = serde_json::from_str(r#"{"enabled_tools":[]}"#).unwrap();
        assert!(empty.enabled_tools.is_empty());
        assert_eq!(
            AgentConfig::default().enabled_tools.len(),
            AgentTool::ALL.len()
        );
        assert!(serde_json::from_str::<AgentConfig>(r#"{"enabled_tools":["invented"]}"#).is_err());
    }
}
