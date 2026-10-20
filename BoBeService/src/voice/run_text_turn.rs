use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

use crate::models::ids::conversation_turn_id_from_wire_id;
use crate::speech::protocol::{ServerMessage, VoicePhase};
use crate::voice::context::VoiceContext;
use crate::voice::filler_library::FillerKind;
use crate::voice::output::VoiceOutput;
use crate::voice::protocol_helpers::{send_error, send_json, send_state};
use crate::voice::sentence_pipeline::SentencePipeline;
use crate::voice::session::{PlaybackReport, SessionVoiceConfig};
use crate::voice::sinks::filler_frames;
use crate::voice::spoken::SpokenScript;
use crate::voice::synthesis::spawn_tts_task;
use tokio_util::task::AbortOnDropHandle;

pub(crate) const FILLER_TRIGGER: Duration = Duration::from_millis(800);
const SENTENCE_PAUSE: Duration = Duration::from_millis(250);
const PLAYBACK_REPORT_LIMIT: Duration = Duration::from_mins(10);
const PLAYBACK_REPORT_MARGIN: Duration = Duration::from_secs(30);

pub(crate) enum VoiceTurnAdmission {
    Acquire,
    AlreadyHeld,
}

struct VoiceRunLifetime {
    session: Arc<crate::runtime::session::RuntimeSession>,
    turn: crate::models::ids::ConversationTurnId,
    admission: Option<crate::runtime::session::UserMessageGuard>,
    registration: Option<crate::runtime::run_registry::RunRegistration>,
    cancellation: tokio_util::sync::CancellationToken,
    settled: bool,
}

impl VoiceRunLifetime {
    async fn settle(&mut self) {
        self.session.settle_interrupted_turn(self.turn).await;
        self.settled = true;
    }
}

impl Drop for VoiceRunLifetime {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if !self.settled {
            let session = Arc::clone(&self.session);
            let turn = self.turn;
            let admission = self.admission.take();
            let registration = self.registration.take();
            tokio::spawn(async move {
                session.settle_interrupted_turn(turn).await;
                drop(admission);
                drop(registration);
            });
        }
    }
}

