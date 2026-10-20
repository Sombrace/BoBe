use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ControlAction {
    Abort,
    Mute,
    Unmute,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VoicePhase {
    Idle,
    Listening,
    Thinking,
    Speaking,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ClientMessage {
    Hello {
        session_id: String,
        #[serde(default)]
        voice_id: Option<String>,
        #[serde(default)]
        speed: Option<f32>,
        speech_offer: Box<crate::speech::SpeechOffer>,
    },
    BargeIn {
        ts_ms: u64,
        playback_ms_played: u64,
        #[serde(default)]
        partial_text: Option<String>,
        playhead_chunk: u64,
        #[serde(default)]
        turn_id: Option<String>,
    },
    PlaybackAck {
        chunk_id: u64,
        played_ms: u64,
        #[serde(default)]
        turn_id: Option<String>,
    },
    Control {
        action: ControlAction,
    },
    TranscriptPartial {
        turn_id: String,
        text: String,
    },
    TranscriptFinal {
        turn_id: String,
        text: String,
    },
    TtsPlaybackStarted {
        turn_id: String,
        synthesis_ms: u64,
    },
    TtsPlaybackComplete {
        turn_id: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ServerMessage {
    SpeechPlan {
        plan: crate::speech::SpeechPlan,
    },
    CaptureCredit {
        stream_id: u32,
        credit_frames: u32,
    },
    SttPartial {
        turn_id: String,
        text: String,
    },
    SttFinal {
        turn_id: String,
        text: String,
    },
    State {
        phase: VoicePhase,
        turn_id: String,
    },
    TranscriptFinal {
        turn_id: String,
        text: String,
    },
    TtsEnd {
        turn_id: String,
    },
    TtsText {
        turn_id: String,
        sequence: u64,
        text: String,
    },
    Truncate {
        turn_id: String,
        keep_ms: u64,
    },
    Error {
        code: String,
        message: String,
    },
}

pub(crate) const VOICE_CAPTURE_HEADER_LEN: usize = 13;
pub(crate) const VOICE_CAPTURE_PCM_BYTES: usize = 640;
pub(crate) const VOICE_CAPTURE_SAMPLE_RATE: u32 = 16_000;
pub(crate) const VOICE_CAPTURE_FRAME_SAMPLES: usize = 320;
const _: () = assert!(VOICE_CAPTURE_PCM_BYTES == VOICE_CAPTURE_FRAME_SAMPLES * size_of::<i16>());

pub(crate) struct VoiceCaptureFrame<'a> {
    pub(crate) stream_id: u32,
    pub(crate) sequence: u32,
    pub(crate) flags: u8,
    pub(crate) payload: &'a [u8],
}

pub(crate) fn decode_voice_capture_frame(bytes: &[u8]) -> Option<VoiceCaptureFrame<'_>> {
    if bytes.len() != VOICE_CAPTURE_HEADER_LEN + VOICE_CAPTURE_PCM_BYTES {
        return None;
    }
    let stream_id = u32::from_be_bytes(bytes[..4].try_into().ok()?);
    let sequence = u32::from_be_bytes(bytes[4..8].try_into().ok()?);
    let flags = bytes[8];
    let reserved = u32::from_be_bytes(bytes[9..13].try_into().ok()?);
    if stream_id == 0 || reserved != 0 || flags & !0b0000_0001 != 0 {
        return None;
    }
    Some(VoiceCaptureFrame {
        stream_id,
        sequence,
        flags,
        payload: &bytes[VOICE_CAPTURE_HEADER_LEN..],
    })
}

pub(crate) const TTS_FRAME_HEADER_LEN: usize = 9;
pub(crate) const FLAG_FILLER: u8 = 0b0000_0001;
pub(crate) const FLAG_FIRST_OF_TURN: u8 = 0b0000_0010;

pub(crate) struct DecodedTtsFrame<'a> {
    pub(crate) chunk_id: u64,
    pub(crate) flags: u8,
    pub(crate) payload: &'a [u8],
}

pub(crate) fn decode_tts_frame(bytes: &[u8]) -> Option<DecodedTtsFrame<'_>> {
    if bytes.len() <= TTS_FRAME_HEADER_LEN {
        return None;
    }
    let chunk_id = u64::from_be_bytes(bytes[..8].try_into().ok()?);
    Some(DecodedTtsFrame {
        chunk_id,
        flags: bytes[8],
        payload: &bytes[TTS_FRAME_HEADER_LEN..],
    })
}

