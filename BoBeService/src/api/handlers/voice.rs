use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::api::middleware::AllowedOrigins;
use crate::api::principal::Principal;
use crate::app_state::AppState;
use crate::runtime::response_streamer::StreamDelivery;
use crate::speech::protocol::VoicePhase;
use crate::voice::context::VoiceContext;
use crate::voice::control::handle_control_text;
use crate::voice::engines::VoiceEngines;
use crate::voice::output::{VoiceOutput, VoiceOutputFrame};
use crate::voice::protocol_helpers::{close_with_error, send_error, send_state};
use crate::voice::session::{VoiceDefaults, VoiceSession};

const OUTBOUND_CHANNEL_CAPACITY: usize = 64;
const KEEPALIVE_PING_INTERVAL: Duration = Duration::from_secs(25);
const KEEPALIVE_STALE_TIMEOUT: Duration = Duration::from_mins(1);
const WS_SEND_TIMEOUT: Duration = Duration::from_secs(2);

fn completion_matches_current_turn(current_turn_id: Option<&str>, completed_turn_id: &str) -> bool {
    current_turn_id.is_some_and(|turn_id| turn_id == completed_turn_id)
}

pub(super) struct VoiceWsPermit(Arc<AtomicBool>);

impl Drop for VoiceWsPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) async fn acquire_permit(
    socket: WebSocket,
    state: &AppState,
) -> Option<(WebSocket, VoiceWsPermit)> {
    if state
        .voice
        .voice_ws_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        warn!("voice.ws_concurrent_rejected");
        close_with_error(
            socket,
            "voice_busy",
            "another voice session is already active",
        )
        .await;
        return None;
    }
    Some((
        socket,
        VoiceWsPermit(Arc::clone(&state.voice.voice_ws_active)),
    ))
}

struct SessionTeardown {
    keepalive: tokio::task::JoinHandle<()>,
    session: Option<VoiceSession>,
    writer: tokio::task::JoinHandle<()>,
    ctx: VoiceContext,
    output: VoiceOutput,
}

const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

async fn teardown_session(td: SessionTeardown) {
    let SessionTeardown {
        keepalive,
        mut session,
        writer,
        ctx,
        output,
    } = td;

    keepalive.abort();

    if let Some(s) = session.as_mut() {
        if let Some(turn) = s.current_turn.take() {
            info!(turn_id = %turn.turn_id, "voice.disconnect_aborting_turn");
            turn.join.abort();
            drop(turn.join.await);
        }
        if tokio::time::timeout(
            WRITER_DRAIN_TIMEOUT,
            send_state(&output, VoicePhase::Idle, &s.session_id),
        )
        .await
        .is_err()
        {
            debug!(session = %s.session_id, "voice.closed_notice_timed_out");
        }
        info!(session = %s.session_id, "voice.disconnect");
    }

    drop(ctx);
    drop(output);

    let mut writer = writer;
    match tokio::time::timeout(WRITER_DRAIN_TIMEOUT, &mut writer).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) if e.is_panic() => {
            error!(error = %e, "voice.ws_writer_panic");
        }
        Ok(Err(e)) if e.is_cancelled() => {}
        Ok(Err(e)) => warn!(error = %e, "voice.ws_writer_join_failed"),
        Err(_) => {
            warn!("voice.ws_writer_drain_timeout_aborting");
            writer.abort();
            if let Err(e) = writer.await
                && !e.is_cancelled()
            {
                warn!(error = %e, "voice.ws_writer_abort_join_failed");
            }
        }
    }
}

pub(super) fn spawn_writer(
    mut ws_tx: futures::stream::SplitSink<WebSocket, Message>,
    mut out_rx: mpsc::Receiver<VoiceOutputFrame>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            let msg = match frame {
                VoiceOutputFrame::Control(control) => {
                    let Ok(text) = serde_json::to_string(&control) else {
                        error!("voice.control_encode_failed");
                        continue;
                    };
                    Message::Text(text.into())
                }
                VoiceOutputFrame::Audio(bytes) => Message::Binary(bytes.into()),
                VoiceOutputFrame::Ping => Message::Ping(Vec::new().into()),
            };
            if !matches!(
                tokio::time::timeout(WS_SEND_TIMEOUT, ws_tx.send(msg)).await,
                Ok(Ok(()))
            ) {
                break;
            }
        }
        drop(ws_tx.close().await);
    })
}

pub(super) fn spawn_keepalive(output: VoiceOutput) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(KEEPALIVE_PING_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if !output.ping().await {
                break;
            }
        }
    })
}

pub(crate) async fn voice_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Extension(allowed_origins): Extension<AllowedOrigins>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> Response {
    upgrade_voice(ws, state, allowed_origins, principal, headers, false)
}

