use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::Stream;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::error::AppError;

use super::{
    ChatDelta, ChatPrompt, JobInput, JobOutput, McpRuntimeConfig, McpServerStatus,
    McpToolDescriptor, MemoryFile, WorkerClass, WorkerError,
};

pub(crate) type ChatStream = Pin<Box<dyn Stream<Item = ChatDelta> + Send>>;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct BackgroundResult {
    pub(crate) session_id: String,
    pub(crate) output: String,
}

pub(crate) trait ChatRuntime: Send + Sync {
    fn send(&self, prompt: ChatPrompt) -> ChatStream;
}

#[derive(Debug, Default)]
pub(crate) struct AgentAuthorityLifecycle {
    lock: Arc<RwLock<()>>,
    maintenance_active: Arc<AtomicBool>,
}

pub(crate) struct AgentMaintenanceGuard {
    flag: Arc<AtomicBool>,
}

impl Drop for AgentMaintenanceGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

impl AgentAuthorityLifecycle {
    pub(crate) async fn write_for_owner_operation(&self) -> OwnedRwLockWriteGuard<()> {
        Arc::clone(&self.lock).write_owned().await
    }

    pub(crate) fn maintenance_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.maintenance_active)
    }

    pub(crate) fn try_begin_maintenance(&self) -> Result<AgentMaintenanceGuard, AppError> {
        self.maintenance_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| AppError::Conflict("agent maintenance is already in progress".into()))?;
        Ok(AgentMaintenanceGuard {
            flag: Arc::clone(&self.maintenance_active),
        })
    }

    pub(crate) async fn read(&self) -> Result<OwnedRwLockReadGuard<()>, AppError> {
        if self.maintenance_active.load(Ordering::Acquire) {
            return Err(AppError::Conflict("agent maintenance is active".into()));
        }
        let guard = Arc::clone(&self.lock).read_owned().await;
        if self.maintenance_active.load(Ordering::Acquire) {
            drop(guard);
            return Err(AppError::Conflict("agent maintenance is active".into()));
        }
        Ok(guard)
    }
}

#[async_trait::async_trait]
pub(crate) trait AgentRuntime: Send + Sync {
    async fn cancel_run(&self, run_id: uuid::Uuid) -> Result<bool, AppError>;
    async fn steer_run(
        &self,
        run_id: uuid::Uuid,
        text: &str,
        follow_up: bool,
    ) -> Result<(), AppError>;
    async fn chat(&self) -> Result<Arc<dyn ChatRuntime>, AppError>;
    async fn run_background(
        &self,
        job: &crate::db::agent_job_repo::AgentJob,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<BackgroundResult, AppError>;

    async fn probe_chat_provider(&self) -> Result<super::diagnostics::ProviderProbe, AppError>;
    async fn context_status(
        &self,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<super::diagnostics::AgentContextStatus, AppError>;
    async fn compact_context(
        &self,
        keep_recent_tokens: Option<u32>,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<super::diagnostics::CompactionResult, AppError>;

    async fn run_auxiliary(
        &self,
        class: WorkerClass,
        job: JobInput,
    ) -> Result<JobOutput, WorkerError>;

    fn memory_file(&self) -> Arc<MemoryFile>;

    async fn live_mcp_servers(&self) -> Result<Vec<McpServerStatus>, AppError>;
    async fn live_mcp_tools(&self, server_name: &str) -> Result<Vec<McpToolDescriptor>, AppError>;
    async fn apply_mcp_config(&self, config: McpRuntimeConfig) -> Result<(), AppError>;

    async fn reload_skills(&self) -> Result<(), AppError> {
        Ok(())
    }
    async fn shutdown_all(&self);
    async fn quiesce_for_snapshot(&self) -> Result<(), AppError>;
    async fn purge_sessions(&self) -> Result<(), AppError>;
}
