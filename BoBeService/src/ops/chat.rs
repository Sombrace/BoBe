#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "interactive CLI chat streams assistant and tool events"
)]

use std::io::Write;
use std::time::Duration;

use futures::StreamExt;
use serde::Serialize;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::ids::{ConversationTurnId, message_id_for_turn};
use crate::ops::status::OpsTarget;
use crate::util::sse::types::{EventType, StreamBundle};

#[derive(Debug, Serialize)]
pub(crate) struct ChatReport {
    pub(crate) request_id: Uuid,
    pub(crate) message_id: String,
    pub(crate) content: String,
    pub(crate) cancelled: bool,
    pub(crate) tool_calls: Vec<ChatToolCall>,
    pub(crate) tool_calls_complete: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) success: Option<bool>,
}

pub(crate) async fn turn(
    target: &OpsTarget,
    prompt: &str,
    timeout: Duration,
    json: bool,
) -> Result<ChatReport, AppError> {
    if prompt.trim().is_empty() {
        return Err(AppError::Validation("chat prompt must not be empty".into()));
    }
    let request_id = Uuid::new_v4();
    let message_id = message_id_for_turn(ConversationTurnId::from(request_id));
    let response = target.stream_get("/events").await?;
    if !response.status().is_success() {
        return Err(AppError::Internal(format!(
            "event subscription failed with HTTP {}",
            response.status()
        )));
    }
    let mut stream = response.bytes_stream();
    let submission = target
        .post_json(
            "/message",
            &serde_json::json!({
                "content": prompt,
                "request_id": request_id,
            }),
        )
        .await?;
    let submission_status = submission.status();
    let submission_body: serde_json::Value = submission.json().await.map_err(|error| {
        AppError::Internal(format!(
            "message submission returned invalid JSON ({submission_status}): {error}"
        ))
    })?;
    if !submission_status.is_success() {
        return Err(AppError::Conflict(
            submission_body
                .get("error")
                .or_else(|| submission_body.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("daemon rejected the message")
                .to_owned(),
        ));
    }
    if submission_body["request_status"] == "completed"
        && let Some(content) = snapshot_content(target, &message_id).await?
    {
        render_recovered_content("", &content, json)?;
        return Ok(ChatReport {
            request_id,
            message_id,
            content,
            cancelled: false,
            tool_calls: Vec::new(),
            tool_calls_complete: false,
        });
    }

    let consume = consume_events(target, &mut stream, &message_id, request_id, json);
    tokio::select! {
        result = tokio::time::timeout(timeout, consume) => match result {
            Ok(Ok(report)) => Ok(report),
            Ok(Err(error)) => {
                drop(cancel(target, request_id).await);
                Err(error)
            }
            Err(_) => {
                cancel(target, request_id).await?;
                Err(AppError::Internal(format!(
                    "chat turn timed out after {} seconds",
                    timeout.as_secs()
                )))
            }
        },
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(AppError::Io)?;
            cancel(target, request_id).await?;
            Ok(ChatReport {
                request_id,
                message_id,
                content: String::new(),
                cancelled: true,
                tool_calls: Vec::new(),
                tool_calls_complete: false,
            })
        }
    }
}

