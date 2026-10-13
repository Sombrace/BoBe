use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VoiceModelKind {
    Tts,
    Stt,
}

impl VoiceModelKind {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Tts => crate::constants::voice_wire::MODEL_KIND_TTS,
            Self::Stt => crate::constants::voice_wire::MODEL_KIND_STT,
        }
    }

    pub(super) fn all() -> impl Iterator<Item = VoiceModelKind> {
        ARTIFACTS.iter().map(|a| a.kind)
    }
}

pub(super) struct ModelArtifact {
    pub(super) kind: VoiceModelKind,
    pub(super) url: &'static str,
    pub(super) is_tarball: bool,
    pub(super) target_subpath: &'static str,
    pub(super) required_files: &'static [&'static str],
    pub(super) sha256: &'static str,
    pub(super) max_bytes: u64,
}

pub(super) const ARTIFACTS: &[ModelArtifact] = &[
    ModelArtifact {
        kind: VoiceModelKind::Tts,
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2",
        is_tarball: true,
        target_subpath: crate::constants::voice_wire::KOKORO_MODEL_DIR,
        required_files: &["model.onnx", "voices.bin"],
        sha256: "c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298",
        max_bytes: 375_000_000,
    },
    ModelArtifact {
        kind: VoiceModelKind::Stt,
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
        is_tarball: true,
        target_subpath: crate::constants::voice_wire::STREAMING_STT_MODEL_DIR,
        required_files: &[
            "encoder-epoch-99-avg-1.int8.onnx",
            "decoder-epoch-99-avg-1.onnx",
            "joiner-epoch-99-avg-1.int8.onnx",
            "tokens.txt",
            "test_wavs/0.wav",
        ],
        sha256: "9c559283e8498d3fe95913c79ca1cb454bb26281ac2b102b41306c7d752765d9",
        max_bytes: 140_000_000,
    },
];

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct VoiceInstallSnapshot {
    pub(crate) models: Vec<ModelProgress>,
    pub(crate) status: InstallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ModelProgress {
    pub(crate) kind: VoiceModelKind,
    pub(crate) label: &'static str,
    pub(crate) status: String,
    pub(crate) bytes_downloaded: u64,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) percent: Option<u8>,
}

impl ModelProgress {
    pub(super) fn pending(kind: VoiceModelKind) -> Self {
        Self {
            kind,
            label: kind.label(),
            status: "pending".into(),
            bytes_downloaded: 0,
            bytes_total: None,
            percent: None,
        }
    }

    pub(super) fn complete(kind: VoiceModelKind, note: &str) -> Self {
        Self {
            kind,
            label: kind.label(),
            status: note.into(),
            bytes_downloaded: 0,
            bytes_total: None,
            percent: Some(100),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallStatus {
    #[default]
    Idle,
    Running,
    Complete,
    Canceled,
    Failed,
}
