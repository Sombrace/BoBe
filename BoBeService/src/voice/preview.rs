use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio_util::task::AbortOnDropHandle;

use crate::error::AppError;
use crate::speech::protocol::ServerMessage;
use crate::speech::providers::pocket::{VOICES, migrate_voice};
use crate::speech::{TtsEngine, TtsTextDelivery};
use crate::voice::output::VoiceOutput;
use crate::voice::protocol_helpers::send_json;
use crate::voice::sentence_pipeline::SentencePipeline;
use crate::voice::session::VoiceDefaults;
use crate::voice::spoken::SpokenScript;
use crate::voice::synthesis::spawn_tts_task;

pub(crate) const MAX_TEXT_BYTES: usize = 8_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewRequest {
    pub(crate) text: String,
    pub(crate) voice: Option<String>,
    pub(crate) speed: Option<f32>,
    #[serde(default)]
    pub(crate) wait_for_playback: bool,
}

pub(crate) struct Preview {
    pub(crate) voice: String,
    pub(crate) speed: f32,
    pub(crate) wait_for_playback: bool,
    sentences: Vec<String>,
}

impl Preview {
    pub(crate) fn prepare(
        request: PreviewRequest,
        defaults: &VoiceDefaults,
    ) -> Result<Self, AppError> {
        if request.text.len() > MAX_TEXT_BYTES {
            return Err(AppError::Validation(format!(
                "Voice preview text must be at most {MAX_TEXT_BYTES} bytes"
            )));
        }
        let voice = migrate_voice(request.voice.as_deref().unwrap_or(&defaults.persona));
        if !VOICES.contains(&voice) {
            return Err(AppError::Validation(format!(
                "Unknown Pocket TTS voice: {voice}"
            )));
        }
        let speed = request.speed.unwrap_or(defaults.speed);
        if !speed.is_finite() || !(0.5..=2.0).contains(&speed) {
            return Err(AppError::Validation(
                "Speech speed must be between 0.5 and 2.0".into(),
            ));
        }
        let mut pipeline = SentencePipeline::new(Instant::now());
        let mut sentences = pipeline.feed(&request.text);
        sentences.extend(pipeline.flush());
        if sentences.is_empty() {
            return Err(AppError::Validation(
                "Voice preview requires speakable text".into(),
            ));
        }
        Ok(Self {
            voice: voice.to_owned(),
            speed,
            wait_for_playback: request.wait_for_playback,
            sentences,
        })
    }

    pub(crate) async fn run(
        self,
        tts: Arc<dyn TtsEngine>,
        output: VoiceOutput,
        turn_id: String,
        script: Arc<SpokenScript>,
        playback_complete: oneshot::Receiver<()>,
    ) -> Result<(), AppError> {
        let (sender, receiver) = mpsc::channel(16);
        let worker = AbortOnDropHandle::new(spawn_tts_task(
            tts,
            turn_id.clone(),
            receiver,
            output.clone(),
            Arc::new(AtomicBool::new(false)),
            self.voice,
            self.speed,
            TtsTextDelivery::SentenceCaptions,
            Instant::now(),
            Arc::clone(&script),
        ));
        for sentence in self.sentences {
            if sender.send(sentence).await.is_err() {
                break;
            }
        }
        drop(sender);
        match worker.await {
            Ok(true) => {}
            Ok(false) => {
                return Err(AppError::Internal(
                    "Speech synthesis did not complete".into(),
                ));
            }
            Err(error) => return Err(AppError::Internal(format!("Speech worker failed: {error}"))),
        }
        send_json(&output, &ServerMessage::TtsEnd { turn_id }).await;
        if self.wait_for_playback {
            tokio::time::timeout(
                crate::voice::run_text_turn::playback_report_limit(script.audio_duration()),
                playback_complete,
            )
            .await
            .map_err(|_| AppError::Internal("Playback acknowledgement timed out".into()))?
            .map_err(|_| AppError::Canceled("Playback client disconnected".into()))?;
        }
        Ok(())
    }
}
