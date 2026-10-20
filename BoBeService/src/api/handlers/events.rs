use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use tokio::sync::broadcast;

use crate::api::principal::Principal;
use crate::app_state::AppState;
use crate::util::sse::body_requests::BodyActions;
use crate::util::sse::factories::snapshot_required_event;
use crate::util::sse::types::{EventAudience, EventType, StreamBundle};

pub(crate) async fn stream_events(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let after_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let client_incarnation = headers
        .get("bobe-runtime-incarnation")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<uuid::Uuid>().ok());
    let runtime_incarnation_id = state.infra.identity.runtime_incarnation_id;
    let incarnation_changed = client_incarnation.is_some_and(|id| id != runtime_incarnation_id);
    let mut subscription = state.infra.event_queue.subscribe_in_channel(
        (!incarnation_changed).then_some(after_event_id).flatten(),
        principal.conversation_channel(),
    );
    let initial_indicator = subscription.take_initial_indicator();
    let subscribe_resume_after = subscription.resume_after();
    let actions = BodyActions::from_header(
        headers
            .get("bobe-body-actions")
            .and_then(|value| value.to_str().ok()),
    );
    let (connection_lease, mut body_requests) =
        state.infra.connection_manager.connect(&principal, actions);
    tracing::info!(
        connection_id = connection_lease.id(),
        after_event_id,
        "sse.connected"
    );

    let initial_event = if incarnation_changed {
        let mut gap = snapshot_required_event(
            after_event_id.map(|value| value.saturating_add(1)),
            None,
            subscribe_resume_after,
            "runtime_incarnation_changed",
        );
        gap.runtime_incarnation_id = Some(runtime_incarnation_id);
        gap.audience = Some(EventAudience::Owner);
        Some(gap)
    } else if after_event_id.is_none() {
        initial_indicator
    } else {
        None
    };

    let event_stream = async_stream::stream! {
        let connection_lease = connection_lease;
        if let Some(event) = initial_event {
            let terminal = event.event_type == EventType::SnapshotRequired;
            yield Ok(to_sse_event(&event));
            if terminal {
                return;
            }
        }
        loop {
            tokio::select! {
                biased;
                () = connection_lease.cancelled() => {
                    tracing::info!(connection_id = connection_lease.id(), "sse.revoked");
                    break;
                }
                Some(request) = body_requests.recv() => {
                    yield Ok(to_sse_event(&request));
                }
                received = subscription.recv() => match received {
                    Ok(bundle) => {
                        let terminal = bundle.event_type == EventType::SnapshotRequired;
                        yield Ok(to_sse_event(&bundle));
                        if terminal {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(connection_id = connection_lease.id(), missed, "sse.slow_consumer");
                        let mut gap = snapshot_required_event(
                            None,
                            None,
                            state.infra.event_queue.latest_event_id(),
                            "slow_consumer",
                        );
                        gap.runtime_incarnation_id = Some(runtime_incarnation_id);
                        gap.audience = Some(EventAudience::Owner);
                        yield Ok(to_sse_event(&gap));
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    };

    Sse::new(event_stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn to_sse_event(bundle: &StreamBundle) -> Event {
    let event = Event::default()
        .event("message")
        .json_data(bundle)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "sse.serialize_event_failed");
            Event::default().event("message").data("")
        });
    if let Some(event_id) = bundle.event_id {
        event.id(event_id.to_string())
    } else {
        event
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use crate::util::sse::factories::text_delta_event;

    use super::*;

    #[tokio::test]
    async fn sse_event_carries_the_broker_cursor() {
        let mut bundle = text_delta_event("message", "hello", false);
        bundle.event_id = Some(42);
        let stream = futures::stream::iter([Ok::<_, Infallible>(to_sse_event(&bundle))]);
        let response = axum::response::IntoResponse::into_response(Sse::new(stream));
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("SSE body");
        let text = std::str::from_utf8(&body).expect("UTF-8 SSE");
        assert!(text.lines().any(|line| line == "id: 42"));
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .expect("event data");
        let decoded: StreamBundle = serde_json::from_str(data).expect("event JSON");
        assert_eq!(decoded.event_id, Some(42));
        assert_eq!(decoded.message_id, "message");
    }
}
