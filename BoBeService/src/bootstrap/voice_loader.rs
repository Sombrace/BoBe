use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{info, warn};

use crate::speech::TtsEngine;
use crate::voice::engines::VoiceEnginesSnapshot;
use crate::voice::filler_library::FillerLibrary;

pub(super) async fn build_voice_engines_snapshot(models_root: PathBuf) -> VoiceEnginesSnapshot {
    let stt_models_root = models_root.clone();
    let (tts, stt) = tokio::join!(
        tokio::task::spawn_blocking(move || load_tts(&models_root)),
        tokio::task::spawn_blocking(move || load_stt(&stt_models_root)),
    );
    let tts = match tts {
        Ok(tts) => tts,
        Err(error) => {
            warn!(%error, "voice.tts_load_join_failed");
            None
        }
    };
    let stt = match stt {
        Ok(stt) => stt,
        Err(error) => {
            warn!(%error, "voice.stt_load_join_failed");
            None
        }
    };
    assemble_snapshot(stt, tts).await
}

async fn assemble_snapshot(
    stt: Option<Arc<dyn crate::speech::SttEngine>>,
    tts: Option<Arc<dyn TtsEngine>>,
) -> VoiceEnginesSnapshot {
    let Some(engine) = tts else {
        return VoiceEnginesSnapshot {
            stt,
            ..VoiceEnginesSnapshot::default()
        };
    };
    match FillerLibrary::render(Arc::clone(&engine)).await {
        Ok(library) => VoiceEnginesSnapshot {
            stt,
            tts: Some(engine),
            filler_library: Some(Arc::new(library)),
        },
        Err(error) => {
            warn!(%error, "voice.tts_probe_failed");
            VoiceEnginesSnapshot {
                stt,
                ..VoiceEnginesSnapshot::default()
            }
        }
    }
}

fn load_stt(models_root: &Path) -> Option<Arc<dyn crate::speech::SttEngine>> {
    let provider = if cfg!(target_os = "macos") {
        "coreml"
    } else {
        "cpu"
    };
    let dir = models_root.join(crate::constants::voice_wire::STREAMING_STT_MODEL_DIR);
    if !dir.is_dir() {
        info!(path = %dir.display(), "voice.stt_model_not_installed");
        return None;
    }
    match crate::speech::providers::sherpa::streaming_stt::SherpaStreamingStt::load(
        &dir, provider, 4,
    ) {
        Ok(engine) => {
            let rtf = crate::speech::SttEngine::health(&engine).real_time_factor;
            info!(path = %dir.display(), provider, rtf, "voice.stt_loaded");
            Some(Arc::new(engine))
        }
        Err(error) => {
            warn!(%error, "voice.stt_load_failed");
            None
        }
    }
}

fn load_tts(models_root: &Path) -> Option<Arc<dyn TtsEngine>> {
    let dir = models_root.join(crate::constants::voice_wire::POCKET_TTS_MODEL_DIR);
    if !dir.is_dir() {
        info!(path = %dir.display(), "voice.tts_model_not_installed");
        return None;
    }
    match crate::speech::providers::pocket::PocketTts::load(&dir) {
        Ok(engine) => {
            info!(path = %dir.display(), engine = "pocket_tts", language = "en", "voice.tts_loaded");
            Some(Arc::new(engine))
        }
        Err(error) => {
            warn!(%error, "voice.tts_load_failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::assemble_snapshot;
    use crate::error::AppError;
    use crate::speech::TtsEngine;

    struct LoadsButCannotSpeak;

    impl TtsEngine for LoadsButCannotSpeak {
        fn synthesize_stream(
            &self,
            _: &str,
            _: &str,
            _: f32,
            _: &mut dyn FnMut(&[f32]) -> bool,
        ) -> Result<(), AppError> {
            Err(AppError::Internal("stream failed".into()))
        }

        fn sample_rate(&self) -> u32 {
            24_000
        }
    }

    struct Speaks;

    impl TtsEngine for Speaks {
        fn synthesize_stream(
            &self,
            _: &str,
            _: &str,
            _: f32,
            emit: &mut dyn FnMut(&[f32]) -> bool,
        ) -> Result<(), AppError> {
            emit(&[0.1; 240]);
            Ok(())
        }

        fn sample_rate(&self) -> u32 {
            24_000
        }
    }

    #[tokio::test]
    async fn a_voice_that_loads_but_cannot_speak_is_not_ready() {
        let snapshot = assemble_snapshot(None, Some(Arc::new(LoadsButCannotSpeak))).await;
        assert!(!snapshot.is_complete());
        assert!(snapshot.tts.is_none());
        assert!(snapshot.filler_library.is_none());
    }

    #[tokio::test]
    async fn a_voice_that_speaks_is_ready_with_its_fillers() {
        let snapshot = assemble_snapshot(None, Some(Arc::new(Speaks))).await;
        assert!(snapshot.is_complete());
        assert!(snapshot.filler_library.is_some());
    }
}