pub(super) fn upgrade_voice(
    ws: WebSocketUpgrade,
    state: Arc<AppState>,
    allowed_origins: AllowedOrigins,
    principal: Principal,
    headers: HeaderMap,
    preview: bool,
) -> Response {
    if let Some(origin) = headers.get(header::ORIGIN)
        && !allowed_origins.contains(origin)
    {
        warn!("security.voice_ws_origin_blocked");
        return StatusCode::FORBIDDEN.into_response();
    }

    let ws = ws.protocols([crate::constants::voice_wire::SUBPROTOCOL_V2]);
    if ws.selected_protocol().is_none() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    ws.max_message_size(32_768)
        .max_frame_size(32_768)
        .on_upgrade(async move |socket| {
            if preview {
                super::voice_preview::handle_socket(socket, state, principal).await;
            } else {
                handle_socket(socket, state, principal).await;
            }
        })
        .into_response()
}

#[allow(
    clippy::collapsible_match,
    reason = "outer match has Binary/Close arms that prevent if-let collapse; \
              async call also disallows match guards"
)]
async fn handle_socket(socket: WebSocket, state: Arc<AppState>, principal: Principal) {
    let engines = VoiceEngines::from_state(&state);

    let voice_defaults = VoiceDefaults::from_state(&state);
    if !voice_defaults.enabled {
        warn!("voice.disabled_by_settings");
        close_with_error(socket, "voice_disabled", "voice mode disabled in settings").await;
        return;
    }

    let Some((socket, _ws_permit)) = acquire_permit(socket, &state).await else {
        return;
    };
    let connection_lease = state.infra.connection_manager.connect_voice(&principal);

    let (ws_tx, mut rx) = socket.split();
    let (output, out_rx) = VoiceOutput::channel(OUTBOUND_CHANNEL_CAPACITY);

    let writer = spawn_writer(ws_tx, out_rx);
    let keepalive = spawn_keepalive(output.clone());

    let mut session: Option<VoiceSession> = None;
    let runtime_session = Arc::clone(&state.runtime.runtime_session);

    let (turn_completion_tx, mut turn_completion_rx) = mpsc::channel::<String>(8);

    let networked_runtime = state.config().server.is_networked();
    let ctx = VoiceContext {
        conversation_channel: principal.conversation_channel(),
        networked_runtime,
        output: output.clone(),
        runtime_session: Arc::clone(&runtime_session),
        engines: engines.clone(),
        voice_defaults: voice_defaults.clone(),
        voice_sink: Arc::clone(&state.voice.voice_sink),
        delivery: StreamDelivery::SseWithLiveObserver,
        turn_completion_tx,
    };

    loop {
        tokio::select! {
            biased;
            () = connection_lease.cancelled() => {
                info!(connection_id = connection_lease.id(), "voice.revoked");
                break;
            }
            Some(completed_turn_id) = turn_completion_rx.recv() => {
                let current_turn_id = session
                    .as_ref()
                    .and_then(|session| session.current_turn.as_ref())
                    .map(|turn| turn.turn_id.as_str());
                if completion_matches_current_turn(current_turn_id, &completed_turn_id)
                    && let Some(s) = session.as_mut()
                    && let Some(turn) = s.finish_turn() {
                    debug!(turn_id = %turn.turn_id, "voice.turn_completed_slot_cleared");
                    drop(turn.join.await);
                } else {
                    debug!(turn_id = %completed_turn_id, "voice.stale_turn_completion_ignored");
                }
            }
            next = tokio::time::timeout(KEEPALIVE_STALE_TIMEOUT, rx.next()) => {
                let next = match next {
                    Ok(Some(m)) => m,
                    Ok(None) => break,
                    Err(_) => {
                        warn!("voice.keepalive_recv_timeout_closing");
                        break;
                    }
                };
                let msg = match next {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(error = %e, "voice.ws_recv_error");
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => {
                        if !handle_control_text(text.as_str(), &ctx, &mut session).await {
                            break;
                        }
                    }
                    Message::Binary(bytes) => {
                        let Some(frame) = crate::speech::protocol::decode_voice_capture_frame(&bytes) else {
                            warn!("voice.invalid_capture_frame");
                            send_error(&ctx.output, "capture_protocol_error", "Invalid capture frame").await;
                            break;
                        };
                        if !crate::voice::capture_in::handle_capture_frame(
                            frame,
                            &ctx,
                            &mut session,
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Message::Close(_) => {
                        info!("voice.close_received");
                        break;
                    }
                    Message::Pong(_) | Message::Ping(_) => {
                    }
                }
            }
        }
    }

    teardown_session(SessionTeardown {
        keepalive,
        session,
        writer,
        ctx,
        output,
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_completion_cannot_clear_replacement_turn() {
        assert!(completion_matches_current_turn(Some("turn-a"), "turn-a"));
        assert!(!completion_matches_current_turn(Some("turn-b"), "turn-a"));
        assert!(!completion_matches_current_turn(None, "turn-a"));
    }
}
