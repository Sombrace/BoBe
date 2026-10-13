use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use axum::response::Response;
use futures::StreamExt;
use tokio::sync::oneshot;
use tokio_util::task::AbortOnDropHandle;

use crate::api::middleware::AllowedOrigins;
use crate::api::principal::Principal;
use crate::app_state::AppState;
use crate::speech::protocol::{ClientMessage, ControlAction, ServerMessage, VoicePhase};
use crate::voice::engines::VoiceEngines;
use crate::voice::output::VoiceOutput;
use crate::voice::preview::{Preview, PreviewRequest};
use crate::voice::protocol_helpers::{close_with_error, send_error, send_json, send_state};
use crate::voice::session::VoiceDefaults;
use crate::voice::spoken::SpokenScript;

pub(crate) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Extension(origins): Extension<AllowedOrigins>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> Response {
    super::voice::upgrade_voice(ws, state, origins, principal, headers, true)
}

pub(super) async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    principal: Principal,
) {
    let Some((reserved_socket, _permit)) = super::voice::acquire_permit(socket, &state).await
    else {
        return;
    };
    socket = reserved_socket;
    let lease = state.infra.connection_manager.connect_voice(&principal);
    let defaults = VoiceDefaults::from_state(&state);
    if !defaults.enabled {
        close_with_error(socket, "voice_disabled", "Voice is disabled in settings").await;
        return;
    }
    let Some(tts) = VoiceEngines::from_state(&state).tts else {
        close_with_error(
            socket,
            "engines_unavailable",
            "English speech is not ready. Check bobe voice-models status, or run bobe voice-models install to repair it.",
        )
        .await;
        return;
    };
    let first = tokio::select! {
        () = lease.cancelled() => return,
        first = tokio::time::timeout(Duration::from_secs(10), socket.next()) => first,
    };
    let request = if let Ok(Some(Ok(Message::Text(text)))) = first {
        serde_json::from_str::<PreviewRequest>(&text)
    } else {
        close_with_error(
            socket,
            "invalid_preview",
            "Send a voice preview request within ten seconds",
        )
        .await;
        return;
    };
    let prepared = match request.map_err(|e| e.to_string()).and_then(|r| {
        Preview::prepare(r, &defaults).map_err(|error| match error {
            crate::error::AppError::Validation(message) => message,
            error => error.to_string(),
        })
    }) {
        Ok(prepared) => prepared,
        Err(error) => {
            close_with_error(socket, "invalid_preview", &error).await;
            return;
        }
    };
    let admission = match state
        .runtime
        .runtime_session
        .try_begin_user_message(principal.conversation_channel())
    {
        Ok(guard) => guard,
        Err(error) => {
            close_with_error(socket, "conflict", error).await;
            return;
        }
    };
    let turn_cancellation = admission.cancellation();
    let turn_id = format!("preview_{}", uuid::Uuid::new_v4());
    let ready = serde_json::json!({
        "type": "preview_ready", "turn_id": turn_id, "voice_id": prepared.voice,
        "speed": prepared.speed, "sample_rate": tts.sample_rate(),
    });
    if !matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            socket.send(Message::Text(ready.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    ) {
        return;
    }
    let (ws_tx, mut ws_rx) = socket.split();
    let (output, receiver) = VoiceOutput::channel(64);
    let mut writer = AbortOnDropHandle::new(super::voice::spawn_writer(ws_tx, receiver));
    let keepalive = AbortOnDropHandle::new(super::voice::spawn_keepalive(output.clone()));
    send_state(&output, VoicePhase::Speaking, &turn_id).await;
    let (playback_tx, playback_rx) = oneshot::channel();
    let mut playback_tx = Some(playback_tx);
    let script = Arc::new(SpokenScript::default());
    let mut worker = AbortOnDropHandle::new(tokio::spawn(prepared.run(
        tts,
        output.clone(),
        turn_id.clone(),
        script,
        playback_rx,
    )));
    let cancelled = loop {
        tokio::select! {
            biased;
            () = lease.cancelled() => break true,
            () = turn_cancellation.cancelled() => break true,
            message = ws_rx.next() => match message {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(ClientMessage::Control { action: ControlAction::Abort }) => break true,
                    Ok(ClientMessage::BargeIn { partial_text: Some(text), .. })
                        if crate::voice::turn_flow::judge_barge_in(&text) == crate::voice::turn_flow::BargeIn::Interrupt => break true,
                    Ok(ClientMessage::TtsPlaybackComplete { turn_id: completed }) if completed == turn_id => {
                        if let Some(sender) = playback_tx.take() { let _ = sender.send(()); }
                    }
                    Ok(ClientMessage::PlaybackAck { .. } | ClientMessage::TtsPlaybackStarted { .. }) => {},
                    _ => { send_error(&output, "invalid_preview_control", "This connection accepts playback receipts or cancellation").await; break true; }
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break true,
            },
            result = &mut worker => {
                match result {
                    Ok(Ok(())) => {},
                    Ok(Err(error)) => send_error(&output, "tts_failed", &error.to_string()).await,
                    Err(error) => send_error(&output, "tts_failed", &format!("Speech worker failed: {error}")).await,
                }
                break false;
            }
        }
    };
    if cancelled {
        worker.abort();
        drop(worker.await);
        send_json(
            &output,
            &ServerMessage::Truncate {
                turn_id: turn_id.clone(),
                keep_ms: 0,
            },
        )
        .await;
    }
    send_state(&output, VoicePhase::Listening, &turn_id).await;
    drop(keepalive);
    drop(output);
    if tokio::time::timeout(Duration::from_secs(2), &mut writer)
        .await
        .is_err()
    {
        writer.abort();
        drop(writer.await);
    }
}
