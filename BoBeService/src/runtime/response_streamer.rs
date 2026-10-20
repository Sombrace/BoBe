use std::pin::Pin;
use std::time::{Duration, Instant};

use futures::{Stream, StreamExt};
use tracing::{debug, error, info, warn};

use crate::constants::MILLIS_PER_SECOND;
use crate::models::ids::new_message_id;
use crate::provider::ChatDelta;
use crate::util::sse::event_queue::EventQueue;
use crate::util::sse::factories::{
    end_of_turn_event, error_event, text_delta_event, tool_call_complete_event,
    tool_call_start_event,
};

const STALL_THRESHOLD: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamDelivery {
    Sse,
    SseWithLiveObserver,
    ChannelSse {
        channel: crate::models::ids::ConversationChannelId,
        live_observer: bool,
    },
    EndpointOnly,
}

impl StreamDelivery {
    pub(crate) fn in_channel(self, channel: crate::models::ids::ConversationChannelId) -> Self {
        if self.is_endpoint_only() {
            self
        } else {
            Self::ChannelSse {
                channel,
                live_observer: self.has_irreversible_observer(),
            }
        }
    }

    pub(crate) fn push(self, queue: &EventQueue, event: crate::util::sse::types::StreamBundle) {
        match self {
            Self::ChannelSse { channel, .. } => queue.push_in_channel(channel, event),
            Self::Sse | Self::SseWithLiveObserver => {
                queue.push_in_channel(crate::models::ids::ConversationChannelId::OWNER, event);
            }
            Self::EndpointOnly => {}
        }
    }

    pub(crate) const fn emits_sse(self) -> bool {
        matches!(
            self,
            Self::Sse | Self::SseWithLiveObserver | Self::ChannelSse { .. }
        )
    }

    pub(crate) const fn is_endpoint_only(self) -> bool {
        matches!(self, Self::EndpointOnly)
    }

    pub(crate) const fn has_irreversible_observer(self) -> bool {
        matches!(
            self,
            Self::SseWithLiveObserver
                | Self::EndpointOnly
                | Self::ChannelSse {
                    live_observer: true,
                    ..
                }
        )
    }
}

#[derive(Debug)]
pub(crate) struct StreamResult {
    message_id: String,
    pub(crate) full_response: String,
    pub(crate) chunk_count: usize,
    pub(crate) duration_ms: f64,
    pub(crate) success: bool,
    pub(crate) first_token_ms: Option<f64>,
}

impl StreamResult {
    pub(crate) fn emit_terminal(&self, event_queue: &EventQueue, delivery: StreamDelivery) {
        if delivery.emits_sse() {
            delivery.push(
                event_queue,
                end_of_turn_event(&self.message_id, &self.full_response),
            );
        }
    }

    pub(crate) fn chunks_per_sec(&self) -> f64 {
        if self.duration_ms > 0.0 {
            self.chunk_count as f64 / (self.duration_ms / MILLIS_PER_SECOND)
        } else {
            0.0
        }
    }
}

#[derive(Default)]
struct ResponseMessages {
    messages: Vec<(String, String)>,
}

enum ResponseUpdate {
    Append(String),
    Replace(String),
}

impl ResponseMessages {
    fn index(&mut self, id: &str) -> usize {
        if let Some(index) = self
            .messages
            .iter()
            .position(|(existing, _)| existing == id)
        {
            return index;
        }
        self.messages.push((id.to_owned(), String::new()));
        self.messages.len() - 1
    }

