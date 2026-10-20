use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;
use uuid::Uuid;

use super::factories::{indicator_event, snapshot_required_event};
use super::types::{EventAudience, IndicatorType, StreamBundle};
use crate::models::ids::ConversationChannelId;

type ActiveIndicators = HashMap<Uuid, (ConversationChannelId, IndicatorType)>;

pub(crate) struct EventQueue {
    history: Mutex<VecDeque<Arc<StreamBundle>>>,
    max_size: usize,
    sender: broadcast::Sender<Arc<StreamBundle>>,
    next_event_id: AtomicU64,
    runtime_incarnation_id: Uuid,
    indicators: Mutex<ActiveIndicators>,
}

pub(crate) struct EventSubscription {
    channel: ConversationChannelId,
    initial_indicator: Option<StreamBundle>,
    replay: VecDeque<Arc<StreamBundle>>,
    pending_gap: Option<StreamBundle>,
    receiver: broadcast::Receiver<Arc<StreamBundle>>,
    resume_after: Option<u64>,
}

impl EventSubscription {
    pub(crate) fn take_initial_indicator(&mut self) -> Option<StreamBundle> {
        self.initial_indicator.take()
    }

    pub(crate) fn resume_after(&self) -> Option<u64> {
        self.resume_after
    }

    pub(crate) async fn recv(&mut self) -> Result<Arc<StreamBundle>, broadcast::error::RecvError> {
        loop {
            let event = if let Some(gap) = self.pending_gap.take() {
                Arc::new(gap)
            } else if let Some(event) = self.replay.pop_front() {
                event
            } else {
                self.receiver.recv().await?
            };
            if event.audience == Some(EventAudience::Owner)
                && event
                    .channel_id
                    .is_none_or(|channel| channel == self.channel)
            {
                return Ok(event);
            }
        }
    }
}

impl EventQueue {
    pub(crate) fn push_in_channel(&self, channel: ConversationChannelId, mut event: StreamBundle) {
        event.channel_id = Some(channel);
        self.push(event);
    }
    #[cfg(test)]
    pub(crate) fn new(max_size: usize) -> Self {
        Self::with_incarnation(max_size, Uuid::nil())
    }