async fn consume_events(
    target: &OpsTarget,
    stream: &mut (impl futures::Stream<Item = Result<axum::body::Bytes, reqwest::Error>> + Unpin),
    message_id: &str,
    request_id: Uuid,
    json: bool,
) -> Result<ChatReport, AppError> {
    let mut buffer = Vec::<u8>::new();
    let mut content = String::new();
    let mut tool_calls = Vec::<ChatToolCall>::new();
    while let Some(chunk) = stream.next().await {
        if let Ok(chunk) = chunk {
            buffer.extend_from_slice(&chunk);
        } else {
            let snapshot = wait_for_snapshot(target, message_id).await?;
            render_recovered_content(&content, &snapshot, json)?;
            return Ok(ChatReport {
                request_id,
                message_id: message_id.to_owned(),
                content: snapshot,
                cancelled: false,
                tool_calls,
                tool_calls_complete: false,
            });
        }
        while let Some(end) = frame_end(&buffer) {
            let frame = buffer.drain(..end).collect::<Vec<_>>();
            drain_frame_separator(&mut buffer);
            let Some(bundle) = parse_frame(&frame)? else {
                continue;
            };
            match bundle.event_type {
                EventType::SnapshotRequired => {
                    let snapshot = wait_for_snapshot(target, message_id).await?;
                    render_recovered_content(&content, &snapshot, json)?;
                    return Ok(ChatReport {
                        request_id,
                        message_id: message_id.to_owned(),
                        content: snapshot,
                        cancelled: false,
                        tool_calls,
                        tool_calls_complete: false,
                    });
                }
                EventType::TextDelta if bundle.message_id == message_id => {
                    let done = bundle.payload["done"].as_bool().unwrap_or(false);
                    let delta = bundle.payload["delta"].as_str().unwrap_or_default();
                    if done {
                        render_recovered_content(&content, delta, json)?;
                        content = delta.to_owned();
                        return Ok(ChatReport {
                            request_id,
                            message_id: message_id.to_owned(),
                            content,
                            cancelled: false,
                            tool_calls,
                            tool_calls_complete: true,
                        });
                    }
                    content.push_str(delta);
                    if !json {
                        print!("{delta}");
                        std::io::stdout().flush()?;
                    }
                }
                EventType::ToolCallStart if bundle.message_id == message_id => {
                    let call = ChatToolCall {
                        id: bundle.payload["tool_call_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        name: bundle.payload["tool_name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        success: None,
                    };
                    if !json {
                        eprintln!("\n[tool] {}", call.name);
                    }
                    tool_calls.push(call);
                }
                EventType::ToolCallComplete if bundle.message_id == message_id => {
                    let id = bundle.payload["tool_call_id"].as_str().unwrap_or_default();
                    let success = bundle.payload["success"].as_bool();
                    if let Some(call) = tool_calls.iter_mut().rev().find(|call| call.id == id) {
                        call.success = success;
                    }
                }
                EventType::Error
                    if bundle.message_id == message_id || bundle.message_id.is_empty() =>
                {
                    let message = bundle.payload["message"]
                        .as_str()
                        .unwrap_or("chat stream failed");
                    return Err(AppError::Internal(message.to_owned()));
                }
                EventType::Indicator
                | EventType::ConversationClosed
                | EventType::ConversationChanged
                | EventType::Error
                | EventType::TextDelta
                | EventType::ToolCallStart
                | EventType::ToolCallComplete
                | EventType::BodyRequest => {}
            }
        }
    }
    let snapshot = wait_for_snapshot(target, message_id).await?;
    render_recovered_content(&content, &snapshot, json)?;
    Ok(ChatReport {
        request_id,
        message_id: message_id.to_owned(),
        content: snapshot,
        cancelled: false,
        tool_calls,
        tool_calls_complete: false,
    })
}

pub(crate) async fn acknowledge(target: &OpsTarget, report: &ChatReport) -> Result<(), AppError> {
    if report.cancelled || report.content.is_empty() {
        return Ok(());
    }
    std::io::stdout().flush()?;
    let response = target
        .post(&format!("/conversation/{}/presented", report.request_id))
        .await?;
    if !response.status().is_success() {
        return Err(AppError::Internal(format!(
            "response was printed but its presentation receipt failed with HTTP {}",
            response.status()
        )));
    }
    Ok(())
}

async fn cancel(target: &OpsTarget, request_id: Uuid) -> Result<(), AppError> {
    let response = target.post(&format!("/runs/{request_id}/cancel")).await?;
    if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::Internal(format!(
            "chat cancellation failed with HTTP {}",
            response.status()
        )));
    }
    Ok(())
}

async fn snapshot_content(
    target: &OpsTarget,
    message_id: &str,
) -> Result<Option<String>, AppError> {
    let response = target.get("/conversation/current").await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let snapshot: serde_json::Value = response.json().await?;
    Ok(snapshot["turns"]
        .as_array()
        .and_then(|turns| {
            turns.iter().find(|turn| {
                turn["message_id"] == message_id && turn["is_complete"].as_bool() == Some(true)
            })
        })
        .and_then(|turn| turn["content"].as_str())
        .map(str::to_owned))
}

