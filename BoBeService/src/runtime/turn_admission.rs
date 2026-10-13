use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::models::ids::ConversationChannelId;
use crate::util::atomic_flag_guard::AtomicFlagGuard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnSource {
    User,
    Capture,
    Checkin,
    Maintenance,
    Background,
}

impl TurnSource {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Capture => "capture",
            Self::Checkin => "checkin",
            Self::Maintenance => "maintenance",
            Self::Background => "background",
        }
    }
}

struct State {
    channels: HashMap<ConversationChannelId, ChannelState>,
    waiting: HashMap<ConversationChannelId, VecDeque<u64>>,
    maintenance: Arc<AtomicBool>,
}

struct ChannelState {
    busy: Arc<AtomicBool>,
    source: TurnSource,
    cancellation: CancellationToken,
}

pub(crate) struct TurnAdmission {
    state: Mutex<State>,
    wake: Arc<Notify>,
    next_ticket: AtomicU64,
    capacity: usize,
}

pub(crate) struct TurnPermit {
    flag: Option<AtomicFlagGuard>,
    wake: Arc<Notify>,
    cancellation: CancellationToken,
}

impl Drop for TurnPermit {
    fn drop(&mut self) {
        drop(self.flag.take());
        self.wake.notify_waiters();
    }
}

impl TurnPermit {
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

pub(crate) struct QueuedTurn {
    admission: Arc<TurnAdmission>,
    channel: ConversationChannelId,
    ticket: u64,
}

impl Drop for QueuedTurn {
    fn drop(&mut self) {
        let mut state = self.admission.lock();
        if let Some(waiting) = state.waiting.get_mut(&self.channel) {
            waiting.retain(|ticket| *ticket != self.ticket);
            if waiting.is_empty() {
                state.waiting.remove(&self.channel);
            }
        }
        drop(state);
        self.admission.wake.notify_waiters();
    }
}

impl QueuedTurn {
    pub(crate) async fn admit(self) -> TurnPermit {
        loop {
            let notification = self.admission.wake.notified();
            if let Some(permit) =
                self.admission
                    .try_channel(TurnSource::User, self.channel, Some(self.ticket))
            {
                return permit;
            }
            notification.await;
        }
    }
}

impl TurnAdmission {
    #[cfg(test)]
    pub(crate) fn new() -> Arc<Self> {
        Self::with_capacity(8)
    }

