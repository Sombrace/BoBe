use opusic_c::{Application, Bitrate, Channels, Encoder as OpusEncoder, SampleRate};
use tracing::warn;

const TTS_OPUS_BITRATE_BPS: i32 = 24_000;

pub(crate) fn make_opus_encoder(sample_rate: u32) -> Option<OpusEncoder> {
    let sample_rate = match sample_rate {
        8_000 => SampleRate::Hz8000,
        12_000 => SampleRate::Hz12000,
        16_000 => SampleRate::Hz16000,
        24_000 => SampleRate::Hz24000,
        48_000 => SampleRate::Hz48000,
        other => {
            warn!(sample_rate = other, "voice.opus_unsupported_sample_rate");
            return None;
        }
    };
    let mut encoder = match OpusEncoder::new(Channels::Mono, sample_rate, Application::Voip) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = ?e, "voice.opus_encoder_failed");
            return None;
        }
    };
    if let Err(e) = encoder.set_bitrate(Bitrate::Value(TTS_OPUS_BITRATE_BPS as u32)) {
        warn!(error = ?e, "voice.opus_bitrate_failed");
    }
    Some(encoder)
}

pub(crate) fn encode_pcm_with(
    encoder: &mut OpusEncoder,
    pcm: &[f32],
    sample_rate: u32,
) -> Vec<Vec<u8>> {
    let frame_samples = (sample_rate as usize) / 50;
    let mut frames = Vec::new();
    for chunk in pcm.chunks(frame_samples) {
        let mut input = chunk.to_vec();
        if input.len() < frame_samples {
            input.resize(frame_samples, 0.0);
        }
        let mut out = vec![0_u8; 1500];
        match encoder.encode_float_to_slice(&input, &mut out) {
            Ok(n) => {
                out.truncate(n);
                frames.push(out);
            }
            Err(e) => {
                warn!(error = ?e, "voice.opus_encode_failed");
                break;
            }
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
    use super::*;
    use opusic_c::{Channels, Decoder as OpusDecoder, SampleRate};

    #[test]
    fn encoder_produces_frames_for_24khz_voice() {
        let mut encoder = make_opus_encoder(24_000).expect("encoder");
        let pcm = vec![0.0_f32; 24_000 / 10];
        let frames = encode_pcm_with(&mut encoder, &pcm, 24_000);
        assert_eq!(frames.len(), 5, "expected 5 × 20ms frames");
        for f in &frames {
            assert!(!f.is_empty(), "frame should be non-empty");
            assert!(
                f.len() < 200,
                "VoIP @ 24kbps fits in <200B per frame: {}",
                f.len()
            );
        }
    }

    #[test]
    fn round_trip_preserves_signal_within_tolerance() {
        let sr: usize = 24_000;
        let mut pcm = vec![0.0_f32; sr / 5];
        for (i, s) in pcm.iter_mut().enumerate() {
            let t = i as f32 / sr as f32;
            *s = (t * 1_000.0 * std::f32::consts::TAU).sin() * 0.5;
        }

        let mut encoder = make_opus_encoder(24_000).expect("encoder");
        let frames = encode_pcm_with(&mut encoder, &pcm, 24_000);

        let mut decoder = OpusDecoder::new(Channels::Mono, SampleRate::Hz24000).expect("decoder");
        let mut decoded_pcm: Vec<u16> = Vec::with_capacity(pcm.len());
        let mut scratch = vec![0_u16; sr / 50];
        for frame in &frames {
            let n = decoder
                .decode_to_slice(frame, &mut scratch, false)
                .expect("decode succeeds");
            decoded_pcm.extend_from_slice(&scratch[..n]);
        }
        assert!(!decoded_pcm.is_empty(), "got decoded samples");

        let in_rms = (pcm.iter().map(|&s| s * s).sum::<f32>() / pcm.len() as f32).sqrt();
        let out_rms_i = decoded_pcm
            .iter()
            .map(|&s| {
                let f = f32::from(s as i16) / 32_767.0;
                f * f
            })
            .sum::<f32>()
            / decoded_pcm.len() as f32;
        let out_rms = out_rms_i.sqrt();
        let ratio = out_rms / in_rms;
        assert!(
            ratio > 0.5 && ratio < 1.5,
            "round-trip RMS ratio out of range: {ratio} (in={in_rms}, out={out_rms})"
        );
    }

    #[test]
    fn pad_short_frame_to_full_20ms() {
        let mut encoder = make_opus_encoder(24_000).expect("encoder");
        let short = vec![0.0_f32; 100];
        let frames = encode_pcm_with(&mut encoder, &short, 24_000);
        assert_eq!(frames.len(), 1);
    }
}