async fn wait_for_snapshot(target: &OpsTarget, message_id: &str) -> Result<String, AppError> {
    loop {
        if let Some(content) = snapshot_content(target, message_id).await? {
            return Ok(content);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn render_recovered_content(partial: &str, complete: &str, json: bool) -> Result<(), AppError> {
    if json {
        return Ok(());
    }
    if let Some(suffix) = complete.strip_prefix(partial) {
        print!("{suffix}");
    } else {
        print!("\n{complete}");
    }
    println!();
    std::io::stdout().flush().map_err(AppError::Io)
}

fn frame_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .or_else(|| buffer.windows(4).position(|window| window == b"\r\n\r\n"))
}

fn drain_frame_separator(buffer: &mut Vec<u8>) {
    if buffer.starts_with(b"\r\n\r\n") {
        buffer.drain(..4);
    } else if buffer.starts_with(b"\n\n") {
        buffer.drain(..2);
    }
}

fn parse_frame(frame: &[u8]) -> Result<Option<StreamBundle>, AppError> {
    let frame = String::from_utf8(frame.to_vec())
        .map_err(|error| AppError::Internal(format!("event stream was not UTF-8: {error}")))?;
    let data = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&data)
        .map(Some)
        .map_err(|error| AppError::Internal(format!("invalid event payload: {error}")))
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::extract::State;
    use axum::response::sse::{Event, Sse};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use tokio::sync::broadcast;

    use super::*;

    #[test]
    fn parses_axum_sse_frame() {
        let frame = br#"event: message
data: {"type":"text_delta","message_id":"msg_1","timestamp":"now","description":"text_delta","payload":{"delta":"hi","done":false}}
"#;
        let bundle = parse_frame(frame).unwrap().unwrap();
        assert_eq!(bundle.event_type, EventType::TextDelta);
        assert_eq!(bundle.payload["delta"], "hi");
    }

    #[test]
    fn finds_lf_and_crlf_frame_boundaries() {
        assert_eq!(frame_end(b"data: x\n\nrest"), Some(7));
        assert_eq!(frame_end(b"data: x\r\n\r\nrest"), Some(7));
    }

    #[derive(Clone)]
    struct FixtureState {
        subscribed: Arc<AtomicBool>,
        events: broadcast::Sender<StreamBundle>,
    }

    async fn fixture_events(
        State(state): State<FixtureState>,
    ) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
        state.subscribed.store(true, Ordering::Release);
        let mut receiver = state.events.subscribe();
        let stream = async_stream::stream! {
            let indicator = crate::util::sse::factories::indicator_event(
                crate::util::sse::types::IndicatorType::Idle,
            );
            yield Ok(Event::default().data(serde_json::to_string(&indicator).unwrap()));
            while let Ok(bundle) = receiver.recv().await {
                yield Ok(Event::default().data(serde_json::to_string(&bundle).unwrap()));
            }
        };
        Sse::new(stream)
    }

    async fn fixture_message(
        State(state): State<FixtureState>,
        Json(body): Json<serde_json::Value>,
    ) -> (reqwest::StatusCode, Json<serde_json::Value>) {
        if !state.subscribed.load(Ordering::Acquire) {
            return (
                reqwest::StatusCode::CONFLICT,
                Json(serde_json::json!({"message":"event stream was not subscribed"})),
            );
        }
        let request_id = body["request_id"].as_str().unwrap();
        let turn_id = ConversationTurnId::from(Uuid::parse_str(request_id).unwrap());
        let message_id = message_id_for_turn(turn_id);
        state
            .events
            .send(crate::util::sse::factories::trigger_error_event(
                "capture",
                "unrelated background failure",
                true,
            ))
            .unwrap();
        state
            .events
            .send(crate::util::sse::factories::text_delta_event(
                &message_id,
                "hello",
                false,
            ))
            .unwrap();
        state
            .events
            .send(crate::util::sse::factories::end_of_turn_event(
                &message_id,
                "hello",
            ))
            .unwrap();
        (
            reqwest::StatusCode::OK,
            Json(serde_json::json!({
                "message_id": message_id,
                "request_status": "running",
                "replayed": false,
            })),
        )
    }

    #[tokio::test]
    async fn subscribes_before_submit_and_does_not_duplicate_terminal_body() {
        crate::util::tls::install_crypto_provider();
        let (events, _) = broadcast::channel(8);
        let state = FixtureState {
            subscribed: Arc::new(AtomicBool::new(false)),
            events,
        };
        let app = Router::new()
            .route("/events", get(fixture_events))
            .route("/message", post(fixture_message))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut config = crate::config::Config::default();
        config.server.host = "127.0.0.1".into();
        config.server.port = address.port();
        let target = OpsTarget::resolve(&config, None, None).unwrap();

        let report = turn(&target, "test", Duration::from_secs(2), true)
            .await
            .unwrap();

        assert_eq!(report.content, "hello");
        assert!(report.tool_calls_complete);
        server.abort();
    }

    #[derive(Clone)]
    struct SnapshotState {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    async fn delayed_snapshot(State(state): State<SnapshotState>) -> Json<serde_json::Value> {
        let call = state.calls.fetch_add(1, Ordering::AcqRel);
        Json(if call == 0 {
            serde_json::json!({"conversation_id":"conversation","turns":[]})
        } else {
            serde_json::json!({
                "conversation_id":"conversation",
                "turns":[{
                    "message_id":"msg_recovered",
                    "content":"complete after gap",
                    "is_complete":true
                }]
            })
        })
    }

    #[tokio::test]
    async fn snapshot_gap_recovery_waits_for_durable_completion() {
        crate::util::tls::install_crypto_provider();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let app = Router::new()
            .route("/conversation/current", get(delayed_snapshot))
            .with_state(SnapshotState {
                calls: Arc::clone(&calls),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut config = crate::config::Config::default();
        config.server.host = "127.0.0.1".into();
        config.server.port = address.port();
        let target = OpsTarget::resolve(&config, None, None).unwrap();

        let content = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_snapshot(&target, "msg_recovered"),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(content, "complete after gap");
        assert!(calls.load(Ordering::Acquire) >= 2);
        server.abort();
    }
}