    fn body(&self) -> String {
        self.messages
            .iter()
            .map(|(_, body)| body.as_str())
            .filter(|body| !body.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn append(&mut self, id: &str, text: &str) -> ResponseUpdate {
        let index = self.index(id);
        let separator = self.messages[index].1.is_empty()
            && !text.is_empty()
            && self.messages[..index]
                .iter()
                .any(|(_, body)| !body.is_empty());
        self.messages[index].1.push_str(text);
        if self.messages[index + 1..]
            .iter()
            .any(|(_, body)| !body.is_empty())
        {
            return ResponseUpdate::Replace(self.body());
        }
        ResponseUpdate::Append(if separator {
            format!("\n\n{text}")
        } else {
            text.to_owned()
        })
    }

    fn complete(&mut self, id: &str, content: String) -> ResponseUpdate {
        let index = self.index(id);
        if let Some(suffix) = content.strip_prefix(&self.messages[index].1) {
            return self.append(id, suffix);
        }
        self.messages[index].1 = content;
        ResponseUpdate::Replace(self.body())
    }
}

struct StreamAccumulator {
    msg_id: String,
    start_time: Instant,
    sequence: usize,
    full_response: String,
    success: bool,
    first_token_time: Option<Instant>,
}

impl StreamAccumulator {
    fn new(msg_id: Option<&str>) -> Self {
        Self {
            msg_id: msg_id.map_or_else(new_message_id, str::to_owned),
            start_time: Instant::now(),
            sequence: 0,
            full_response: String::new(),
            success: true,
            first_token_time: None,
        }
    }

    fn msg_id(&self) -> &str {
        &self.msg_id
    }

    fn mark_failed(&mut self) {
        self.success = false;
    }

    async fn deliver_update<F>(
        &mut self,
        update: ResponseUpdate,
        event_queue: &EventQueue,
        delivery: StreamDelivery,
        on_text_delta: &mut F,
    ) where
        F: AsyncFnMut(String) + Send,
    {
        let suffix = match update {
            ResponseUpdate::Append(suffix) => suffix,
            ResponseUpdate::Replace(body) => {
                if let Some(suffix) = body.strip_prefix(&self.full_response) {
                    suffix.to_owned()
                } else {
                    if delivery.has_irreversible_observer() {
                        self.mark_failed();
                        if delivery.emits_sse() {
                            delivery.push(
                                event_queue,
                                error_event(
                                    self.msg_id(),
                                    "RESPONSE_DIVERGED",
                                    "The assistant response changed after delivery began",
                                    true,
                                ),
                            );
                        }
                    } else {
                        self.full_response = body;
                    }
                    return;
                }
            }
        };
        if !suffix.is_empty() {
            self.first_token_time.get_or_insert_with(Instant::now);
            on_text_delta(suffix.clone()).await;
            if delivery.emits_sse() {
                delivery.push(event_queue, text_delta_event(self.msg_id(), &suffix, false));
            }
            self.full_response.push_str(&suffix);
            self.sequence += 1;
        }
    }

    fn finish(self) -> StreamResult {
        let duration_ms = self.start_time.elapsed().as_secs_f64() * MILLIS_PER_SECOND;
        let first_token_ms = self
            .first_token_time
            .map(|t| (t - self.start_time).as_secs_f64() * MILLIS_PER_SECOND);

        StreamResult {
            message_id: self.msg_id,
            full_response: self.full_response,
            chunk_count: self.sequence,
            duration_ms,
            success: self.success,
            first_token_ms,
        }
    }
}

pub(crate) async fn stream_chat_delta_response<F>(
    stream: Pin<Box<dyn Stream<Item = ChatDelta> + Send>>,
    event_queue: &EventQueue,
    msg_id: Option<&str>,
    delivery: StreamDelivery,
    on_text_delta: F,
) -> StreamResult
where
    F: AsyncFnMut(String) + Send,
{
    stream_with_deadline(
        stream,
        event_queue,
        msg_id,
        delivery,
        on_text_delta,
        crate::provider::WorkerClass::Chat.turn_timeout(),
    )
    .await
}

async fn stream_with_deadline<F>(
    mut stream: Pin<Box<dyn Stream<Item = ChatDelta> + Send>>,
    event_queue: &EventQueue,
    msg_id: Option<&str>,
    delivery: StreamDelivery,
    mut on_text_delta: F,
    timeout: Duration,
) -> StreamResult
where
    F: AsyncFnMut(String) + Send,
{
    let mut state = StreamAccumulator::new(msg_id);
    let mut messages = ResponseMessages::default();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_token_at: Option<Instant> = None;
    let mut stall_count: u32 = 0;

    loop {
        let delta = match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(delta)) => delta,
            Ok(None) => {
                state.mark_failed();
                if delivery.emits_sse() {
                    delivery.push(
                        event_queue,
                        error_event(
                            state.msg_id(),
                            "CHAT_STREAM_CLOSED",
                            "Assistant stream closed before completing the turn",
                            true,
                        ),
                    );
                }
                break;
            }
            Err(_) => {
                state.mark_failed();
                error!(
                    msg_id = state.msg_id(),
                    timeout_ms = timeout.as_millis() as u64,
                    "stream_chat_delta.turn_timeout"
                );
                if delivery.emits_sse() {
                    delivery.push(
                        event_queue,
                        error_event(
                            state.msg_id(),
                            "CHAT_STREAM_TIMEOUT",
                            "Assistant turn exceeded its execution deadline",
                            true,
                        ),
                    );
                }
                break;
            }
        };
        match delta {
            ChatDelta::MessageDelta { message_id, text } => {
                if !text.is_empty() {
                    let now = Instant::now();
                    if let Some(prev) = last_token_at {
                        let gap = now.duration_since(prev);
                        if gap > STALL_THRESHOLD {
                            stall_count = stall_count.saturating_add(1);
                            warn!(
                                msg_id = state.msg_id(),
                                gap_ms = gap.as_millis() as u64,
                                stall_count,
                                "voice.stream.inter_token_stall"
                            );
                        }
                    }
                    last_token_at = Some(now);
                    let body = messages.append(&message_id, &text);
                    state
                        .deliver_update(body, event_queue, delivery, &mut on_text_delta)
                        .await;
                }
            }
            ChatDelta::MessageComplete {
                message_id,
                content,
                output_tokens,
            } => {
                debug!(
                    bytes = content.len(),
                    output_tokens, "stream_chat_delta.message_complete"
                );
                let body = messages.complete(&message_id, content);
                state
                    .deliver_update(body, event_queue, delivery, &mut on_text_delta)
                    .await;
            }
            ChatDelta::ToolStart { id, name } => {
                info!(tool = %name, "tool_call.start");
                if delivery.emits_sse() {
                    delivery.push(
                        event_queue,
                        tool_call_start_event(state.msg_id(), &name, &id),
                    );
                }
            }
            ChatDelta::ToolComplete { id, name, success } => {
                info!(tool = %name, success, "tool_call.complete");
                if delivery.emits_sse() {
                    delivery.push(
                        event_queue,
                        tool_call_complete_event(state.msg_id(), &name, &id, success),
                    );
                }
            }
            ChatDelta::Error { message: msg } => {
                state.mark_failed();
                error!(error = %msg, chunks = state.sequence, "stream_chat_delta.error");
                if delivery.emits_sse() {
                    delivery.push(
                        event_queue,
                        error_event(state.msg_id(), "CHAT_ERROR", &msg, true),
                    );
                }
                break;
            }
            ChatDelta::Done => {
                debug!(chunks = state.sequence, "stream_chat_delta.done");
                break;
            }
        }
        if !state.success {
            break;
        }
    }

