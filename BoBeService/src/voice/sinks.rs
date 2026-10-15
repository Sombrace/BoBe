use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tracing::warn;
use uuid::Uuid;

use crate::error::AppError;
use crate::speech::protocol::{FLAG_FILLER, encode_tts_frame};
use crate::voice::filler_library::{FillerKind, FillerLibrary};
use crate::voice::opus::{encode_pcm_with, make_opus_encoder};
use crate::voice::output::VoiceOutput;

#[derive(Default)]
pub(crate) struct VoiceSink {
    outputs: Mutex<HashMap<Uuid, VoiceOutput>>,
}

impl VoiceSink {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn install(
        self: &Arc<Self>,
        run_id: Uuid,
        output: VoiceOutput,
    ) -> Result<SinkGuard, AppError> {
        let mut outputs = self.lock();
        if outputs.contains_key(&run_id) {
            return Err(AppError::Conflict(
                "voice output is already bound to this run".into(),
            ));
        }
        outputs.insert(run_id, output);
        Ok(SinkGuard {
            sink: Arc::clone(self),
            run_id,
        })
    }

    fn get(&self, run_id: Uuid) -> Option<VoiceOutput> {
        self.lock().get(&run_id).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, VoiceOutput>> {
        self.outputs.lock().unwrap_or_else(|poisoned| {
            tracing::error!("voice output registry was poisoned");
            poisoned.into_inner()
        })
    }
}

pub(crate) struct SinkGuard {
    sink: Arc<VoiceSink>,
    run_id: Uuid,
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        self.sink.lock().remove(&self.run_id);
    }
}

pub(crate) async fn emit_filler(
    sink: &VoiceSink,
    run_id: Uuid,
    library: &FillerLibrary,
    kind: FillerKind,
) {
    let Some(output) = sink.get(run_id) else {
        return;
    };
    let Some(_sequence) = output.try_audio_sequence() else {
        return;
    };
    let Some(pcm) = library.get(kind) else {
        return;
    };
    let Some(frames) = filler_frames(&pcm, library.sample_rate()) else {
        return;
    };
    for framed in frames {
        if !output.audio(framed).await {
            warn!(%run_id, "voice.sink_emit_send_failed");
            return;
        }
    }
}

pub(crate) fn filler_frames(pcm: &[f32], sample_rate: u32) -> Option<Vec<Vec<u8>>> {
    let mut encoder = make_opus_encoder(sample_rate)?;
    Some(
        encode_pcm_with(&mut encoder, pcm, sample_rate)
            .iter()
            .zip(0_u64..)
            .map(|(packet, chunk_id)| encode_tts_frame(chunk_id, FLAG_FILLER, packet))
            .collect(),
    )
}

pub(crate) fn filler_for_tool(tool_name: &str) -> FillerKind {
    let lower = tool_name.to_ascii_lowercase();
    if (lower.contains("web") && lower.contains("search")) || lower == "websearch" {
        FillerKind::ToolWebSearch
    } else if lower == "read" || lower == "read_file" || lower.contains("readfile") {
        FillerKind::ToolReadFile
    } else {
        FillerKind::ToolGeneric
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filler_for_tool_routes_known_names() {
        assert_eq!(filler_for_tool("web_search"), FillerKind::ToolWebSearch);
        assert_eq!(filler_for_tool("WebSearch"), FillerKind::ToolWebSearch);
        assert_eq!(filler_for_tool("Read"), FillerKind::ToolReadFile);
        assert_eq!(filler_for_tool("read_file"), FillerKind::ToolReadFile);
        assert_eq!(filler_for_tool("bash"), FillerKind::ToolGeneric);
        assert_eq!(filler_for_tool("anything_else"), FillerKind::ToolGeneric);
    }

    #[test]
    fn filler_frames_are_numbered_filler_packets() {
        let frames = filler_frames(&[0.0; 960], 24_000).unwrap();
        assert_eq!(frames.len(), 2);
        for (chunk_id, frame) in (0_u64..).zip(&frames) {
            let decoded = crate::speech::protocol::decode_tts_frame(frame).unwrap();
            assert_eq!(decoded.chunk_id, chunk_id);
            assert_eq!(decoded.flags, FLAG_FILLER);
        }
    }

    #[tokio::test]
    async fn concurrent_voice_turns_do_not_redirect_the_first_turns_tool_audio() {
        let sink = Arc::new(VoiceSink::new());
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let (first, mut first_output) = VoiceOutput::channel(8);
        let (second, mut second_output) = VoiceOutput::channel(8);
        let first_guard = sink.install(first_id, first).unwrap();
        let second_guard = sink.install(second_id, second.clone()).unwrap();
        assert!(sink.install(first_id, second).is_err());
        assert!(sink.get(first_id).unwrap().audio(vec![1, 2, 3]).await);
        assert!(
            first_output.try_recv().is_ok(),
            "first turn audio was redirected to a different voice output"
        );
        assert!(second_output.try_recv().is_err());
        drop(first_guard);
        assert!(sink.get(first_id).is_none());
        assert!(sink.get(second_id).unwrap().audio(vec![4, 5, 6]).await);
        assert!(second_output.try_recv().is_ok());
        drop(second_guard);
        assert!(sink.get(second_id).is_none());
    }
}
