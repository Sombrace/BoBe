use std::sync::Arc;

use super::event_queue::EventQueue;
use super::types::IndicatorType;

pub(crate) struct IndicatorGuard {
    queue: Arc<EventQueue>,
    id: uuid::Uuid,
}

impl IndicatorGuard {
    pub(crate) fn new(
        queue: Arc<EventQueue>,
        channel: crate::models::ids::ConversationChannelId,
        indicator: IndicatorType,
    ) -> Self {
        let id = queue.begin_indicator(channel, indicator);
        Self { queue, id }
    }
}

impl Drop for IndicatorGuard {
    fn drop(&mut self) {
        self.queue.finish_indicator(self.id);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn one_completion_does_not_hide_another_active_turn() {
        let queue = Arc::new(EventQueue::new(4));
        let owner = crate::models::ids::ConversationChannelId::OWNER;
        let other = crate::models::ids::ConversationChannelId::for_endpoint("other");
        let first = IndicatorGuard::new(Arc::clone(&queue), owner, IndicatorType::Streaming);
        let second = IndicatorGuard::new(Arc::clone(&queue), other, IndicatorType::Streaming);
        drop(first);
        assert_eq!(queue.current_indicator(), IndicatorType::Streaming);
        assert_eq!(queue.channel_indicator(owner), IndicatorType::Idle);
        assert_eq!(queue.channel_indicator(other), IndicatorType::Streaming);
        assert_eq!(
            queue
                .subscribe_in_channel(None, owner)
                .take_initial_indicator()
                .unwrap()
                .payload["indicator"],
            "IDLE"
        );
        drop(second);
        assert_eq!(queue.current_indicator(), IndicatorType::Idle);
    }

    #[tokio::test]
    async fn task_abort_resets_indicator() {
        let queue = Arc::new(EventQueue::new(4));
        let task_queue = Arc::clone(&queue);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = IndicatorGuard::new(
                Arc::clone(&task_queue),
                crate::models::ids::ConversationChannelId::OWNER,
                IndicatorType::Thinking,
            );
            assert!(ready_tx.send(()).is_ok());
            std::future::pending::<()>().await;
        });

        ready_rx.await.expect("task should install indicator guard");
        assert_eq!(queue.current_indicator(), IndicatorType::Thinking);

        task.abort();
        let join_error = task.await.expect_err("task should be cancelled");
        assert!(join_error.is_cancelled());
        assert_eq!(queue.current_indicator(), IndicatorType::Idle);
    }
}