    state.finish()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use futures::stream;

    use super::*;

    #[tokio::test]
    async fn tool_turn_preserves_distinct_messages_on_every_surface() {
        for delivery in [
            StreamDelivery::Sse,
            StreamDelivery::SseWithLiveObserver,
            StreamDelivery::EndpointOnly,
        ] {
            let events = vec![
                ChatDelta::MessageDelta {
                    message_id: "before".into(),
                    text: "Checking.".into(),
                },
                ChatDelta::MessageComplete {
                    message_id: "before".into(),
                    content: "Checking.".into(),
                    output_tokens: 0,
                },
                ChatDelta::ToolStart {
                    id: "tool".into(),
                    name: "bash".into(),
                },
                ChatDelta::MessageComplete {
                    message_id: "tool-only".into(),
                    content: String::new(),
                    output_tokens: 0,
                },
                ChatDelta::ToolComplete {
                    id: "tool".into(),
                    name: "bash".into(),
                    success: true,
                },
                ChatDelta::MessageDelta {
                    message_id: "after".into(),
                    text: "All".into(),
                },
                ChatDelta::MessageComplete {
                    message_id: "after".into(),
                    content: "All done.".into(),
                    output_tokens: 0,
                },
                ChatDelta::MessageComplete {
                    message_id: "before".into(),
                    content: "Checking.".into(),
                    output_tokens: 0,
                },
                ChatDelta::Done,
            ];
            let observed = Arc::new(Mutex::new(String::new()));
            let sink = Arc::clone(&observed);
            let queue = EventQueue::new(32);
            let result = stream_chat_delta_response(
                Box::pin(stream::iter(events)),
                &queue,
                Some("turn"),
                delivery,
                async move |text| {
                    sink.lock().expect("observer").push_str(&text);
                },
            )
            .await;
            assert!(result.success);
            assert_eq!(result.full_response, "Checking.\n\nAll done.");
            assert_eq!(*observed.lock().expect("observer"), result.full_response);
            assert!(
                queue
                    .clear()
                    .iter()
                    .all(|event| event.payload["code"] != "RESPONSE_DIVERGED")
            );
        }
    }

