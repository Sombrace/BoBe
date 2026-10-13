use std::path::Path;

use ptts::checkpoint::{Checkpoint, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};

use crate::error::AppError;
use crate::speech::TtsEngine;

mod tempo;

pub(crate) const VOICES: &[&str] = &["alba", "azelma", "eponine", "fantine", "javert", "marius"];

/// Fixed English presets; no microphone audio or voice cloning is used.
pub(crate) struct PocketTts {
    synth: Synth,
}

impl PocketTts {
    pub(crate) fn load(directory: &Path) -> Result<Self, AppError> {
        let checkpoint = Checkpoint::resolve(
            directory,
            ResolveOptions {
                quant: Quant::Q80,
                weights: None,
            },
        )
        .map_err(load_error)?;
        let mut builder = checkpoint
            .builder(Lang::En)
            .device(DeviceKind::Cpu)
            .voice(crate::constants::voice_wire::DEFAULT_PERSONA);
        for voice in VOICES {
            builder = builder.add_voice(
                *voice,
                directory.join(format!("embeddings/{voice}.safetensors")),
            );
        }
        let synth = builder.build().map_err(load_error)?;
        if synth.sample_rate() != crate::constants::voice_wire::TTS_OUTPUT_SAMPLE_RATE {
            return Err(AppError::Config(
                "Pocket TTS must produce 24 kHz audio".into(),
            ));
        }
        Ok(Self { synth })
    }
}

fn load_error(error: impl std::fmt::Display) -> AppError {
    AppError::Config(format!("Cannot load Pocket TTS English pack: {error}"))
}

/// Existing installations may still have a Kokoro preset saved in their config.
pub(crate) fn migrate_voice(voice: &str) -> &str {
    let legacy_prefix = voice.split_once('_').map(|(prefix, _)| prefix);
    if matches!(
        legacy_prefix,
        Some(
            "af" | "am"
                | "bf"
                | "bm"
                | "ef"
                | "em"
                | "ff"
                | "hf"
                | "hm"
                | "if"
                | "im"
                | "jf"
                | "jm"
                | "pf"
                | "pm"
                | "zf"
                | "zm"
        )
    ) || voice.parse::<u8>().is_ok_and(|id| id <= 52)
    {
        crate::constants::voice_wire::DEFAULT_PERSONA
    } else {
        voice
    }
}

impl TtsEngine for PocketTts {
    fn synthesize_stream(
        &self,
        text: &str,
        voice: &str,
        speed: f32,
        emit: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<(), AppError> {
        let voice = migrate_voice(voice);
        if !VOICES.contains(&voice) {
            return Err(AppError::Validation(format!(
                "Unknown Pocket TTS voice: {voice}"
            )));
        }
        if !speed.is_finite() || !(0.5..=2.0).contains(&speed) {
            return Err(AppError::Validation(
                "Speech speed must be between 0.5 and 2.0".into(),
            ));
        }
        if text.trim().is_empty() {
            return Ok(());
        }
        let stream = self
            .synth
            .stream_with(text, &SpeechOptions::default().voice(voice))
            .map_err(|e| AppError::Internal(format!("Pocket TTS generation failed: {e}")))?;
        let mut tempo = ((speed - 1.0).abs() > f32::EPSILON)
            .then(|| tempo::Tempo::new(self.sample_rate(), speed));
        for chunk in stream {
            let chunk =
                chunk.map_err(|e| AppError::Internal(format!("Pocket TTS stream failed: {e}")))?;
            let output = match tempo.as_mut() {
                Some(tempo) => tempo.process(&chunk),
                None => chunk,
            };
            if !output.is_empty() && !emit(&output) {
                // Dropping ptts's bounded stream stops and joins its inference workers.
                return Err(AppError::Canceled("Speech playback stopped".into()));
            }
        }
        if let Some(tempo) = tempo.as_mut() {
            let tail = tempo.finish();
            if !tail.is_empty() && !emit(&tail) {
                return Err(AppError::Canceled("Speech playback stopped".into()));
            }
        }
        Ok(())
    }

    fn sample_rate(&self) -> u32 {
        self.synth.sample_rate()
    }
}
