use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::Stream;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::error::AppError;

use super::ChatDelta;

pub(crate) type ChatStream = Pin<Box<dyn Stream<Item = ChatDelta> + Send>>;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct BackgroundResult {
    pub(crate) session_id: String,
    pub(crate) output: String,
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
