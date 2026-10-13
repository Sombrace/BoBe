use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{info, warn};

use crate::voice::engines::VoiceEnginesSnapshot;

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
    let filler_library = match tts.as_ref() {
        Some(tts_engine) => Some(Arc::new(
            crate::voice::filler_library::FillerLibrary::render(Arc::clone(tts_engine)).await,
        )),
        None => None,
    };
    VoiceEnginesSnapshot {
        stt,
        tts,
        filler_library,
    }
}

fn load_stt(models_root: &Path) -> Option<Arc<dyn crate::speech::SttEngine>> {
    let provider = std::env::var("BOBE_VOICE_PROVIDER").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "coreml".to_string()
        } else {
            "cpu".to_string()
        }
    });
    let num_threads: i32 = std::env::var("BOBE_VOICE_NUM_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    let dir = models_root.join(crate::constants::voice_wire::STREAMING_STT_MODEL_DIR);
    if !dir.is_dir() {
        info!(path = %dir.display(), "voice.stt_model_not_installed");
        return None;
    }
    match crate::speech::providers::sherpa::streaming_stt::SherpaStreamingStt::load(
        &dir,
        &provider,
        num_threads,
    ) {
        Ok(engine) => {
            let rtf = crate::speech::SttEngine::health(&engine).real_time_factor;
            info!(path = %dir.display(), provider, rtf, "voice.stt_loaded");
            Some(Arc::new(engine) as Arc<dyn crate::speech::SttEngine>)
        }
        Err(error) => {
            warn!(%error, "voice.stt_load_failed");
            None
        }
    }
}

fn load_tts(models_root: &Path) -> Option<Arc<dyn crate::speech::TtsEngine>> {
    let dir = models_root.join(crate::constants::voice_wire::POCKET_TTS_MODEL_DIR);
    if !dir.is_dir() {
        info!(path = %dir.display(), "voice.tts_model_not_installed");
        return None;
    }
    match crate::speech::providers::pocket::PocketTts::load(&dir) {
        Ok(engine) => {
            info!(path = %dir.display(), engine = "pocket_tts", language = "en", "voice.tts_loaded");
            Some(Arc::new(engine) as Arc<dyn crate::speech::TtsEngine>)
        }
        Err(error) => {
            warn!(%error, "voice.tts_load_failed");
            None
        }
    }
}
