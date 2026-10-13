use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};

use crate::error::AppError;
use crate::speech::tts::TtsEngine;

const SAMPLE_RATE: u32 = 24_000;

pub(crate) struct LocalKokoroTts {
    tts: Mutex<OfflineTts>,
}

impl LocalKokoroTts {
    pub(crate) fn load(
        model: &Path,
        voices: &Path,
        provider: &str,
        num_threads: i32,
    ) -> Result<Self, AppError> {
        let tokens = sibling(model, "tokens.txt");
        let data_dir = sibling(model, "espeak-ng-data");

        for p in [model, voices, tokens.as_path()] {
            if !p.exists() {
                return Err(AppError::Config(format!(
                    "Missing TTS file: {}",
                    p.display()
                )));
            }
        }
        if !data_dir.exists() {
            return Err(AppError::Config(format!(
                "Missing espeak-ng data dir: {}",
                data_dir.display()
            )));
        }

        let dict_dir = sibling(model, "dict");
        let lexicon_us = sibling(model, "lexicon-us-en.txt");
        let lexicon_zh = sibling(model, "lexicon-zh.txt");
        let lexicon = if lexicon_us.exists() {
            let mut paths = vec![lexicon_us.to_string_lossy().into_owned()];
            if lexicon_zh.exists() {
                paths.push(lexicon_zh.to_string_lossy().into_owned());
            }
            Some(paths.join(","))
        } else {
            None
        };
        let dict_dir_opt = dict_dir
            .exists()
            .then(|| dict_dir.to_string_lossy().into_owned());

        let kokoro = OfflineTtsKokoroModelConfig {
            model: Some(model.to_string_lossy().into_owned()),
            voices: Some(voices.to_string_lossy().into_owned()),
            tokens: Some(tokens.to_string_lossy().into_owned()),
            data_dir: Some(data_dir.to_string_lossy().into_owned()),
            length_scale: 1.0,
            dict_dir: dict_dir_opt,
            lexicon,
            lang: None,
        };

        let model_config = OfflineTtsModelConfig {
            kokoro,
            num_threads,
            debug: false,
            provider: Some(provider.to_string()),
            ..Default::default()
        };

        let config = OfflineTtsConfig {
            model: model_config,
            rule_fsts: None,
            max_num_sentences: 1,
            rule_fars: None,
            silence_scale: 1.0,
        };

        let tts = OfflineTts::create(&config)
            .ok_or_else(|| AppError::Config("create Kokoro TTS failed".into()))?;

        Ok(Self {
            tts: Mutex::new(tts),
        })
    }
}

impl TtsEngine for LocalKokoroTts {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, AppError> {
        let tts = self
            .tts
            .lock()
            .map_err(|e| AppError::Internal(format!("TTS mutex poisoned: {e}")))?;
        let sid = voice_id(voice)?;
        let gen_cfg = GenerationConfig {
            silence_scale: 1.0,
            speed,
            sid,
            reference_audio: None,
            reference_sample_rate: 0,
            reference_text: None,
            num_steps: 0,
            extra: None,
        };
        let audio = tts
            .generate_with_config(text, &gen_cfg, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| AppError::Internal("Kokoro generate failed".into()))?;
        Ok(audio.samples().to_vec())
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
}

fn sibling(model_path: &Path, name: &str) -> PathBuf {
    model_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(name)
}

fn voice_id(voice: &str) -> Result<i32, AppError> {
    if let Ok(n) = voice.parse::<i32>() {
        return Ok(n);
    }
    let id = match voice {
        "af_alloy" => 0,
        "af_aoede" => 1,
        "af_bella" => 2,
        "af_heart" => 3,
        "af_jessica" => 4,
        "af_kore" => 5,
        "af_nicole" => 6,
        "af_nova" => 7,
        "af_river" => 8,
        "af_sarah" => 9,
        "af_sky" => 10,
        "am_adam" => 11,
        "am_echo" => 12,
        "am_eric" => 13,
        "am_fenrir" => 14,
        "am_liam" => 15,
        "am_michael" => 16,
        "am_onyx" => 17,
        "am_puck" => 18,
        "am_santa" => 19,
        "bf_alice" => 20,
        "bf_emma" => 21,
        "bf_isabella" => 22,
        "bf_lily" => 23,
        "bm_daniel" => 24,
        "bm_fable" => 25,
        "bm_george" => 26,
        "bm_lewis" => 27,
        "ef_dora" => 28,
        "em_alex" => 29,
        "ff_siwis" => 30,
        "hf_alpha" => 31,
        "hf_beta" => 32,
        "hm_omega" => 33,
        "hm_psi" => 34,
        "if_sara" => 35,
        "im_nicola" => 36,
        "jf_alpha" => 37,
        "jf_gongitsune" => 38,
        "jf_nezumi" => 39,
        "jf_tebukuro" => 40,
        "jm_kumo" => 41,
        "pf_dora" => 42,
        "pm_alex" => 43,
        "pm_santa" => 44,
        "zf_xiaobei" => 45,
        "zf_xiaoni" => 46,
        "zf_xiaoxiao" => 47,
        "zf_xiaoyi" => 48,
        "zm_yunjian" => 49,
        "zm_yunxi" => 50,
        "zm_yunxia" => 51,
        "zm_yunyang" => 52,
        other => {
            return Err(AppError::Validation(format!(
                "unknown Kokoro voice name `{other}`; pass an integer 0..=52"
            )));
        }
    };
    Ok(id)
}
