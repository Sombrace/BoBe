pub(crate) const MILLIS_PER_SECOND: f64 = 1000.0;

pub(crate) const DEFAULT_DAEMON_PORT: u16 = 8766;

pub(crate) const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

pub(crate) const DEFAULT_OLLAMA_V1_URL: &str = "http://127.0.0.1:11434/v1";

pub(crate) mod attach {
    pub(crate) const PROTOCOL_MIN: u32 = 1;
    pub(crate) const PROTOCOL_MAX: u32 = 1;
    pub(crate) const PATH: &str = "/v1/hello";
}

pub(crate) mod privacy {
    pub(crate) const PURGE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);
    pub(crate) const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(50);
    #[allow(
        dead_code,
        reason = "drift checkpoint; Swift and shell probes consume the value"
    )]
    pub(crate) const CLIENT_TIMEOUT_SECS: u64 = 60;
}

pub(crate) mod mcp_status {
    pub(crate) const CONNECTED: &str = "connected";
    pub(crate) const FAILED: &str = "failed";
    pub(crate) const PENDING: &str = "pending";
    pub(crate) const STOPPED: &str = "stopped";
}

pub(crate) const BOBE_DATA_DIR_NAME: &str = ".bobe";

pub(crate) mod tool_call_status {
    pub(crate) const START: &str = "start";
    pub(crate) const COMPLETE: &str = "complete";
}

#[allow(dead_code, reason = "drift checkpoint; Swift consumes the values")]
pub(crate) mod pause_sensitivity_ms {
    pub(crate) const TIGHT: u32 = 400;
    pub(crate) const BALANCED: u32 = 800;
    pub(crate) const PATIENT: u32 = 1500;
}

pub(crate) mod voice_wire {
    pub(crate) const TTS_OUTPUT_SAMPLE_RATE: u32 = 24_000;
    pub(crate) const KOKORO_MODEL_DIR: &str = "kokoro-multi-lang-v1_0";
    pub(crate) const STREAMING_STT_MODEL_DIR: &str =
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17";
    pub(crate) const DEFAULT_PERSONA: &str = "af_bella";
    pub(crate) const MODEL_KIND_TTS: &str = "tts";
    pub(crate) const MODEL_KIND_STT: &str = "stt";
    pub(crate) const SUBPROTOCOL_V2: &str = "bobe.voice.v2";
}

#[cfg(test)]
mod tests {
    #[test]
    fn privacy_timeouts_leave_response_margin() {
        assert!(super::privacy::PURGE_DEADLINE < super::privacy::HTTP_TIMEOUT);
        assert!(
            super::privacy::HTTP_TIMEOUT
                < std::time::Duration::from_secs(super::privacy::CLIENT_TIMEOUT_SECS)
        );
    }
}