pub(crate) async fn run_text_turn(
    text: &str,
    turn_id: &str,
    ctx: &VoiceContext,
    voice_cfg: SessionVoiceConfig,
    playback_complete: Option<oneshot::Receiver<PlaybackReport>>,
    script: Arc<SpokenScript>,
    admission: VoiceTurnAdmission,
) {
    let received_at = chrono::Utc::now();
    let engines = &ctx.engines;
    let output = &ctx.output;
    let runtime_session = &ctx.runtime_session;
    let turn_start = Instant::now();
    let assistant_turn_id = match conversation_turn_id_from_wire_id(turn_id) {
        Ok(turn_id) => turn_id,
        Err(parse_error) => {
            error!(turn_id, error = %parse_error, "voice.invalid_turn_id");
            send_error(
                output,
                "invalid_turn_id",
                "The voice turn identifier is invalid",
            )
            .await;
            send_state(output, VoicePhase::Listening, turn_id).await;
            return;
        }
    };

    let owner_input = match runtime_session
        .bind_owner_answer(ctx.conversation_channel, text, received_at)
        .await
    {
        Ok(input) => input,
        Err(error) => {
            error!(%error,turn_id,"voice.owner_answer_binding_failed");
            send_error(
                output,
                "owner_answer_failed",
                "The answer could not be linked to its question. The action is still paused.",
            )
            .await;
            return;
        }
    };
    let guard = match admission {
        VoiceTurnAdmission::AlreadyHeld => None,
        VoiceTurnAdmission::Acquire => match runtime_session
            .begin_user_message(ctx.conversation_channel)
            .await
        {
            Ok(guard) => Some(guard),
            Err(reason) => {
                send_error(output, "conflict", &reason.to_string()).await;
                send_state(output, VoicePhase::Listening, turn_id).await;
                return;
            }
        },
    };

    let cancellation = tokio_util::sync::CancellationToken::new();
    let registration = match runtime_session.runs.register_cancellation(
        assistant_turn_id.into(),
        cancellation.clone(),
        crate::runtime::run_registry::RunKind::Voice,
        Some(ctx.conversation_channel),
    ) {
        Ok(registration) => registration,
        Err(error) => {
            error!(%error,turn_id,"voice.run_registration_failed");
            send_error(output, "conflict", "This voice turn is already active").await;
            send_state(output, VoicePhase::Listening, turn_id).await;
            return;
        }
    };
    let mut lifetime = VoiceRunLifetime {
        session: Arc::clone(runtime_session),
        turn: assistant_turn_id,
        admission: guard,
        registration: Some(registration),
        cancellation: cancellation.clone(),
        settled: false,
    };
    if let Err(error) = runtime_session
        .answer_bound_input(
            owner_input,
            assistant_turn_id.into(),
            text,
            ctx.conversation_channel,
        )
        .await
    {
        error!(%error,turn_id,"voice.owner_answer_failed");
        send_error(
            output,
            "owner_answer_failed",
            "The answer could not be saved. The action is still paused.",
        )
        .await;
        return;
    }
    let _sink_guard = match ctx
        .voice_sink
        .install(assistant_turn_id.into(), output.clone())
    {
        Ok(guard) => guard,
        Err(error) => {
            error!(%error,turn_id,"voice.output_binding_failed");
            send_error(output, "conflict", "This voice output is already active").await;
            return;
        }
    };

    send_state(output, VoicePhase::Thinking, turn_id).await;
    send_json(
        output,
        &ServerMessage::TranscriptFinal {
            turn_id: turn_id.to_string(),
            text: text.to_string(),
        },
    )
    .await;

    send_state(output, VoicePhase::Speaking, turn_id).await;
    let speaking_start = Instant::now();
    let first_audio_emitted = Arc::new(AtomicBool::new(false));
    let tts = if voice_cfg.speech_plan.uses_endpoint_tts() {
        None
    } else {
        let Some(tts) = engines.tts.as_ref().map(Arc::clone) else {
            send_error(
                output,
                "engines_unavailable",
                "Pocket TTS English pack is not installed",
            )
            .await;
            send_state(output, VoicePhase::Listening, turn_id).await;
            return;
        };
        Some(tts)
    };
    let filler_task = tts.as_ref().and_then(|tts| {
        spawn_filler_watchdog(
            engines
                .fillers
                .as_ref()
                .and_then(|lib| lib.get(FillerKind::Thinking)),
            tts.sample_rate(),
            Arc::clone(&first_audio_emitted),
            output.clone(),
        )
        .map(AbortOnDropHandle::new)
    });
    let (sentence_tx, sentence_rx) = mpsc::channel::<String>(16);
    let spoken = Arc::clone(&script);
    let voice_output_task = if voice_cfg.speech_plan.uses_endpoint_tts() {
        AbortOnDropHandle::new(spawn_client_tts_text_task(
            turn_id.to_string(),
            sentence_rx,
            output.clone(),
            script,
        ))
    } else {
        let Some(tts) = tts else {
            return;
        };
        AbortOnDropHandle::new(spawn_tts_task(
            tts,
            turn_id.to_string(),
            sentence_rx,
            output.clone(),
            Arc::clone(&first_audio_emitted),
            voice_cfg.speech_plan.voice_id.clone(),
            voice_cfg.speed,
            voice_cfg.speech_plan.tts_text_delivery,
            script,
        ))
    };

    let pipeline = Arc::new(tokio::sync::Mutex::new(SentencePipeline::new()));
    let stream_activity = Arc::new(tokio::sync::Notify::new());
    let pause_release = AbortOnDropHandle::new(spawn_pause_release(
        Arc::clone(&pipeline),
        sentence_tx.clone(),
        Arc::clone(&stream_activity),
    ));
    let pipeline_for_obs = Arc::clone(&pipeline);
    let sentence_tx_for_obs = sentence_tx.clone();
    let observer = move |delta: String| {
        let pipeline = Arc::clone(&pipeline_for_obs);
        let sentence_tx = sentence_tx_for_obs.clone();
        let stream_activity = Arc::clone(&stream_activity);
        async move {
            let mut pipeline = pipeline.lock().await;
            stream_activity.notify_one();
            for sentence in pipeline.feed(&delta) {
                if sentence_tx.send(sentence).await.is_err() {
                    return;
                }
            }
        }
    };

    let turn_succeeded = tokio::select! {
        biased;
        () = cancellation.cancelled() => false,
        result = runtime_session.handle_user_message_with_observer(
            text,
            turn_id,
            assistant_turn_id,
            ctx.delivery,
            ctx.conversation_channel,
            observer,
        ) => result,
    };
    lifetime.settle().await;
    {
        let _quiescent = pipeline.lock().await;
        abort_and_settle(pause_release, "sentence pause").await;
    }
    if !turn_succeeded {
        drop(sentence_tx);
        abort_and_settle(voice_output_task, "voice output").await;
        if let Some(filler) = filler_task {
            abort_and_settle(filler, "filler").await;
        }
        send_error(output, "turn_failed", "The assistant turn did not complete").await;
        send_state(output, VoicePhase::Listening, turn_id).await;
        return;
    }

    let remaining = pipeline.lock().await.flush();
    for sentence in remaining {
        if sentence_tx.send(sentence).await.is_err() {
            break;
        }
    }
    drop(pipeline);
    drop(sentence_tx);
    let mut voice_output_task = voice_output_task;
    let output_result = tokio::select! {
        biased;
        ()=cancellation.cancelled()=>{
            voice_output_task.abort();
            send_json(output,&ServerMessage::Truncate {turn_id:turn_id.to_owned(),keep_ms:0}).await;
            voice_output_task.await
        }
        result=&mut voice_output_task=>result,
    };
    let output_produced = match output_result {
        Ok(produced) => produced,
        Err(e) if e.is_panic() => {
            error!(error = %e, "voice.output_task_panic");
            false
        }
        Err(e) if e.is_cancelled() => {
            debug!("voice.output_task_cancelled");
            false
        }
        Err(e) => {
            warn!(error = %e, "voice.output_task_join_failed");
            false
        }
    };

    if let Some(filler) = filler_task {
        abort_and_settle(filler, "filler").await;
    }
    if !output_produced {
        send_error(
            output,
            "tts_failed",
            "Response audio could not be completed",
        )
        .await;
        send_state(output, VoicePhase::Listening, turn_id).await;
        return;
    }

    send_json(
        output,
        &ServerMessage::TtsEnd {
            turn_id: turn_id.to_string(),
        },
    )
    .await;
    if let Some(receiver) = playback_complete {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                send_json(
                    output,
                    &ServerMessage::Truncate {
                        turn_id: turn_id.to_owned(),
                        keep_ms: 0,
                    },
                )
                .await;
            }
            result = tokio::time::timeout(playback_report_limit(spoken.audio_duration()), receiver) => {
                match result {
                    Ok(Ok(PlaybackReport::Heard)) => {
                        if let Err(error) = runtime_session
                            .acknowledge_presentation(assistant_turn_id, ctx.conversation_channel)
                            .await
                        {
                            warn!(%error, turn_id, "voice.presentation_receipt_failed");
                        }
                    }
                    Ok(Ok(PlaybackReport::Unheard)) => {
                        warn!(turn_id, "voice.reply_left_unpresented");
                    }
                    Ok(Err(_)) => debug!("voice.playback_report_sender_dropped"),
                    Err(_) => warn!("voice.playback_report_timeout"),
                }
            }
        }
    }
    send_state(output, VoicePhase::Listening, turn_id).await;
    drop(lifetime);

    let total_ms = turn_start.elapsed().as_millis() as u64;
    tracing::info!(
        speaking_ms = speaking_start.elapsed().as_millis() as u64,
        total_ms,
        "voice.turn_complete"
    );
}