pub(crate) fn encode_tts_frame(chunk_id: u64, flags: u8, opus: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(TTS_FRAME_HEADER_LEN + opus.len());
    buf.extend_from_slice(&chunk_id.to_be_bytes());
    buf.push(flags);
    buf.extend_from_slice(opus);
    buf
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
    use super::*;
    use serde_json::json;

    #[test]
    fn voice_capture_frame_validates_identity_sequence_and_pcm_size() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&7_u32.to_be_bytes());
        frame.extend_from_slice(&3_u32.to_be_bytes());
        frame.push(0);
        frame.extend_from_slice(&0_u32.to_be_bytes());
        frame.extend_from_slice(&vec![0_u8; VOICE_CAPTURE_PCM_BYTES]);
        let decoded = decode_voice_capture_frame(&frame).unwrap();
        assert_eq!(decoded.stream_id, 7);
        assert_eq!(decoded.sequence, 3);
        assert_eq!(decoded.payload.len(), 640);
        frame.pop();
        assert!(decode_voice_capture_frame(&frame).is_none());
    }

    #[test]
    fn voice_capture_frame_rejects_zero_stream_reserved_and_unknown_flags() {
        let mut frame = vec![0_u8; VOICE_CAPTURE_HEADER_LEN + VOICE_CAPTURE_PCM_BYTES];
        assert!(decode_voice_capture_frame(&frame).is_none());
        frame[3] = 1;
        frame[8] = 2;
        assert!(decode_voice_capture_frame(&frame).is_none());
        frame[8] = 0;
        frame[12] = 1;
        assert!(decode_voice_capture_frame(&frame).is_none());
    }

    const MAC_OFFER: &str = r#"{
        "language":"en",
        "playback_rates":[24000],
        "endpoint_stt":[{"id":"apple_speech_analyzer","languages":["en"],"partials":true,"eou":true,"voices":[],"sample_rates":[],"ready":true}],
        "endpoint_tts":[],
        "audio_uplink":false,
        "overrides":{"stt":"auto","tts":"daemon"},
        "remote_audio_consent":false
    }"#;

    const MAC_OFFER_0_9_0: &str = r#"{
        "endpoint_id":"00000000-0000-0000-0000-000000000001",
        "endpoint_kind":"mac_app",
        "language":"en",
        "playback_rates":[24000],
        "endpoint_stt":[{"id":"apple_speech_analyzer","languages":["en"],"partials":true,"eou":true,"voices":[],"sample_rates":[],"ready":true}],
        "endpoint_tts":[],
        "audio_uplink":false,
        "overrides":{"stt":"auto","tts":"daemon"},
        "daemon_is_non_loopback":false,
        "remote_audio_consent":false
    }"#;

    #[test]
    fn capture_frames_carry_twenty_milliseconds_of_sixteen_bit_audio() {
        assert_eq!(
            VOICE_CAPTURE_FRAME_SAMPLES as u32 * 1_000 / VOICE_CAPTURE_SAMPLE_RATE,
            20
        );
        assert_eq!(VOICE_CAPTURE_PCM_BYTES, VOICE_CAPTURE_FRAME_SAMPLES * 2);
    }

    #[test]
    fn hello_requires_a_speech_offer() {
        let raw = r#"{"type":"hello","session_id":"abc"}"#;
        assert!(serde_json::from_str::<ClientMessage>(raw).is_err());
    }

    #[test]
    fn hello_deserializes_capability_offer() {
        let raw = format!(r#"{{"type":"hello","session_id":"abc","speech_offer":{MAC_OFFER}}}"#);
        let parsed: ClientMessage = serde_json::from_str(&raw).unwrap();
        match parsed {
            ClientMessage::Hello {
                session_id,
                voice_id,
                speed,
                speech_offer,
            } => {
                assert_eq!(session_id, "abc");
                assert_eq!(voice_id, None);
                assert_eq!(speed, None);
                assert_eq!(speech_offer.language, "en");
                assert_eq!(speech_offer.endpoint_stt[0].id, "apple_speech_analyzer");
            }
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    #[test]
    fn hello_from_a_0_9_0_app_still_parses_and_ignores_client_identity_claims() {
        let raw =
            format!(r#"{{"type":"hello","session_id":"abc","speech_offer":{MAC_OFFER_0_9_0}}}"#);
        let parsed: ClientMessage = serde_json::from_str(&raw).unwrap();
        let ClientMessage::Hello { speech_offer, .. } = parsed else {
            panic!("expected Hello");
        };
        let current =
            format!(r#"{{"type":"hello","session_id":"abc","speech_offer":{MAC_OFFER}}}"#);
        let ClientMessage::Hello {
            speech_offer: current,
            ..
        } = serde_json::from_str::<ClientMessage>(&current).unwrap()
        else {
            panic!("expected Hello");
        };
        assert_eq!(speech_offer, current);
    }

    #[test]
    fn hello_deserializes_voice_preferences() {
        let raw = format!(
            r#"{{"type":"hello","session_id":"abc","voice_id":"marius","speed":1.2,"speech_offer":{MAC_OFFER}}}"#
        );
        let parsed: ClientMessage = serde_json::from_str(&raw).unwrap();
        match parsed {
            ClientMessage::Hello {
                voice_id, speed, ..
            } => {
                assert_eq!(voice_id, Some("marius".into()));
                assert_eq!(speed, Some(1.2));
            }
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    #[test]
    fn transcript_partial_deserialize() {
        let raw = r#"{"type":"transcript_partial","turn_id":"voice_t1","text":"hello wor"}"#;
        let parsed: ClientMessage = serde_json::from_str(raw).unwrap();
        match parsed {
            ClientMessage::TranscriptPartial { turn_id, text } => {
                assert_eq!(turn_id, "voice_t1");
                assert_eq!(text, "hello wor");
            }
            other => panic!("expected TranscriptPartial, got {other:?}"),
        }
    }

    #[test]
    fn client_tts_text_serializes_with_sequence() {
        let value = serde_json::to_value(ServerMessage::TtsText {
            turn_id: "turn-1".into(),
            sequence: 2,
            text: "Hello.".into(),
        })
        .unwrap();
        assert_eq!(value["type"], "tts_text");
        assert_eq!(value["turn_id"], "turn-1");
        assert_eq!(value["sequence"], 2);
        assert_eq!(value["text"], "Hello.");
    }

    #[test]
    fn transcript_final_deserialize() {
        let raw = r#"{"type":"transcript_final","turn_id":"voice_t1","text":"hello world"}"#;
        let parsed: ClientMessage = serde_json::from_str(raw).unwrap();
        match parsed {
            ClientMessage::TranscriptFinal { turn_id, text } => {
                assert_eq!(turn_id, "voice_t1");
                assert_eq!(text, "hello world");
            }
            other => panic!("expected TranscriptFinal, got {other:?}"),
        }
    }

    #[test]
    fn barge_in_deserialize() {
        let raw = r#"{"type":"barge_in","ts_ms":1000,"playback_ms_played":420,"playhead_chunk":7}"#;
        let parsed: ClientMessage = serde_json::from_str(raw).unwrap();
        match parsed {
            ClientMessage::BargeIn {
                ts_ms,
                playback_ms_played,
                partial_text,
                playhead_chunk,
                turn_id,
            } => {
                assert_eq!(ts_ms, 1_000);
                assert_eq!(playback_ms_played, 420);
                assert_eq!(partial_text, None);
                assert_eq!(playhead_chunk, 7);
                assert_eq!(turn_id, None);
            }
            other => panic!("expected BargeIn, got {other:?}"),
        }
    }

    #[test]
    fn overlap_and_playback_evidence_may_name_their_reply() {
        let raw = r#"{"type":"barge_in","ts_ms":1000,"playback_ms_played":420,"playhead_chunk":7,"turn_id":"voice_a"}"#;
        std::assert_matches!(
            serde_json::from_str::<ClientMessage>(raw).unwrap(),
            ClientMessage::BargeIn { turn_id: Some(ref turn), .. } if turn == "voice_a"
        );
        let legacy = r#"{"type":"playback_ack","chunk_id":3,"played_ms":900}"#;
        std::assert_matches!(
            serde_json::from_str::<ClientMessage>(legacy).unwrap(),
            ClientMessage::PlaybackAck {
                turn_id: None,
                played_ms: 900,
                ..
            }
        );
        let named = r#"{"type":"playback_ack","chunk_id":3,"played_ms":900,"turn_id":"voice_a"}"#;
        std::assert_matches!(
            serde_json::from_str::<ClientMessage>(named).unwrap(),
            ClientMessage::PlaybackAck { turn_id: Some(ref turn), .. } if turn == "voice_a"
        );
    }

    #[test]
    fn barge_in_requires_the_playhead() {
        let raw = r#"{"type":"barge_in","ts_ms":1000,"playback_ms_played":420}"#;
        assert!(serde_json::from_str::<ClientMessage>(raw).is_err());
    }

    #[test]
    fn barge_in_deserialize_with_atomic_partial_evidence() {
        let raw = r#"{
            "type":"barge_in",
            "ts_ms":1000,
            "playback_ms_played":420,
            "partial_text":"please stop now",
            "playhead_chunk":3
        }"#;
        let parsed: ClientMessage = serde_json::from_str(raw).unwrap();
        std::assert_matches!(
            parsed,
            ClientMessage::BargeIn {
                partial_text: Some(ref text),
                ..
            } if text == "please stop now"
        );
    }

    #[test]
    fn control_abort_deserialize() {
        let raw = r#"{"type":"control","action":"abort"}"#;
        let parsed: ClientMessage = serde_json::from_str(raw).unwrap();
        std::assert_matches!(
            parsed,
            ClientMessage::Control {
                action: ControlAction::Abort
            }
        );
    }

    #[test]
    fn state_serialize_uses_snake_case() {
        let msg = ServerMessage::State {
            phase: VoicePhase::Speaking,
            turn_id: "voice_xyz".into(),
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            v,
            json!({"type":"state","phase":"speaking","turn_id":"voice_xyz"})
        );
    }

    #[test]
    fn transcript_final_serialize() {
        let msg = ServerMessage::TranscriptFinal {
            turn_id: "voice_xyz".into(),
            text: "hello world".into(),
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            v,
            json!({"type":"transcript_final","turn_id":"voice_xyz","text":"hello world"})
        );
    }

    #[test]
    fn tts_end_serialize() {
        let msg = ServerMessage::TtsEnd {
            turn_id: "voice_xyz".into(),
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v, json!({"type":"tts_end","turn_id":"voice_xyz"}));
    }

    #[test]
    fn truncate_serialize() {
        let msg = ServerMessage::Truncate {
            turn_id: "voice_xyz".into(),
            keep_ms: 1_500,
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            v,
            json!({"type":"truncate","turn_id":"voice_xyz","keep_ms":1500})
        );
    }

    #[test]
    fn tts_frame_header_round_trip() {
        let opus = vec![1_u8, 2, 3, 4, 5];
        let chunk_id: u64 = 0x0102_0304_0506_0708;
        let flags = FLAG_FILLER | FLAG_FIRST_OF_TURN;
        let framed = encode_tts_frame(chunk_id, flags, &opus);

        assert_eq!(framed.len(), TTS_FRAME_HEADER_LEN + opus.len());
        assert_eq!(
            &framed[0..8],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert_eq!(framed[8], flags);
        assert_eq!(&framed[9..], &opus[..]);
    }

    #[test]
    fn invalid_type_errors() {
        let raw = r#"{"type":"nonexistent_type"}"#;
        let result: Result<ClientMessage, _> = serde_json::from_str(raw);
        assert!(result.is_err());
    }
}
