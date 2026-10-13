use crate::speech::SttEvent;
use crate::speech::protocol::{ServerMessage, VoiceCaptureFrame};
use crate::voice::context::VoiceContext;
use crate::voice::protocol_helpers::{send_error, send_json};
use crate::voice::session::VoiceSession;
use crate::voice::transcript_in;
use crate::voice::turn_flow::{BargeIn, abort_active_turn, judge_barge_in};

const CREDIT_BATCH: u32 = 25;

pub(crate) async fn handle_capture_frame(
    frame: VoiceCaptureFrame<'_>,
    ctx: &VoiceContext,
    session: &mut Option<VoiceSession>,
) -> bool {
    if frame.flags != 0 {
        send_error(
            &ctx.output,
            "capture_protocol_error",
            "Unsupported capture frame flags",
        )
        .await;
        return false;
    }
    let (events, replenish, stream_id, turn_id) = {
        let Some(active_session) = session.as_mut() else {
            return false;
        };
        let muted = active_session.muted;
        let Some(capture) = active_session.capture.as_mut() else {
            return false;
        };
        if frame.stream_id != capture.stream_id
            || frame.sequence != capture.next_sequence
            || capture.credit_frames == 0
        {
            send_error(
                &ctx.output,
                "capture_protocol_error",
                "Invalid capture frame sequence",
            )
            .await;
            return false;
        }
        capture.next_sequence = capture.next_sequence.wrapping_add(1);
        capture.credit_frames -= 1;
        let events = if muted {
            Vec::new()
        } else {
            match capture.stt.accept_pcm_s16le(frame.payload) {
                Ok(events) => events,
                Err(error) => {
                    send_error(&ctx.output, "stt_failed", &error).await;
                    return false;
                }
            }
        };
        let replenish = capture.credit_frames == 0;
        if replenish {
            capture.credit_frames = CREDIT_BATCH;
        }
        (
            events,
            replenish,
            capture.stream_id,
            capture.turn_id.clone(),
        )
    };

    for event in events {
        match event {
            SttEvent::Partial(text) => {
                if let Some(session) = session.as_mut() {
                    session.last_partial_text.clone_from(&text);
                }
                send_json(
                    &ctx.output,
                    &ServerMessage::SttPartial {
                        turn_id: turn_id.clone(),
                        text,
                    },
                )
                .await;
            }
            SttEvent::Final(text) => {
                let active_turn = session
                    .as_ref()
                    .is_some_and(|session| session.current_turn.is_some());
                if active_turn && judge_barge_in(&text) != BargeIn::Interrupt {
                    if !reset_capture_stream(ctx, session) {
                        return false;
                    }
                    continue;
                }
                send_json(
                    &ctx.output,
                    &ServerMessage::SttFinal {
                        turn_id: turn_id.clone(),
                        text: text.clone(),
                    },
                )
                .await;
                if let Some(active) = session
                    .as_mut()
                    .filter(|session| session.current_turn.is_some())
                {
                    abort_active_turn(active, ctx, 0, "daemon_stt_eou").await;
                }
                transcript_in::dispatch_from_control(turn_id.clone(), text, ctx, session).await;
                if !reset_capture_stream(ctx, session) {
                    return false;
                }
            }
        }
    }
    if replenish {
        send_json(
            &ctx.output,
            &ServerMessage::CaptureCredit {
                stream_id,
                credit_frames: CREDIT_BATCH,
            },
        )
        .await;
    }
    true
}

pub(crate) fn reset_capture_stream(ctx: &VoiceContext, session: &mut Option<VoiceSession>) -> bool {
    let Some(engine) = ctx.engines.daemon_stt() else {
        return false;
    };
    let Ok(stream) = engine.create_stream(true) else {
        return false;
    };
    let Some(capture) = session
        .as_mut()
        .and_then(|session| session.capture.as_mut())
    else {
        return false;
    };
    capture.stt = stream;
    capture.turn_id = crate::models::ids::new_turn_id();
    true
}
