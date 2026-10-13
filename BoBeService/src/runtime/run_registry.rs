use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(crate) struct RunRegistry {
    active: Mutex<HashMap<Uuid, ActiveRun>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunKind {
    User,
    Proactive,
    Background,
    Delivery,
    Voice,
}

struct ActiveRun {
    cancellation: CancellationToken,
    channel: Option<crate::models::ids::ConversationChannelId>,
    kind: RunKind,
    stopping: bool,
    queued: bool,
    inputs: HashMap<Uuid, ([u8; 32], Option<bool>)>,
}

#[derive(serde::Serialize)]
pub(crate) struct RunStatus {
    id: Uuid,
    channel: Option<crate::models::ids::ConversationChannelId>,
    kind: RunKind,
    stopping: bool,
    queued: bool,
}

pub(crate) struct RunRegistration {
    registry: Arc<RunRegistry>,
    id: Uuid,
    cancellation: CancellationToken,
}

impl Drop for RunRegistration {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.registry.finish(self.id);
    }
}

impl RunRegistry {
    pub(crate) fn new() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn register_cancellation(
        self: &Arc<Self>,
        run_id: Uuid,
        cancellation: CancellationToken,
        kind: RunKind,
        channel: Option<crate::models::ids::ConversationChannelId>,
    ) -> Result<RunRegistration, crate::error::AppError> {
        let mut active = lock_or_recover(&self.active);
        if active.contains_key(&run_id) {
            return Err(crate::error::AppError::Conflict(
                "run is already registered".into(),
            ));
        }
        active.insert(
            run_id,
            ActiveRun {
                cancellation: cancellation.clone(),
                kind,
                channel,
                stopping: false,
                queued: kind == RunKind::User,
                inputs: HashMap::new(),
            },
        );
        Ok(RunRegistration {
            registry: Arc::clone(self),
            id: run_id,
            cancellation,
        })
    }

    pub(crate) fn cancel(&self, run_id: Uuid) -> bool {
        let cancellation = {
            let mut active = lock_or_recover(&self.active);
            let Some(run) = active.get_mut(&run_id) else {
                return false;
            };
            run.stopping = true;
            run.cancellation.clone()
        };
        cancellation.cancel();
        true
    }

    pub(crate) fn started(&self, run_id: Uuid) {
        if let Some(run) = lock_or_recover(&self.active).get_mut(&run_id) {
            run.queued = false;
        }
    }

    pub(crate) fn begin_input(
        &self,
        run_id: Uuid,
        input_id: Uuid,
        text: &str,
        follow_up: bool,
    ) -> Result<bool, crate::error::AppError> {
        use crate::error::AppError;
        let mut hasher = Sha256::new();
        hasher.update([u8::from(follow_up)]);
        hasher.update(text.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let mut active = lock_or_recover(&self.active);
        let run = active
            .get_mut(&run_id)
            .ok_or_else(|| AppError::NotFound("run is no longer active".into()))?;
        if let Some((previous, result)) = run.inputs.get(&input_id) {
            if *previous != digest {
                return Err(AppError::Conflict(
                    "input identity was reused with different content".into(),
                ));
            }
            return match result {
                Some(true) => Ok(false),
                Some(false) => Err(AppError::RequestReplayUnsafe("The earlier input was not confirmed. Inspect the current run before sending new guidance.".into())),
                None => Err(AppError::Conflict("this input is still being submitted".into())),
            };
        }
        if run.stopping || run.cancellation.is_cancelled() {
            return Err(AppError::Conflict("run is stopping".into()));
        }
        if run.inputs.len() >= 128 {
            return Err(AppError::Conflict("run input limit reached".into()));
        }
        run.inputs.insert(input_id, (digest, None));
        Ok(true)
    }

    pub(crate) fn finish_input(&self, run_id: Uuid, input_id: Uuid, success: bool) {
        if let Some(run) = lock_or_recover(&self.active).get_mut(&run_id)
            && let Some((_, result)) = run.inputs.get_mut(&input_id)
        {
            *result = Some(success);
        }
    }

    pub(crate) fn interrupt_automation(&self, channel: crate::models::ids::ConversationChannelId) {
        let ids = lock_or_recover(&self.active)
            .iter()
            .filter(|(_, run)| {
                run.channel == Some(channel)
                    && matches!(run.kind, RunKind::Proactive | RunKind::Delivery)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in ids {
            self.cancel(id);
        }
    }

    pub(crate) fn kind(
        &self,
        id: Uuid,
        channel: Option<crate::models::ids::ConversationChannelId>,
    ) -> Option<RunKind> {
        lock_or_recover(&self.active)
            .get(&id)
            .filter(|run| channel.is_none() || run.channel == channel)
            .map(|run| run.kind)
    }

    pub(crate) fn list(
        &self,
        channel: Option<crate::models::ids::ConversationChannelId>,
    ) -> Vec<RunStatus> {
        lock_or_recover(&self.active)
            .iter()
            .filter(|(_, run)| channel.is_none() || run.channel == channel)
            .map(|(id, run)| RunStatus {
                id: *id,
                channel: run.channel,
                kind: run.kind,
                stopping: run.stopping || run.cancellation.is_cancelled(),
                queued: run.queued,
            })
            .collect()
    }

    fn finish(&self, run_id: Uuid) {
        lock_or_recover(&self.active).remove(&run_id);
    }

    #[cfg(test)]
    fn contains(&self, run_id: Uuid) -> bool {
        lock_or_recover(&self.active).contains_key(&run_id)
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::error!("run registry mutex poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn input_identity_and_channel_scope_are_bound_to_the_active_run() {
        let registry = Arc::new(RunRegistry::new());
        let id = Uuid::new_v4();
        let channel = crate::models::ids::ConversationChannelId::for_endpoint("body");
        let _run = registry
            .register_cancellation(id, CancellationToken::new(), RunKind::User, Some(channel))
            .unwrap();
        assert_eq!(registry.kind(id, Some(channel)), Some(RunKind::User));
        assert!(
            registry
                .kind(id, Some(crate::models::ids::ConversationChannelId::OWNER))
                .is_none()
        );
        let input = Uuid::new_v4();
        assert!(
            registry
                .begin_input(id, input, "change direction", false)
                .unwrap()
        );
        assert!(
            registry
                .begin_input(id, input, "change direction", false)
                .is_err()
        );
        registry.finish_input(id, input, true);
        assert!(
            !registry
                .begin_input(id, input, "change direction", false)
                .unwrap()
        );
        assert!(
            registry
                .begin_input(id, input, "different direction", false)
                .is_err()
        );
        assert!(
            registry
                .begin_input(id, input, "change direction", true)
                .is_err()
        );
    }

    #[tokio::test]
    async fn registered_run_can_be_cancelled_and_finished() {
        let registry = Arc::new(RunRegistry::new());
        let run_id = Uuid::new_v4();
        let cancellation = CancellationToken::new();
        let registration = registry
            .register_cancellation(run_id, cancellation.clone(), RunKind::User, None)
            .unwrap();
        assert!(registry.contains(run_id));
        assert!(
            registry
                .register_cancellation(run_id, CancellationToken::new(), RunKind::User, None)
                .is_err()
        );
        assert!(registry.cancel(run_id));
        assert!(cancellation.is_cancelled());
        assert!(registry.contains(run_id));
        drop(registration);
        assert!(!registry.contains(run_id));
        assert!(!registry.cancel(run_id));
    }
}