    pub(crate) fn with_capacity(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                channels: HashMap::new(),
                waiting: HashMap::new(),
                maintenance: Arc::new(AtomicBool::new(false)),
            }),
            wake: Arc::new(Notify::new()),
            next_ticket: AtomicU64::new(1),
            capacity,
        })
    }

    pub(crate) fn try_admit(&self, source: TurnSource) -> Option<TurnPermit> {
        self.try_channel(source, ConversationChannelId::OWNER, None)
    }

    pub(crate) fn try_admit_channel(
        &self,
        source: TurnSource,
        channel: ConversationChannelId,
    ) -> Option<TurnPermit> {
        self.try_channel(source, channel, None)
    }

    fn try_channel(
        &self,
        source: TurnSource,
        channel: ConversationChannelId,
        ticket: Option<u64>,
    ) -> Option<TurnPermit> {
        let mut state = self.lock();
        state
            .channels
            .retain(|_, slot| slot.busy.load(Ordering::Acquire));
        if state.maintenance.load(Ordering::Acquire) {
            return None;
        }
        let active = state
            .channels
            .values()
            .filter(|slot| slot.busy.load(Ordering::Acquire))
            .count();
        let (flag, cancellation) = if source == TurnSource::Maintenance {
            if active != 0 || !state.waiting.is_empty() {
                return None;
            }
            (Arc::clone(&state.maintenance), CancellationToken::new())
        } else {
            if source != TurnSource::User && !state.waiting.is_empty() {
                return None;
            }
            if active >= self.capacity {
                return None;
            }
            if let Some(waiting) = state.waiting.get(&channel)
                && (source != TurnSource::User || ticket != waiting.front().copied())
            {
                return None;
            }
            let slot = state
                .channels
                .entry(channel)
                .or_insert_with(|| ChannelState {
                    busy: Arc::new(AtomicBool::new(false)),
                    source,
                    cancellation: CancellationToken::new(),
                });
            if slot.busy.load(Ordering::Acquire) {
                if source == TurnSource::User && slot.source != TurnSource::User {
                    slot.cancellation.cancel();
                }
                return None;
            }
            slot.source = source;
            slot.cancellation = CancellationToken::new();
            (Arc::clone(&slot.busy), slot.cancellation.clone())
        };
        Some(TurnPermit {
            flag: Some(AtomicFlagGuard::try_acquire(flag)?),
            wake: Arc::clone(&self.wake),
            cancellation,
        })
    }

    pub(crate) async fn admit_user(
        self: &Arc<Self>,
        channel: ConversationChannelId,
    ) -> Result<TurnPermit, AppError> {
        Ok(self.queue_user(channel)?.admit().await)
    }

    pub(crate) fn queue_user(
        self: &Arc<Self>,
        channel: ConversationChannelId,
    ) -> Result<QueuedTurn, AppError> {
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        {
            let mut state = self.lock();
            if state.maintenance.load(Ordering::Acquire) {
                return Err(AppError::Conflict(
                    "personal data maintenance is active".into(),
                ));
            }
            if state.waiting.values().map(VecDeque::len).sum::<usize>() >= 64 {
                return Err(AppError::Conflict(
                    "the incoming message queue is full".into(),
                ));
            }
            state.waiting.entry(channel).or_default().push_back(ticket);
            if let Some(active) = state.channels.get(&channel)
                && active.source != TurnSource::User
            {
                active.cancellation.cancel();
            }
            if state
                .channels
                .values()
                .filter(|slot| slot.busy.load(Ordering::Acquire))
                .count()
                >= self.capacity
                && let Some(active) = state.channels.values().find(|slot| {
                    slot.source != TurnSource::User && !slot.cancellation.is_cancelled()
                })
            {
                active.cancellation.cancel();
            }
        }
        Ok(QueuedTurn {
            admission: Arc::clone(self),
            channel,
            ticket,
        })
    }

    pub(crate) fn is_idle(&self) -> bool {
        let state = self.lock();
        !state.maintenance.load(Ordering::Acquire)
            && state
                .channels
                .values()
                .all(|slot| !slot.busy.load(Ordering::Acquire))
    }

    pub(crate) fn accepts_messages(&self) -> bool {
        !self.lock().maintenance.load(Ordering::Acquire)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| {
            tracing::error!("turn admission state was poisoned");
            poisoned.into_inner()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_input_preempts_automation_but_waits_for_its_settlement() {
        let admission = TurnAdmission::with_capacity(1);
        let automated = admission.try_admit(TurnSource::Checkin).unwrap();
        let cancellation = automated.cancellation();
        let other = ConversationChannelId::for_endpoint("other");
        let queued = admission.queue_user(other).unwrap();
        assert!(cancellation.is_cancelled());
        let user = tokio::spawn(queued.admit());
        tokio::task::yield_now().await;
        assert!(!user.is_finished());
        drop(automated);
        let permit = tokio::time::timeout(std::time::Duration::from_secs(1), user)
            .await
            .unwrap()
            .unwrap();
        assert!(admission.try_admit(TurnSource::Checkin).is_none());
        drop(permit);
        assert!(admission.is_idle());
    }

    #[tokio::test]
    async fn message_order_is_reserved_before_tasks_are_scheduled() {
        let admission = TurnAdmission::new();
        let first = admission.queue_user(ConversationChannelId::OWNER).unwrap();
        let second = admission.queue_user(ConversationChannelId::OWNER).unwrap();
        let waiting = tokio::spawn(second.admit());
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        let active = first.admit().await;
        assert!(!waiting.is_finished());
        drop(active);
        drop(waiting.await.unwrap());
        assert!(admission.is_idle());
    }

    #[tokio::test]
    async fn channels_are_independent_but_a_channel_and_maintenance_remain_exclusive() {
        let admission = TurnAdmission::new();
        let owner = admission.try_admit(TurnSource::User).unwrap();
        assert!(admission.try_admit(TurnSource::Checkin).is_none());
        assert!(admission.try_admit(TurnSource::Maintenance).is_none());
        let other = ConversationChannelId::for_endpoint("other");
        let independent = admission
            .try_admit_channel(TurnSource::User, other)
            .unwrap();
        drop(owner);
        assert!(admission.try_admit(TurnSource::Maintenance).is_none());
        drop(independent);
        assert!(admission.try_admit(TurnSource::Maintenance).is_some());
    }

    #[tokio::test]
    async fn waiting_user_input_has_priority_over_the_next_automated_turn() {
        let admission = TurnAdmission::new();
        let first = admission.try_admit(TurnSource::Checkin).unwrap();
        let next = Arc::clone(&admission);
        let user =
            tokio::spawn(
                async move { next.admit_user(ConversationChannelId::OWNER).await.unwrap() },
            );
        tokio::task::yield_now().await;
        drop(first);
        assert!(admission.try_admit(TurnSource::Background).is_none());
        let permit = user.await.unwrap();
        assert!(admission.try_admit(TurnSource::Checkin).is_none());
        drop(permit);
        assert!(admission.is_idle());
    }
}