    pub(crate) fn with_incarnation(max_size: usize, runtime_incarnation_id: Uuid) -> Self {
        assert!(
            max_size > 0,
            "SSE event history must retain at least one event"
        );
        let (sender, _) = broadcast::channel(max_size);
        Self {
            history: Mutex::new(VecDeque::with_capacity(max_size)),
            max_size,
            sender,
            next_event_id: AtomicU64::new(1),
            runtime_incarnation_id,
            indicators: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn push(&self, mut event: StreamBundle) {
        let mut history = lock_or_recover(&self.history, "event_queue.history");
        let event_id = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        event.event_id = Some(event_id);
        event.runtime_incarnation_id = Some(self.runtime_incarnation_id);
        event.audience.get_or_insert(EventAudience::Owner);
        if event.run_id.is_none()
            && (event.message_id.starts_with("msg_") || event.message_id.starts_with("voice_"))
            && let Ok(turn_id) =
                crate::models::ids::conversation_turn_id_from_wire_id(&event.message_id)
        {
            let id = Uuid::from(turn_id);
            event.run_id = Some(id);
            event.turn_id = Some(id);
        }
        let event = Arc::new(event);

        if history.len() >= self.max_size {
            history.pop_front();
            tracing::debug!(
                event_id,
                "SSE event history overflow, dropping oldest event"
            );
        }
        history.push_back(Arc::clone(&event));
        let _receiver_count = self.sender.send(event);
    }

    #[cfg(test)]
    pub(crate) fn subscribe(&self, after_event_id: Option<u64>) -> EventSubscription {
        self.subscribe_in_channel(after_event_id, ConversationChannelId::OWNER)
    }

    pub(crate) fn subscribe_in_channel(
        &self,
        after_event_id: Option<u64>,
        channel: ConversationChannelId,
    ) -> EventSubscription {
        let indicators = lock_or_recover(&self.indicators, "event_queue.indicators");
        let current_indicator = aggregate_indicator(&indicators, Some(channel));
        let history = lock_or_recover(&self.history, "event_queue.history");
        let receiver = self.sender.subscribe();
        let latest = history.back().and_then(|event| event.event_id);
        let oldest = history.front().and_then(|event| event.event_id);

        let (replay, pending_gap) = match after_event_id {
            None => (VecDeque::new(), None),
            Some(after) if history.is_empty() => (
                VecDeque::new(),
                Some(snapshot_required_event(
                    Some(after.saturating_add(1)),
                    None,
                    None,
                    "history_unavailable",
                )),
            ),
            Some(after) if latest.is_some_and(|latest| after > latest) => (
                VecDeque::new(),
                Some(snapshot_required_event(
                    latest.map(|value| value.saturating_add(1)),
                    Some(after),
                    latest,
                    "cursor_ahead_of_history",
                )),
            ),
            Some(after) if oldest.is_some_and(|oldest| after.saturating_add(1) < oldest) => (
                VecDeque::new(),
                Some(snapshot_required_event(
                    Some(after.saturating_add(1)),
                    oldest.map(|value| value.saturating_sub(1)),
                    latest,
                    "history_expired",
                )),
            ),
            Some(after) => (
                history
                    .iter()
                    .filter(|event| event.event_id.is_some_and(|id| id > after))
                    .cloned()
                    .collect(),
                None,
            ),
        };
        let pending_gap = pending_gap.map(|mut gap| {
            gap.runtime_incarnation_id = Some(self.runtime_incarnation_id);
            gap.audience = Some(EventAudience::Owner);
            gap
        });
        let initial_indicator = after_event_id.is_none().then(|| {
            let mut event = indicator_event(current_indicator);
            event.channel_id = Some(channel);
            event.runtime_incarnation_id = Some(self.runtime_incarnation_id);
            event.audience = Some(EventAudience::Owner);
            event
        });
        drop(history);

        EventSubscription {
            channel,
            initial_indicator,
            replay,
            pending_gap,
            receiver,
            resume_after: latest,
        }
    }

    pub(crate) fn latest_event_id(&self) -> Option<u64> {
        lock_or_recover(&self.history, "event_queue.history")
            .back()
            .and_then(|event| event.event_id)
    }

    #[cfg(test)]
    pub(crate) fn current_indicator(&self) -> IndicatorType {
        aggregate_indicator(
            &lock_or_recover(&self.indicators, "event_queue.indicators"),
            None,
        )
    }

    pub(crate) fn channel_indicator(&self, channel: ConversationChannelId) -> IndicatorType {
        aggregate_indicator(
            &lock_or_recover(&self.indicators, "event_queue.indicators"),
            Some(channel),
        )
    }

    pub(crate) fn begin_indicator(
        &self,
        channel: ConversationChannelId,
        indicator: IndicatorType,
    ) -> Uuid {
        let mut active = lock_or_recover(&self.indicators, "event_queue.indicators");
        let previous = aggregate_indicator(&active, Some(channel));
        let id = Uuid::new_v4();
        active.insert(id, (channel, indicator));
        let current = aggregate_indicator(&active, Some(channel));
        if previous != current {
            self.push_in_channel(channel, indicator_event(current));
        }
        id
    }

    pub(crate) fn finish_indicator(&self, id: Uuid) {
        let mut active = lock_or_recover(&self.indicators, "event_queue.indicators");
        if let Some((channel, _)) = active.remove(&id) {
            self.push_in_channel(
                channel,
                indicator_event(aggregate_indicator(&active, Some(channel))),
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn clear(&self) -> Vec<StreamBundle> {
        let mut history = lock_or_recover(&self.history, "event_queue.history");
        history.drain(..).map(Arc::unwrap_or_clone).collect()
    }
}

fn aggregate_indicator(
    active: &ActiveIndicators,
    channel: Option<ConversationChannelId>,
) -> IndicatorType {
    active
        .values()
        .filter(|(key, _)| channel.is_none_or(|channel| channel == *key))
        .map(|(_, indicator)| *indicator)
        .max_by_key(|indicator| match indicator {
            IndicatorType::Idle => 0,
            IndicatorType::Thinking => 1,
            IndicatorType::Streaming => 2,
        })
        .unwrap_or_default()
}

fn lock_or_recover<'a, T>(
    mutex: &'a Mutex<T>,
    lock_name: &'static str,
) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::error!(lock = lock_name, "mutex poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::util::sse::factories::text_delta_event;
    use crate::util::sse::types::EventType;

    #[tokio::test]
    async fn channel_subscriptions_do_not_receive_other_channel_text_or_replay() {
        let queue = EventQueue::with_incarnation(8, Uuid::new_v4());
        let mac = ConversationChannelId::for_endpoint("mac");
        let body = ConversationChannelId::for_endpoint("body");
        let mut live = queue.subscribe_in_channel(None, body);
        queue.push_in_channel(mac, text_delta_event("mac-message", "private mac", false));
        queue.push_in_channel(body, text_delta_event("body-message", "body reply", false));
        assert_eq!(live.recv().await.unwrap().payload["delta"], "body reply");
        let mut replay = queue.subscribe_in_channel(Some(0), body);
        assert_eq!(replay.recv().await.unwrap().payload["delta"], "body reply");
    }

    #[tokio::test]
    async fn subscribers_receive_the_same_event() {
        let queue = EventQueue::with_incarnation(4, Uuid::new_v4());
        let mut first = queue.subscribe(None);
        let mut second = queue.subscribe(None);

        queue.push(text_delta_event("message", "hello", false));

        let first = first.recv().await.unwrap();
        let second = second.recv().await.unwrap();
        assert_eq!(first.event_id, second.event_id);
        assert_eq!(first.payload["delta"], "hello");
        assert_eq!(second.payload["delta"], "hello");
    }

    #[tokio::test]
    async fn message_events_carry_run_and_turn_correlation() {
        let queue = EventQueue::with_incarnation(4, Uuid::new_v4());
        let turn_id = crate::models::ids::ConversationTurnId::new();
        let message_id = crate::models::ids::message_id_for_turn(turn_id);
        let mut subscription = queue.subscribe(None);
        queue.push(text_delta_event(&message_id, "hello", false));

        let event = subscription.recv().await.unwrap();
        let expected = Uuid::from(turn_id);
        assert_eq!(event.run_id, Some(expected));
        assert_eq!(event.turn_id, Some(expected));
    }

    #[tokio::test]
    async fn cursor_replays_retained_events() {
        let queue = EventQueue::with_incarnation(4, Uuid::new_v4());
        queue.push(text_delta_event("message", "one", false));
        queue.push(text_delta_event("message", "two", false));

        let mut subscription = queue.subscribe(Some(1));
        let event = subscription.recv().await.unwrap();
        assert_eq!(event.event_id, Some(2));
        assert_eq!(event.payload["delta"], "two");
    }

    #[tokio::test]
    async fn expired_cursor_requires_snapshot() {
        let queue = EventQueue::with_incarnation(2, Uuid::new_v4());
        queue.push(text_delta_event("message", "one", false));
        queue.push(text_delta_event("message", "two", false));
        queue.push(text_delta_event("message", "three", false));

        let mut subscription = queue.subscribe(Some(0));
        let event = subscription.recv().await.unwrap();
        assert_eq!(event.event_type, EventType::SnapshotRequired);
        assert_eq!(event.payload["snapshot_required"], true);
        assert_eq!(event.payload["reason"], "history_expired");
    }

    #[tokio::test]
    async fn empty_history_with_cursor_requires_snapshot() {
        let queue = EventQueue::with_incarnation(2, Uuid::new_v4());
        let mut subscription = queue.subscribe(Some(42));

        let event = subscription.recv().await.unwrap();
        assert_eq!(event.event_type, EventType::SnapshotRequired);
        assert_eq!(event.payload["reason"], "history_unavailable");
        assert_eq!(event.payload["resume_after"], serde_json::Value::Null);
    }

    #[test]
    fn concurrent_publication_preserves_sequence_order() {
        let queue = Arc::new(EventQueue::with_incarnation(256, Uuid::new_v4()));
        std::thread::scope(|scope| {
            for producer in 0..8 {
                let queue = Arc::clone(&queue);
                scope.spawn(move || {
                    for _ in 0..16 {
                        queue.push(text_delta_event(&format!("message-{producer}"), "x", false));
                    }
                });
            }
        });

        let events = queue.clear();
        assert_eq!(events.len(), 128);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.event_id)
                .collect::<Vec<_>>(),
            (1..=128).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn lagged_receiver_reports_lag() {
        let queue = EventQueue::with_incarnation(2, Uuid::new_v4());
        let mut subscription = queue.subscribe(None);
        for _ in 0..3 {
            queue.push(text_delta_event("message", "x", false));
        }

        assert!(matches!(
            subscription.recv().await,
            Err(broadcast::error::RecvError::Lagged(1))
        ));
    }
}
