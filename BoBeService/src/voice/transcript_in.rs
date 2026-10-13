use std::sync::Arc;
use std::time::Instant;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::voice::context::VoiceContext;
use crate::voice::run_text_turn::{VoiceTurnAdmission, run_text_turn};
use crate::voice::session::{SessionVoiceConfig, TurnInFlight, VoiceSession};
use crate::voice::spoken::SpokenScript;
use crate::voice::turn_flow::{abort_active_turn, pending_interruption};

pub(crate) async fn dispatch_from_control(
    turn_id: String,
    text: String,
    ctx: &VoiceContext,
    session: &mut Option<VoiceSession>,
) {
    let Some(s) = session.as_mut() else {
        warn!(turn_id = %turn_id, "voice.transcript_final_no_session");
        return;
    };
    if s.muted {
        warn!(turn_id = %turn_id, "voice.transcript_final_dropped_muted");
        return;
    }
    if s.current_turn.is_some() {
        let Some(keep_ms) = pending_interruption(s, &text) else {
            warn!(
                turn_id = %turn_id,
                "voice.transcript_final_during_active_turn_dropped"
            );
            return;
        };
        abort_active_turn(s, ctx, keep_ms, "barge_in_final").await;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        warn!(turn_id = %turn_id, "voice.empty_transcript_final_dropped");
        return;
    }
    info!(
        turn_id = %turn_id,
        chars = trimmed.chars().count(),
        "voice.transcript_final"
    );
    let (playback_complete, playback_receiver) = oneshot::channel();
    let script = Arc::new(SpokenScript::default());
    let join: JoinHandle<()> = spawn_text_turn(
        text,
        turn_id.clone(),
        ctx,
        s,
        playback_receiver,
        Arc::clone(&script),
    );
    s.begin_turn(TurnInFlight {
        turn_id,
        join,
        started_at: Instant::now(),
        playback_complete: Some(playback_complete),
        script,
    });
}

fn spawn_text_turn(
    text: String,
    turn_id: String,
    ctx: &VoiceContext,
    s: &VoiceSession,
    playback_complete: oneshot::Receiver<()>,
    script: Arc<SpokenScript>,
) -> JoinHandle<()> {
    let task_ctx = ctx.clone();
    let voice_cfg = s.voice_cfg.clone();
    let completion_tx = ctx.turn_completion_tx.clone();
    let completion_turn_id = turn_id.clone();
    tokio::spawn(async move {
        run_mode_b_turn(
            text,
            turn_id,
            task_ctx,
            voice_cfg,
            playback_complete,
            script,
        )
        .await;
        drop(completion_tx.send(completion_turn_id).await);
    })
}

#[tracing::instrument(
    name = "voice.turn",
    skip_all,
    fields(turn_id = %turn_id, source = "client_stt")
)]
async fn run_mode_b_turn(
    text: String,
    turn_id: String,
    ctx: VoiceContext,
    voice_cfg: SessionVoiceConfig,
    playback_complete: oneshot::Receiver<()>,
    script: Arc<SpokenScript>,
) {
    let trimmed = text.trim();
    run_text_turn(
        trimmed,
        &turn_id,
        &ctx,
        voice_cfg,
        Some(playback_complete),
        script,
        VoiceTurnAdmission::Acquire,
    )
    .await;
}
