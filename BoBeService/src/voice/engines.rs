use std::sync::Arc;

use crate::app_state::AppState;
use crate::speech::{SttEngine, TtsEngine};
use crate::voice::filler_library::FillerLibrary;

#[derive(Default)]
pub(crate) struct VoiceEnginesSnapshot {
    pub(crate) stt: Option<Arc<dyn SttEngine>>,
    pub(crate) tts: Option<Arc<dyn TtsEngine>>,
    pub(crate) filler_library: Option<Arc<FillerLibrary>>,
}

impl VoiceEnginesSnapshot {
    pub(crate) fn is_complete(&self) -> bool {
        self.tts.is_some()
    }

    pub(crate) fn supports_daemon_stt(&self) -> bool {
        self.stt
            .as_ref()
            .is_some_and(|engine| engine.health().is_viable())
    }
}

#[derive(Clone)]
pub(crate) struct VoiceEngines {
    pub(crate) stt: Option<Arc<dyn SttEngine>>,
    pub(crate) tts: Option<Arc<dyn TtsEngine>>,
    pub(crate) fillers: Option<Arc<FillerLibrary>>,
}

impl VoiceEngines {
    pub(crate) fn from_state(state: &AppState) -> Self {
        Self::from_snapshot(&state.voice.voice_engines.load())
    }

    pub(crate) fn from_snapshot(snap: &VoiceEnginesSnapshot) -> Self {
        Self {
            stt: snap.stt.as_ref().map(Arc::clone),
            tts: snap.tts.as_ref().map(Arc::clone),
            fillers: snap.filler_library.as_ref().map(Arc::clone),
        }
    }

    pub(crate) fn daemon_stt(&self) -> Option<&Arc<dyn SttEngine>> {
        self.stt
            .as_ref()
            .filter(|engine| engine.health().is_viable())
    }

    pub(crate) fn supports_server_tts(&self) -> bool {
        self.tts.is_some()
    }
}