    #[tokio::test]
    async fn same_message_correction_preserves_other_messages() {
        let events = vec![
            ChatDelta::MessageComplete {
                message_id: "before".into(),
                content: "Checking.".into(),
                output_tokens: 0,
            },
            ChatDelta::MessageDelta {
                message_id: "after".into(),
                text: "draft".into(),
            },
            ChatDelta::MessageComplete {
                message_id: "after".into(),
                content: "final".into(),
                output_tokens: 0,
            },
            ChatDelta::Done,
        ];
        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &EventQueue::new(16),
            None,
            StreamDelivery::Sse,
            async |_| {},
        )
        .await;
        assert!(result.success);
        assert_eq!(result.full_response, "Checking.\n\nfinal");
    }

    #[tokio::test]
    async fn quiet_tool_waits_within_whole_turn_deadline() {
        let events = async_stream::stream! {
            yield ChatDelta::ToolStart { id: "tool".into(), name: "bash".into() };
            tokio::time::sleep(Duration::from_millis(40)).await;
            yield ChatDelta::ToolComplete { id: "tool".into(), name: "bash".into(), success: true };
            yield ChatDelta::Done;
        };
        let result = stream_with_deadline(
            Box::pin(events),
            &EventQueue::new(16),
            None,
            StreamDelivery::Sse,
            async |_| {},
            Duration::from_secs(1),
        )
        .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn whole_turn_deadline_is_not_extended_by_tool_events() {
        let events = async_stream::stream! {
            loop {
                yield ChatDelta::ToolStart { id: "tool".into(), name: "bash".into() };
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        let queue = EventQueue::new(64);
        let result = stream_with_deadline(
            Box::pin(events),
            &queue,
            None,
            StreamDelivery::Sse,
            async |_| {},
            Duration::from_millis(20),
        )
        .await;
        assert!(!result.success);
        assert!(
            queue
                .clear()
                .iter()
                .any(|event| event.payload["code"] == "CHAT_STREAM_TIMEOUT")
        );
    }

    #[tokio::test]
    async fn stream_closing_without_done_is_incomplete() {
        let result = stream_chat_delta_response(
            Box::pin(stream::empty()),
            &EventQueue::new(16),
            None,
            StreamDelivery::EndpointOnly,
            async |_| {},
        )
        .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn terminal_content_repairs_lagged_suffix() {
        let events = vec![
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: "Hel".into(),
            },
            ChatDelta::MessageComplete {
                message_id: "m1".into(),
                content: "Hello".into(),
                output_tokens: 1,
            },
            ChatDelta::Done,
        ];
        let observed = Arc::new(Mutex::new(String::new()));
        let observed_for_callback = Arc::clone(&observed);
        let queue = EventQueue::new(16);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("msg"),
            StreamDelivery::Sse,
            async move |delta| {
                observed_for_callback
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push_str(&delta);
            },
        )
        .await;

        assert_eq!(result.full_response, "Hello");
        result.emit_terminal(&queue, StreamDelivery::Sse);
        let terminal = queue.clear().pop().expect("terminal event should exist");
        assert_eq!(terminal.payload["done"], true);
        assert_eq!(terminal.payload["delta"], "Hello");
        assert_eq!(
            *observed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            "Hello"
        );
    }

    #[tokio::test]
    async fn endpoint_stream_fails_when_terminal_content_rewrites_observed_text() {
        let events = vec![
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: "Wrong answer.".into(),
            },
            ChatDelta::MessageComplete {
                message_id: "m1".into(),
                content: "Correct answer.".into(),
                output_tokens: 2,
            },
            ChatDelta::Done,
        ];
        let queue = EventQueue::new(16);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("msg"),
            StreamDelivery::EndpointOnly,
            async |_| {},
        )
        .await;

        assert_eq!(result.full_response, "Wrong answer.");
        assert!(!result.success);
    }

    #[tokio::test]
    async fn sse_voice_observer_fails_on_terminal_rewrite() {
        let events = vec![
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: "Spoken answer.".into(),
            },
            ChatDelta::MessageComplete {
                message_id: "m1".into(),
                content: "Persisted answer.".into(),
                output_tokens: 2,
            },
            ChatDelta::Done,
        ];
        let queue = EventQueue::new(16);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("msg"),
            StreamDelivery::SseWithLiveObserver,
            async |_| {},
        )
        .await;

        assert_eq!(result.full_response, "Spoken answer.");
        assert!(!result.success);
        let events = queue.clear();
        assert!(events.iter().any(|event| {
            event.payload["code"] == "RESPONSE_DIVERGED" && event.payload["recoverable"] == true
        }));
    }

    #[tokio::test]
    async fn terminal_event_survives_queue_overflow_with_full_response() {
        let events = vec![
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: "first".into(),
            },
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: " second".into(),
            },
            ChatDelta::Done,
        ];
        let queue = EventQueue::new(1);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("msg"),
            StreamDelivery::Sse,
            async |_| {},
        )
        .await;

        result.emit_terminal(&queue, StreamDelivery::Sse);
        let queued = queue.clear();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].payload["done"], true);
        assert_eq!(queued[0].payload["delta"], "first second");
        assert_eq!(result.full_response, "first second");
    }

    #[tokio::test]
    async fn terminal_content_is_authoritative_when_deltas_diverge() {
        let events = vec![
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: "draft".into(),
            },
            ChatDelta::MessageComplete {
                message_id: "m1".into(),
                content: "final".into(),
                output_tokens: 1,
            },
            ChatDelta::Done,
        ];
        let queue = EventQueue::new(16);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("msg"),
            StreamDelivery::Sse,
            async |_| {},
        )
        .await;

        assert_eq!(result.full_response, "final");
    }

    #[tokio::test]
    async fn endpoint_only_delivery_never_enters_the_sse_queue() {
        let events = vec![
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: "private".into(),
            },
            ChatDelta::ToolStart {
                id: "tool-1".into(),
                name: "read_file".into(),
            },
            ChatDelta::ToolComplete {
                id: "tool-1".into(),
                name: "read_file".into(),
                success: true,
            },
            ChatDelta::Done,
        ];
        let queue = EventQueue::new(16);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("body-turn"),
            StreamDelivery::EndpointOnly,
            async |_| {},
        )
        .await;

        assert_eq!(result.full_response, "private");
        result.emit_terminal(&queue, StreamDelivery::EndpointOnly);
        assert!(queue.clear().is_empty());
    }

    #[tokio::test]
    async fn endpoint_only_failure_is_preserved_without_leaking_error_content() {
        let events = vec![
            ChatDelta::Error {
                message: "private failure".into(),
            },
            ChatDelta::Done,
        ];
        let queue = EventQueue::new(16);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("body-turn"),
            StreamDelivery::EndpointOnly,
            async |_| {},
        )
        .await;

        assert!(!result.success);
        assert!(queue.clear().is_empty());
    }

    #[tokio::test]
    async fn caller_controls_terminal_publication() {
        let events = vec![
            ChatDelta::MessageDelta {
                message_id: "m1".into(),
                text: "durable".into(),
            },
            ChatDelta::Done,
        ];
        let queue = EventQueue::new(16);

        let result = stream_chat_delta_response(
            Box::pin(stream::iter(events)),
            &queue,
            Some("msg"),
            StreamDelivery::Sse,
            async |_| {},
        )
        .await;

        assert!(
            queue
                .clear()
                .iter()
                .all(|event| event.payload["done"] == false)
        );

        result.emit_terminal(&queue, StreamDelivery::Sse);
        let terminal = queue.clear().pop().expect("terminal event should exist");
        assert_eq!(terminal.payload["done"], true);
        assert_eq!(terminal.payload["delta"], "durable");
    }
}
