use tracing::{debug, info, warn};

use crate::models::ids::new_turn_id;
use crate::speech::SpeechPlanError;
use crate::speech::protocol::{ClientMessage, ControlAction, ServerMessage, VoicePhase};
use crate::voice::cancel_phrases::is_cancel_phrase;
use crate::voice::capture_in::reset_capture_stream;
use crate::voice::context::VoiceContext;
use crate::voice::protocol_helpers::{send_error, send_json, send_state};
use crate::voice::session::{PlaybackReport, SessionVoiceConfig, VoiceSession};
use crate::voice::transcript_in;
use crate::voice::turn_flow::{abort_active_turn, handle_barge_in, pending_interruption};

pub(crate) async fn handle_control_text(
    text: &str,
    ctx: &VoiceContext,
    session: &mut Option<VoiceSession>,
) -> bool {
    let output = &ctx.output;
    let voice_defaults = &ctx.voice_defaults;
    let parsed: Result<ClientMessage, _> = serde_json::from_str(text);
    match parsed {
        Ok(ClientMessage::Hello {
            session_id,
            voice_id,
            speed,
            speech_offer,
        }) => {
            info!(
                session = %session_id,
                voice_id = ?voice_id,
                speed = ?speed,
                language = %speech_offer.language,
                "voice.hello"
            );
            let speech_plan = match crate::speech::SpeechPlan::resolve_voice_v2(
                &speech_offer,
                voice_defaults,
                voice_id,
                ctx.engines.daemon_stt().map(|engine| engine.descriptor()),
                ctx.engines.supports_server_tts(),
                ctx.networked_runtime,
            ) {
                Ok(plan) => plan,
                Err(SpeechPlanError::InvalidOffer(message)) => {
                    send_error(output, "invalid_speech_offer", &message).await;
                    return false;
                }
                Err(SpeechPlanError::NoSpeechPath(capability)) => {
                    send_error(
                        output,
                        "no_speech_path",
                        &format!("No ready {capability} engine matches the offer"),
                    )
                    .await;
                    return false;
                }
                Err(SpeechPlanError::RemoteAudioConsentRequired) => {
                    send_error(
                        output,
                        "remote_audio_consent_required",
                        "Daemon STT requires explicit microphone-uplink consent",
                    )
                    .await;
                    return false;
                }
            };
            let cfg = SessionVoiceConfig::new(speed, speech_plan.clone(), voice_defaults);
            let mut s = VoiceSession::new(session_id, cfg);
            if speech_plan.stt_site == crate::speech::plan::SpeechSite::Daemon {
                let Some(stt) = ctx.engines.daemon_stt() else {
                    send_error(output, "engines_unavailable", "Daemon STT is unavailable").await;
                    return false;
                };
                let stream_id = rand::random::<std::num::NonZeroU32>().get();
                let turn_id = new_turn_id();
                let stt_stream = match stt.create_stream(true) {
                    Ok(stream) => stream,
                    Err(error) => {
                        send_error(output, "stt_stream_failed", &error).await;
                        return false;
                    }
                };
                s.capture = Some(crate::voice::session::VoiceCaptureState {
                    stream_id,
                    next_sequence: 0,
                    credit_frames: 25,
                    turn_id,
                    stt: stt_stream,
                });
            }
            let initial_turn = new_turn_id();
            if let Some(prev) = session.as_mut() {
                abort_active_turn(prev, ctx, 0, "rehello").await;
            }
            *session = Some(s);
            send_json(
                output,
                &ServerMessage::SpeechPlan {
                    plan: speech_plan.clone(),
                },
            )
            .await;
            if let Some(capture) = session
                .as_ref()
                .and_then(|session| session.capture.as_ref())
            {
                send_json(
                    output,
                    &ServerMessage::CaptureCredit {
                        stream_id: capture.stream_id,
                        credit_frames: capture.credit_frames,
                    },
                )
                .await;
            }
            send_state(output, VoicePhase::Listening, &initial_turn).await;
            true
        }
        Ok(ClientMessage::TtsPlaybackStarted {
            turn_id,
            synthesis_ms,
        }) => {
            let current = session
                .as_mut()
                .is_some_and(|session| session.note_playback_started(&turn_id));
            debug!(turn_id = %turn_id, synthesis_ms, current, "voice.client_tts_playback_started");
            true
        }
        Ok(ClientMessage::TtsPlaybackComplete { turn_id }) => {
            match session
                .as_mut()
                .and_then(|session| session.report_playback_complete(&turn_id))
            {
                Some(PlaybackReport::Heard) => {
                    debug!(turn_id = %turn_id, "voice.playback_complete");
                }
                Some(PlaybackReport::Unheard) => {
                    warn!(turn_id = %turn_id, "voice.playback_complete_without_start");
                }
                None => {
                    debug!(turn_id = %turn_id, "voice.stale_playback_complete_ignored");
                }
            }
            true
        }
        Ok(ClientMessage::BargeIn {
            ts_ms,
            playback_ms_played,
            partial_text,
            playhead_chunk,
            turn_id,
        }) => {
            info!(ts_ms, playback_ms_played, "voice.barge_in_received");
            let Some(s) = session.as_mut() else {
                return true;
            };
            if !s.accepts_overlap(turn_id.as_deref()) {
                debug!(turn_id = ?turn_id, "voice.stale_barge_in_ignored");
                return true;
            }
            if let Some(evidence) = partial_text {
                s.last_partial_text = evidence;
            }
            handle_barge_in(ctx, session, playback_ms_played, playhead_chunk).await;
            true
        }
        Ok(ClientMessage::PlaybackAck {
            chunk_id,
            played_ms,
            turn_id,
        }) => {
            let current = session
                .as_mut()
                .is_some_and(|session| session.note_playback_ack(turn_id.as_deref(), played_ms));
            debug!(chunk_id, played_ms, current, "voice.playback_ack");
            true
        }
        Ok(ClientMessage::Control { action }) => {
            handle_control_action(action, ctx, session).await;
            true
        }
        Ok(ClientMessage::TranscriptPartial { turn_id, text }) => {
            handle_transcript_partial(turn_id, text, ctx, session).await;
            true
        }
        Ok(ClientMessage::TranscriptFinal { turn_id, text }) => {
            transcript_in::dispatch_from_control(turn_id, text, ctx, session).await;
            true
        }
        Err(e) => {
            warn!(error = %e, "voice.invalid_json");
            send_error(output, "invalid_json", &format!("{e}")).await;
            !serde_json::from_str::<serde_json::Value>(text)
                .is_ok_and(|value| value["type"] == "hello")
        }
    }
}

