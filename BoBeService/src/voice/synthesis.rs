use crate::speech::protocol::{FLAG_FIRST_OF_TURN, ServerMessage, encode_tts_frame};
use crate::speech::{TtsEngine, TtsTextDelivery};
use crate::voice::opus::{PcmPacketizer, make_opus_encoder};
use crate::voice::output::VoiceOutput;
use crate::voice::protocol_helpers::send_json;
use crate::voice::spoken::SpokenScript;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

pub(crate) fn spawn_tts_task(
    tts: Arc<dyn TtsEngine>,
    turn_id: String,
    mut sentence_rx: mpsc::Receiver<String>,
    output: VoiceOutput,
    first_audio_emitted: Arc<AtomicBool>,
    voice_id: String,
    speed: f32,
    tts_text_delivery: TtsTextDelivery,
    script: Arc<SpokenScript>,
) -> tokio::task::JoinHandle<bool> {
    let sample_rate = tts.sample_rate();
    tokio::spawn(async move {
        let Some(mut encoder) = make_opus_encoder(sample_rate) else {
            return false;
        };
        let mut chunk_id: u64 = 0;
        let mut sentence_sequence: u64 = 0;
        let mut first_chunk_pending = true;
        let mut produced = false;
        while let Some(sentence) = sentence_rx.recv().await {
            let _sequence = output.audio_sequence().await;
            let tts_clone = Arc::clone(&tts);
            let sentence_owned = sentence.clone();
            let voice_id = voice_id.clone();
            let (pcm_tx, mut pcm_rx) = mpsc::channel::<Vec<f32>>(2);
            let synth = tokio::task::spawn_blocking(move || {
                tts_clone.synthesize_stream(&sentence_owned, &voice_id, speed, &mut |pcm| {
                    pcm_tx.blocking_send(pcm.to_vec()).is_ok()
                })
            });
            let sentence_index = script.begin_sentence(&sentence);
            let mut packetizer = PcmPacketizer::new(sample_rate);
            if tts_text_delivery == TtsTextDelivery::SentenceCaptions {
                send_json(
                    &output,
                    &ServerMessage::TtsText {
                        turn_id: turn_id.clone(),
                        sequence: sentence_sequence,
                        text: sentence.clone(),
                    },
                )
                .await;
                sentence_sequence = sentence_sequence.saturating_add(1);
            }
            loop {
                let pcm = pcm_rx.recv().await;
                let packets = match pcm.as_ref() {
                    Some(pcm) => packetizer.push(&mut encoder, pcm),
                    None => packetizer.finish(&mut encoder),
                };
                let packets = match packets {
                    Ok(packets) => packets,
                    Err(error) => {
                        warn!(%error, "voice.tts_encode_failed");
                        return false;
                    }
                };
                for opus_packet in packets {
                    let mut flags = 0_u8;
                    if first_chunk_pending {
                        flags |= FLAG_FIRST_OF_TURN;
                        first_chunk_pending = false;
                        first_audio_emitted.swap(true, Ordering::AcqRel);
                    }
                    let framed = encode_tts_frame(chunk_id, flags, &opus_packet);
                    if !output.audio(framed).await {
                        return produced;
                    }
                    chunk_id = chunk_id.saturating_add(1);
                    script.add_audio(Duration::from_millis(20));
                    produced = true;
                }
                if pcm.is_none() {
                    break;
                }
            }
            match synth.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    warn!(%error, "voice.tts_synth_failed");
                    return false;
                }
                Err(error) => {
                    warn!(%error, "voice.tts_join_failed");
                    return false;
                }
            }
            script.complete_sentence(sentence_index, chunk_id);
        }
        script.finish();
        produced
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    struct UntilCancelled(Mutex<Option<oneshot::Sender<()>>>);

    impl TtsEngine for UntilCancelled {
        fn sample_rate(&self) -> u32 {
            24_000
        }

        fn synthesize_stream(
            &self,
            _: &str,
            _: &str,
            _: f32,
            emit: &mut dyn FnMut(&[f32]) -> bool,
        ) -> Result<(), AppError> {
            while emit(&[0.1; 480]) {}
            if let Some(sender) = self.0.lock().unwrap().take() {
                let _ = sender.send(());
            }
            Err(AppError::Canceled("stopped".into()))
        }
    }

    #[tokio::test]
    async fn aborting_output_releases_a_synthesizer_blocked_by_backpressure() {
        let (stopped_tx, stopped_rx) = oneshot::channel();
        let tts = Arc::new(UntilCancelled(Mutex::new(Some(stopped_tx))));
        let (sentence_tx, sentence_rx) = mpsc::channel(1);
        let (output, mut frames) = VoiceOutput::channel(1);
        let next_output = output.clone();
        let task = spawn_tts_task(
            tts,
            "preview_test".into(),
            sentence_rx,
            output,
            Arc::new(AtomicBool::new(false)),
            "alba".into(),
            1.0,
            TtsTextDelivery::None,
            Arc::new(SpokenScript::default()),
        );
        sentence_tx.send("Keep talking.".into()).await.unwrap();
        drop(sentence_tx);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), frames.recv())
                .await
                .unwrap()
                .is_some()
        );
        assert!(next_output.try_audio_sequence().is_none());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(next_output.try_audio_sequence().is_some());
        tokio::time::timeout(Duration::from_secs(1), stopped_rx)
            .await
            .unwrap()
            .unwrap();
    }
}
