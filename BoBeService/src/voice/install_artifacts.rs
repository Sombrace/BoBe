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

pub(super) struct ModelFile {
    pub(super) path: &'static str,
    pub(super) url: &'static str,
    pub(super) sha256: &'static str,
    pub(super) max_bytes: u64,
}

pub(super) struct ModelArtifact {
    pub(super) kind: VoiceModelKind,
    pub(super) files: &'static [ModelFile],
    pub(super) is_tarball: bool,
    pub(super) target_subpath: &'static str,
    pub(super) required_files: &'static [&'static str],
}

// One immutable, generation-matched set: weights, tokenizer, config and presets.
// Provenance and attribution are also installed alongside the model.
const POCKET_FILES: &[ModelFile] = &[
    ModelFile {
        path: "config.json",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/config.json",
        sha256: "c1178a4fb93365c9d5086d6a075540e88eaaf117277d7a82c7f2246af77f39c1",
        max_bytes: 988,
    },
    ModelFile {
        path: "tokenizer.json",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/tokenizer.json",
        sha256: "f498428e1eafee50492f7be13dc9bfafcfc12e508cd0eb1b01c92ecd5d8c6687",
        max_bytes: 245_020,
    },
    ModelFile {
        path: "model.q8.gguf",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/model.q8.gguf",
        sha256: "dda32506821b9895d67cf66232e97c4187f92ec82e51e7e537835bb681f964e8",
        max_bytes: 164_952_320,
    },
    ModelFile {
        path: "embeddings/alba.safetensors",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/embeddings/alba.safetensors",
        sha256: "ad234695323e4030336b6afc8a050c97e3110603e11ecd8226d9562488300a50",
        max_bytes: 512_088,
    },
    ModelFile {
        path: "embeddings/azelma.safetensors",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/embeddings/azelma.safetensors",
        sha256: "ef33fad34437cb187d2702f0a946d8ba7a01efdb8efbc8088c770d49c181ba73",
        max_bytes: 659_544,
    },
    ModelFile {
        path: "embeddings/eponine.safetensors",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/embeddings/eponine.safetensors",
        sha256: "bb31940f62da665391de139da2e57d740757df26b73d7ec24152c78a3b8ac0c5",
        max_bytes: 573_528,
    },
    ModelFile {
        path: "embeddings/fantine.safetensors",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/embeddings/fantine.safetensors",
        sha256: "b6918a2ece002d2d9037ff53c4ea38730175e8798786658b0958443edf49d355",
        max_bytes: 540_760,
    },
    ModelFile {
        path: "embeddings/javert.safetensors",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/embeddings/javert.safetensors",
        sha256: "2e857904ee76657e083b0e92664d21bd133e37df320af6eb04f752e679422d91",
        max_bytes: 512_088,
    },
    ModelFile {
        path: "embeddings/marius.safetensors",
        url: "https://huggingface.co/davencyw/pocket-tts-ptts/resolve/7cc3c90b88f3e6617ded1b9b74100d1cd0162d90/embeddings/marius.safetensors",
        sha256: "33f75e45fac0005630671f4b1bb632d51b6a083b18417de94855bbd7596a0630",
        max_bytes: 512_088,
    },
];

pub(super) const ARTIFACTS: &[ModelArtifact] = &[
    ModelArtifact {
        kind: VoiceModelKind::Tts,
        files: POCKET_FILES,
        is_tarball: false,
        target_subpath: crate::constants::voice_wire::POCKET_TTS_MODEL_DIR,
        required_files: &[
            "config.json",
            "tokenizer.json",
            "model.q8.gguf",
            "embeddings/alba.safetensors",
            "embeddings/azelma.safetensors",
            "embeddings/eponine.safetensors",
            "embeddings/fantine.safetensors",
            "embeddings/javert.safetensors",
            "embeddings/marius.safetensors",
            "NOTICE.txt",
        ],
    },
    ModelArtifact {
        kind: VoiceModelKind::Stt,
        files: &[ModelFile {
            path: "stt.tar.bz2",
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
            sha256: "9c559283e8498d3fe95913c79ca1cb454bb26281ac2b102b41306c7d752765d9",
            max_bytes: 140_000_000,
        }],
        is_tarball: true,
        target_subpath: crate::constants::voice_wire::STREAMING_STT_MODEL_DIR,
        required_files: &[
            "encoder-epoch-99-avg-1.int8.onnx",
            "decoder-epoch-99-avg-1.onnx",
            "joiner-epoch-99-avg-1.int8.onnx",
            "tokens.txt",
            "test_wavs/0.wav",
        ],
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