async fn abort_and_settle<T>(task: AbortOnDropHandle<T>, task_name: &'static str) {
    let handle = task.detach();
    handle.abort();
    match handle.await {
        Err(error) if error.is_cancelled() => {}
        Err(error) => warn!(%error, task_name, "voice.child_task_join_failed"),
        Ok(_) => debug!(task_name, "voice.child_task_completed_before_abort"),
    }
}

pub(crate) fn playback_report_limit(synthesised: Option<Duration>) -> Duration {
    synthesised.map_or(PLAYBACK_REPORT_LIMIT, |audio| {
        (audio + PLAYBACK_REPORT_MARGIN).min(PLAYBACK_REPORT_LIMIT)
    })
}

fn spawn_pause_release(
    pipeline: Arc<tokio::sync::Mutex<SentencePipeline>>,
    sentence_tx: mpsc::Sender<String>,
    stream_activity: Arc<tokio::sync::Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            stream_activity.notified().await;
            loop {
                tokio::select! {
                    () = stream_activity.notified() => {}
                    () = tokio::time::sleep(SENTENCE_PAUSE) => break,
                }
            }
            let mut pipeline = pipeline.lock().await;
            for sentence in pipeline.settle() {
                if sentence_tx.send(sentence).await.is_err() {
                    return;
                }
            }
        }
    })
}

