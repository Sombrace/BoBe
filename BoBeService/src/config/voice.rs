use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct VoiceConfig {
    pub(crate) enabled: bool,
    pub(crate) persona: String,
    pub(crate) speed: f32,
    pub(crate) stt_language: String,
    pub(crate) pause_sensitivity: PauseSensitivity,
    pub(crate) show_partial_caption: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PauseSensitivity {
    Tight,
    Balanced,
    Patient,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            persona: crate::constants::voice_wire::DEFAULT_PERSONA.into(),
            speed: 1.0,
            stt_language: "en".into(),
            pause_sensitivity: PauseSensitivity::Balanced,
            show_partial_caption: true,
        }
    }
}
