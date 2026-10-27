use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};

use crate::speech::{SttDescriptor, SttEngine, SttEvent, SttHealth, SttStream};

const SAMPLE_RATE: i32 = crate::speech::protocol::VOICE_CAPTURE_SAMPLE_RATE as i32;
const LEADING_PADDING: usize = SAMPLE_RATE as usize;
const TRAILING_PADDING: usize = SAMPLE_RATE as usize * 66 / 100;
const PROBE_AUDIO: Duration = Duration::from_secs(1);
const DESCRIPTOR: SttDescriptor = SttDescriptor {
    id: "sherpa_zipformer_en_20m_2023_02_17",
    version: "2023-02-17",
    languages: &["en"],
    sample_rate: SAMPLE_RATE as u32,
    partials: true,
    eou: true,
};

pub(crate) struct SherpaStreamingStt {
    recognizer: Arc<OnlineRecognizer>,
    health: SttHealth,
}

impl SherpaStreamingStt {
    pub(crate) fn load(model_dir: &Path, num_threads: i32) -> Result<Self, String> {
        let encoder = model_dir.join("encoder-epoch-99-avg-1.int8.onnx");
        let decoder = model_dir.join("decoder-epoch-99-avg-1.onnx");
        let joiner = model_dir.join("joiner-epoch-99-avg-1.int8.onnx");
        let tokens = model_dir.join("tokens.txt");
        for path in [&encoder, &decoder, &joiner, &tokens] {
            if !path.is_file() {
                return Err(format!("missing STT model file: {}", path.display()));
            }
        }

        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = Some(path_string(&encoder)?);
        config.model_config.transducer.decoder = Some(path_string(&decoder)?);
        config.model_config.transducer.joiner = Some(path_string(&joiner)?);
        config.model_config.tokens = Some(path_string(&tokens)?);
        config.model_config.provider = Some("cpu".to_owned());
        config.model_config.num_threads = num_threads.max(1);
        config.decoding_method = Some("greedy_search".into());
        config.enable_endpoint = true;
        config.rule1_min_trailing_silence = 2.4;
        config.rule2_min_trailing_silence = 1.2;
        config.rule3_min_utterance_length = 20.0;

        let recognizer = Arc::new(
            OnlineRecognizer::create(&config)
                .ok_or_else(|| "sherpa-onnx failed to create streaming recognizer".to_owned())?,
        );
        let health = probe(&recognizer, model_dir)?;
        if !health.is_viable() {
            return Err(format!(
                "STT real-time probe failed: RTF {:.3} must be below {:.3}",
                health.real_time_factor,
                SttHealth::MAX_VIABLE_REAL_TIME_FACTOR
            ));
        }
        let engine = Self { recognizer, health };
        let descriptor = engine.descriptor();
        if descriptor.sample_rate != SAMPLE_RATE as u32 || !descriptor.partials || !descriptor.eou {
            return Err("STT descriptor does not match streaming provider behavior".into());
        }
        let mut stream = engine.create_stream(false)?;
        let probe_frame = vec![0_u8; 640];
        if !stream.accept_pcm_s16le(&probe_frame)?.is_empty() {
            return Err("silent STT probe frame produced an unexpected transcript".into());
        }
        if !stream.finish()?.is_empty() {
            return Err("empty STT stream produced an unexpected transcript".into());
        }
        Ok(engine)
    }
}

impl SttEngine for SherpaStreamingStt {
    fn descriptor(&self) -> &SttDescriptor {
        &DESCRIPTOR
    }

    fn health(&self) -> &SttHealth {
        &self.health
    }

    fn create_stream(&self, endpoint_finals: bool) -> Result<Box<dyn SttStream>, String> {
        Ok(Box::new(SherpaStream {
            recognizer: Arc::clone(&self.recognizer),
            stream: primed_stream(&self.recognizer),
            last_text: String::new(),
            finished: false,
            endpoint_finals,
        }))
    }
}

struct SherpaStream {
    recognizer: Arc<OnlineRecognizer>,
    stream: OnlineStream,
    last_text: String,
    finished: bool,
    endpoint_finals: bool,
}

impl SttStream for SherpaStream {
    fn accept_pcm_s16le(&mut self, frame: &[u8]) -> Result<Vec<SttEvent>, String> {
        if self.finished {
            return Err("STT stream is already finished".into());
        }
        if !frame.len().is_multiple_of(2) {
            return Err("PCM s16le frame has an odd byte count".into());
        }
        let samples = frame
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| f32::from(i16::from_le_bytes(*sample)) / 32_768.0)
            .collect::<Vec<_>>();
        self.stream.accept_waveform(SAMPLE_RATE, &samples);
        Ok(self.decode_ready(false))
    }

    fn finish(&mut self) -> Result<Vec<SttEvent>, String> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.stream
            .accept_waveform(SAMPLE_RATE, &vec![0.0; TRAILING_PADDING]);
        self.stream.input_finished();
        Ok(self.decode_ready(true))
    }
}