async fn handle_transcript_partial(
    turn_id: String,
    text: String,
    ctx: &VoiceContext,
    session: &mut Option<VoiceSession>,
) {
    let Some(s) = session.as_mut() else { return };
    if s.muted {
        debug!(turn_id = %turn_id, "voice.transcript_partial_dropped_muted");
        return;
    }
    if !s.accepts_partial(&turn_id) {
        debug!(turn_id = %turn_id, "voice.stale_transcript_partial_ignored");
        return;
    }
    s.last_partial_text = text;
    if s.current_turn.is_some() && is_cancel_phrase(&s.last_partial_text) {
        info!(
            turn_id = %turn_id,
            partial_chars = s.last_partial_text.chars().count(),
            "voice.cancel_phrase_detected"
        );
        let keep_ms = s.last_acked_played_ms;
        abort_active_turn(s, ctx, keep_ms, "cancel_phrase").await;
        return;
    }
    if let Some(keep_ms) = pending_interruption(s, &s.last_partial_text) {
        info!(turn_id = %turn_id, "voice.barge_in_confirmed_by_transcript");
        abort_active_turn(s, ctx, keep_ms, "barge_in_transcript").await;
    }
}

fn abort_acknowledgement(session_id: &str) -> ServerMessage {
    ServerMessage::State {
        phase: VoicePhase::Listening,
        turn_id: session_id.to_owned(),
    }
}

pub(crate) async fn handle_control_action(
    action: ControlAction,
    ctx: &VoiceContext,
    session: &mut Option<VoiceSession>,
) {
    let output = &ctx.output;
    match action {
        ControlAction::Abort => {
            info!("voice.control.abort");
            if let Some(s) = session.as_mut() {
                abort_active_turn(s, ctx, 0, "control_abort").await;
                s.last_partial_text.clear();
                send_json(output, &abort_acknowledgement(&s.session_id)).await;
            }
        }
        ControlAction::Mute => {
            if let Some(s) = session.as_mut() {
                s.muted = true;
                s.last_partial_text.clear();
                if s.capture.is_some() && !reset_capture_stream(ctx, session) {
                    send_error(
                        output,
                        "stt_failed",
                        "Could not reset speech recognition while muting",
                    )
                    .await;
                }
                if let Some(s) = session.as_ref() {
                    info!(session = %s.session_id, "voice.control.mute");
                }
            }
        }
        ControlAction::Unmute => {
            if let Some(s) = session.as_mut() {
                s.muted = false;
                info!(session = %s.session_id, "voice.control.unmute");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::abort_acknowledgement;

    #[test]
    fn every_abort_is_acknowledged_by_the_session_listening_again() {
        assert_eq!(
            serde_json::to_value(abort_acknowledgement("voice-1700000000")).unwrap(),
            serde_json::json!({"type": "state", "phase": "listening", "turn_id": "voice-1700000000"})
        );
    }
}