fn spawn_client_tts_text_task(
    turn_id: String,
    mut sentence_rx: mpsc::Receiver<String>,
    output: VoiceOutput,
    script: Arc<SpokenScript>,
) -> tokio::task::JoinHandle<bool> {
    tokio::spawn(async move {
        let mut sequence = 0_u64;
        let mut produced = false;
        while let Some(text) = sentence_rx.recv().await {
            produced = true;
            script.push(&text, sequence.saturating_add(1));
            send_json(
                &output,
                &ServerMessage::TtsText {
                    turn_id: turn_id.clone(),
                    sequence,
                    text,
                },
            )
            .await;
            sequence = sequence.saturating_add(1);
        }
        script.finish();
        produced
    })
}

fn spawn_filler_watchdog(
    filler_pcm: Option<Arc<Vec<f32>>>,
    sample_rate: u32,
    first_audio_emitted: Arc<AtomicBool>,
    output: VoiceOutput,
) -> Option<JoinHandle<()>> {
    let pcm = filler_pcm?;
    Some(tokio::spawn(async move {
        tokio::time::sleep(FILLER_TRIGGER).await;
        let Some(_sequence) = output.try_audio_sequence() else {
            return;
        };
        if first_audio_emitted.swap(true, Ordering::AcqRel) {
            debug!("voice.filler_skipped_tts_already_emitted");
            return;
        }
        debug!(samples = pcm.len(), "voice.filler_emit");
        let Some(frames) = filler_frames(&pcm, sample_rate) else {
            return;
        };
        for framed in frames {
            if !output.audio(framed).await {
                return;
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::output::VoiceOutputFrame;

    #[test]
    fn the_wait_for_a_playback_report_follows_the_audio_that_was_sent() {
        assert_eq!(playback_report_limit(None), PLAYBACK_REPORT_LIMIT);
        assert_eq!(
            playback_report_limit(Some(Duration::from_secs(12))),
            Duration::from_secs(42)
        );
        assert_eq!(
            playback_report_limit(Some(Duration::from_hours(1))),
            PLAYBACK_REPORT_LIMIT
        );
    }

    #[tokio::test]
    async fn a_finished_sentence_is_sent_when_the_stream_pauses() {
        let pipeline = Arc::new(tokio::sync::Mutex::new(SentencePipeline::new()));
        let activity = Arc::new(tokio::sync::Notify::new());
        let (sentence_tx, mut sentence_rx) = mpsc::channel::<String>(4);
        let _task = AbortOnDropHandle::new(spawn_pause_release(
            Arc::clone(&pipeline),
            sentence_tx,
            Arc::clone(&activity),
        ));

        assert_eq!(
            pipeline.lock().await.feed("Let me check that."),
            Vec::<String>::new()
        );
        activity.notify_one();
        tokio::time::sleep(SENTENCE_PAUSE / 3).await;
        assert!(sentence_rx.try_recv().is_err());
        let released = tokio::time::timeout(SENTENCE_PAUSE * 8, sentence_rx.recv()).await;
        assert_eq!(
            released.ok().flatten().as_deref(),
            Some("Let me check that.")
        );
    }

    #[tokio::test]
    async fn settled_child_cannot_emit_audio_after_terminal_state() {
        let (output, mut receiver) = VoiceOutput::channel(4);
        let child_output = output.clone();
        let child = AbortOnDropHandle::new(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            child_output.audio(vec![1, 2, 3]).await
        }));

        abort_and_settle(child, "test output").await;
        send_state(&output, VoicePhase::Listening, "turn_test").await;
        drop(output);

        let mut frames = Vec::new();
        while let Some(frame) = receiver.recv().await {
            frames.push(frame);
        }
        assert_eq!(frames.len(), 1);
        std::assert_matches!(frames.first(), Some(VoiceOutputFrame::Control(_)));
    }
}