impl SherpaStream {
    fn decode_ready(&mut self, finishing: bool) -> Vec<SttEvent> {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        let Some(result) = self.recognizer.get_result(&self.stream) else {
            return Vec::new();
        };
        let text = result.text.trim().to_owned();
        if text.is_empty() {
            return Vec::new();
        }
        let is_final = finishing
            || result.is_final
            || (self.endpoint_finals && self.recognizer.is_endpoint(&self.stream));
        if !is_final && text == self.last_text {
            return Vec::new();
        }
        self.last_text.clone_from(&text);
        if is_final {
            vec![SttEvent::Final(text)]
        } else {
            vec![SttEvent::Partial(text)]
        }
    }
}

fn primed_stream(recognizer: &OnlineRecognizer) -> OnlineStream {
    let stream = recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE, &vec![0.0; LEADING_PADDING]);
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    stream
}

fn probe(recognizer: &OnlineRecognizer, model_dir: &Path) -> Result<SttHealth, String> {
    let corpus_path = model_dir.join("test_wavs/0.wav");
    if corpus_path.is_file() {
        let bytes = std::fs::read(&corpus_path)
            .map_err(|error| format!("read STT probe corpus {}: {error}", corpus_path.display()))?;
        let samples = parse_pcm16_wave(&bytes)?;
        let audio = Duration::from_secs_f64(samples.len() as f64 / f64::from(SAMPLE_RATE));
        let float_samples = samples
            .into_iter()
            .map(|sample| f32::from(sample) / 32_768.0)
            .collect::<Vec<_>>();
        return probe_samples(
            recognizer,
            &primed_stream(recognizer),
            &float_samples,
            audio,
        );
    }
    let stream = primed_stream(recognizer);
    let silence = vec![0.0_f32; SAMPLE_RATE as usize * PROBE_AUDIO.as_secs() as usize];
    probe_samples(recognizer, &stream, &silence, PROBE_AUDIO)
}

fn probe_samples(
    recognizer: &OnlineRecognizer,
    stream: &OnlineStream,
    samples: &[f32],
    measured_audio: Duration,
) -> Result<SttHealth, String> {
    if measured_audio.is_zero() {
        return Err("STT probe audio must not be empty".into());
    }
    let started = Instant::now();
    stream.accept_waveform(SAMPLE_RATE, samples);
    stream.accept_waveform(SAMPLE_RATE, &vec![0.0; TRAILING_PADDING]);
    stream.input_finished();
    while recognizer.is_ready(stream) {
        recognizer.decode(stream);
    }
    Ok(SttHealth {
        real_time_factor: started.elapsed().as_secs_f64() / measured_audio.as_secs_f64(),
        measured_audio,
    })
}

fn parse_pcm16_wave(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("expected a RIFF WAVE file".into());
    }
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let kind = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| "invalid WAVE chunk size")?,
        ) as usize;
        let start = offset + 8;
        let end = start.saturating_add(size);
        if end > bytes.len() {
            return Err("truncated WAVE chunk".into());
        }
        if kind == b"data" {
            if !size.is_multiple_of(2) {
                return Err("PCM16 WAVE data has odd byte count".into());
            }
            return Ok(bytes[start..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|sample| i16::from_le_bytes(*sample))
                .collect());
        }
        offset = end + (size % 2);
    }
    Err("WAVE file has no data chunk".into())
}

fn path_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("STT model path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires BOBE_TEST_STT_MODEL_DIR and BOBE_TEST_STT_WAV"]
    fn real_streaming_model_transcribes_corpus() {
        let model_dir = std::env::var("BOBE_TEST_STT_MODEL_DIR").expect("STT model directory");
        let wav_path = std::env::var("BOBE_TEST_STT_WAV").expect("STT corpus WAV");
        let engine = SherpaStreamingStt::load(Path::new(&model_dir), 4).unwrap();
        let wav = std::fs::read(wav_path).unwrap();
        let samples = parse_pcm16_wave(&wav).unwrap();
        let mut stream = engine.create_stream(false).unwrap();
        let mut events = Vec::new();
        for chunk in samples.chunks(320) {
            let bytes = chunk
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect::<Vec<_>>();
            events.extend(stream.accept_pcm_s16le(&bytes).unwrap());
        }
        events.extend(stream.finish().unwrap());
        let final_text = events.into_iter().find_map(|event| match event {
            SttEvent::Final(text) => Some(text),
            SttEvent::Partial(_) => None,
        });
        let final_text = final_text.unwrap_or_default().to_ascii_lowercase();
        let expected =
            std::env::var("BOBE_TEST_STT_EXPECTED").unwrap_or_else(|_| "yellow lamps".into());
        assert!(
            final_text.contains(&expected.to_ascii_lowercase()),
            "{final_text}"
        );
        assert!(engine.health().is_viable());
    }
}
