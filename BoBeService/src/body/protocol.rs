use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::BodyTheme;

pub(crate) const BODY_SUBPROTOCOL_V1: &str = "bobe.body.v1";
pub(crate) const ADAPTER_SUBPROTOCOL_V1: &str = "bobe.speech-adapter.v1";
pub(crate) const BODY_MDNS_AUTH: &str = "mtls-pinned";
pub(crate) const PROTOCOL_MAJOR: u8 = 1;
pub(crate) const MIN_SUPPORTED_PROTOCOL_MINOR: u8 = 1;
pub(crate) const PROTOCOL_MINOR: u8 = 2;
pub(crate) const ADAPTER_PROTOCOL_MINOR: u8 = 1;
pub(crate) const MAX_CONTROL_BYTES: usize = 4_096;
pub(crate) const MEDIA_HEADER_LEN: usize = 20;
pub(crate) const MEDIA_FLAG_DISCONTINUITY: u16 = 0x0001;
pub(crate) const MIC_PCM_BYTES: usize = 640;
pub(crate) const SPEAKER_PCM_BYTES: usize = 960;
pub(crate) const ADAPTER_MEDIA_HEADER_LEN: usize = 40;
pub(crate) const CAPTURE_CODEC: &str = "pcm_s16le";
pub(crate) const CAPTURE_SAMPLE_RATE_HZ: u32 = 16_000;
pub(crate) const CAPTURE_CHANNELS: u8 = 1;
pub(crate) const AUDIO_FRAME_DURATION_MS: u16 = 20;
pub(crate) const MAX_PLAYBACK_CREDIT_MS: u32 = 300;
pub(crate) const MAX_LEASE_LIFETIME_MS: u32 = 600_000;
pub(crate) const SPEAKER_CODEC: &str = "pcm_s16le";
pub(crate) const SPEAKER_SAMPLE_RATE_HZ: u32 = 24_000;
pub(crate) const SPEAKER_CHANNELS: u8 = 1;

#[derive(Debug, Deserialize)]
pub(crate) struct BodyHello {
    pub(crate) protocol_major: u8,
    pub(crate) protocol_minor: u8,
    pub(crate) device_id: String,
    pub(crate) boot_id: String,
    pub(crate) firmware: String,
    pub(crate) hardware: String,
    pub(crate) capabilities: crate::body::capabilities::BodyCapabilities,
    pub(crate) limits: crate::body::capabilities::BodyLimits,
}

pub(crate) fn negotiate_protocol_minor(hello: &BodyHello) -> Option<u8> {
    (hello.protocol_major == PROTOCOL_MAJOR
        && (MIN_SUPPORTED_PROTOCOL_MINOR..=PROTOCOL_MINOR).contains(&hello.protocol_minor))
    .then_some(hello.protocol_minor)
}

#[derive(Debug, Serialize)]
pub(crate) struct BodyWelcome<'a> {
    pub(crate) r#type: &'static str,
    pub(crate) protocol_major: u8,
    pub(crate) protocol_minor: u8,
    pub(crate) session_id: &'a str,
    pub(crate) connection_generation: u64,
    pub(crate) heartbeat_interval_ms: u32,
    pub(crate) controller_epoch: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ThemeSet {
    pub(crate) r#type: &'static str,
    pub(crate) protocol_major: u8,
    pub(crate) state_version: u64,
    pub(crate) theme_id: BodyTheme,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CaptureRequest {
    pub(crate) request_id: String,
    pub(crate) stream_id: u32,
    pub(crate) trigger: String,
    #[serde(default)]
    pub(crate) wake: Option<WakeCandidateMetadata>,
    pub(crate) codec: String,
    pub(crate) sample_rate_hz: u32,
    pub(crate) channels: u8,
    pub(crate) frame_duration_ms: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WakeCandidateMetadata {
    pub(crate) confidence: f32,
    pub(crate) snr_db: f32,
    pub(crate) energy_dbfs: f32,
    #[serde(default)]
    pub(crate) detected_monotonic_us: u64,
    #[serde(default)]
    pub(crate) model_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CaptureCancel {
    pub(crate) request_id: String,
    pub(crate) stream_id: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct CaptureDecision<'a> {
    pub(crate) r#type: &'static str,
    pub(crate) protocol_major: u8,
    pub(crate) request_id: &'a str,
    pub(crate) stream_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) lease_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) turn_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) lease_ttl_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RouteDescriptor {
    pub(crate) device_id: String,
    pub(crate) connection_generation: u64,
    pub(crate) body_session_id: Uuid,
    pub(crate) lease_id: Uuid,
    pub(crate) turn_id: String,
    pub(crate) capture_stream_id: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RoutedStreamControl {
    pub(crate) lease_id: Uuid,
    pub(crate) turn_id: String,
    pub(crate) stream_id: u32,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaybackCredit {
    pub(crate) lease_id: Uuid,
    pub(crate) turn_id: String,
    pub(crate) stream_id: u32,
    pub(crate) credit_ms: u32,
    #[serde(default)]
    pub(crate) played_samples: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdapterWelcome {
    pub(crate) r#type: &'static str,
    pub(crate) protocol_major: u8,
    pub(crate) protocol_minor: u8,
    pub(crate) adapter_generation: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BodyHealthReport {
    #[serde(default)]
    pub(crate) build_number: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BodyBatteryState {
    #[serde(default)]
    pub(crate) percent: u8,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateStateReport {
    pub(crate) phase: String,
    #[serde(default)]
    pub(crate) progress_percent: u8,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

impl UpdateStateReport {
    pub(crate) const KNOWN_PHASES: [&'static str; 7] = [
        "accepted",
        "downloading",
        "verifying",
        "ready",
        "rebooting",
        "failed",
        "rejected",
    ];

    pub(crate) fn fields_are_valid(&self) -> bool {
        !self.phase.is_empty()
            && self.phase.len() <= 32
            && self.progress_percent <= 100
            && self
                .reason
                .as_deref()
                .is_none_or(|reason| !reason.is_empty() && reason.len() <= 64)
    }

    pub(crate) fn has_known_phase(&self) -> bool {
        Self::KNOWN_PHASES.contains(&self.phase.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OfferDecision {
    Send,
    AlreadyOffered,
    NotNewer,
    NotConfigured,
    NoHealthYet,
    StaleConnection,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct AdapterAudioFormat {
    pub(crate) codec: String,
    pub(crate) sample_rate_hz: u32,
    pub(crate) channels: u8,
    pub(crate) frame_duration_ms: u16,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(crate) enum AdapterOutbound {
    #[serde(rename = "capture.open")]
    CaptureOpen {
        route: RouteDescriptor,
        codec: &'static str,
        sample_rate_hz: u32,
        channels: u8,
        frame_duration_ms: u16,
    },
    #[serde(rename = "capture.close")]
    CaptureClose { route: RouteDescriptor },
    #[serde(rename = "turn.cancel")]
    TurnCancel {
        route: RouteDescriptor,
        reason: &'static str,
    },
    #[serde(rename = "turn.complete")]
    TurnComplete {
        route: RouteDescriptor,
        reason: &'static str,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum AdapterInbound {
    #[serde(rename = "adapter.hello")]
    Hello {
        protocol_major: u8,
        protocol_minor: u8,
        capture_format: AdapterAudioFormat,
    },
    #[serde(rename = "transcript.partial")]
    TranscriptPartial {
        connection_generation: u64,
        lease_id: Uuid,
        text: String,
    },
    #[serde(rename = "transcript.final")]
    TranscriptFinal {
        connection_generation: u64,
        lease_id: Uuid,
        text: String,
    },
    #[serde(rename = "transcript.empty")]
    TranscriptEmpty {
        connection_generation: u64,
        lease_id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MediaHeader {
    pub(crate) kind: u8,
    pub(crate) flags: u16,
    pub(crate) stream_id: u32,
    pub(crate) sequence: u32,
    pub(crate) monotonic_us: u64,
}

impl MediaHeader {
    pub(crate) fn parse(data: &[u8]) -> Option<Self> {
        let header = data.get(..MEDIA_HEADER_LEN)?;
        if header[0] != PROTOCOL_MAJOR {
            return None;
        }
        let kind = header[1];
        let flags = u16::from_be_bytes([header[2], header[3]]);
        let stream_id = u32::from_be_bytes(header[4..8].try_into().ok()?);
        let sequence = u32::from_be_bytes(header[8..12].try_into().ok()?);
        let monotonic_us = u64::from_be_bytes(header[12..20].try_into().ok()?);
        if !matches!(kind, 0x01 | 0x02) || flags & !MEDIA_FLAG_DISCONTINUITY != 0 || stream_id == 0
        {
            return None;
        }
        Some(Self {
            kind,
            flags,
            stream_id,
            sequence,
            monotonic_us,
        })
    }

    pub(crate) fn validate_microphone_frame(data: &[u8]) -> Option<Self> {
        let header = Self::parse(data)?;
        (header.kind == 0x01 && data.len() == MEDIA_HEADER_LEN + MIC_PCM_BYTES).then_some(header)
    }

    pub(crate) fn encode_speaker_frame(
        stream_id: u32,
        sequence: u32,
        monotonic_us: u64,
        pcm: &[u8],
    ) -> Option<Vec<u8>> {
        if stream_id == 0 || pcm.len() != SPEAKER_PCM_BYTES {
            return None;
        }
        let mut frame = Vec::with_capacity(MEDIA_HEADER_LEN + pcm.len());
        frame.push(PROTOCOL_MAJOR);
        frame.push(0x02);
        frame.extend_from_slice(&0_u16.to_be_bytes());
        frame.extend_from_slice(&stream_id.to_be_bytes());
        frame.extend_from_slice(&sequence.to_be_bytes());
        frame.extend_from_slice(&monotonic_us.to_be_bytes());
        frame.extend_from_slice(pcm);
        Some(frame)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum AdapterMediaKind {
    MicrophonePcm = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdapterMediaHeader {
    pub(crate) kind: AdapterMediaKind,
    pub(crate) connection_generation: u64,
    pub(crate) lease_id: Uuid,
    pub(crate) stream_id: u32,
    pub(crate) sequence: u32,
}

impl AdapterMediaHeader {
    pub(crate) fn encode(self, payload: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(ADAPTER_MEDIA_HEADER_LEN + payload.len());
        output.push(PROTOCOL_MAJOR);
        output.push(self.kind as u8);
        output.extend_from_slice(&0_u16.to_be_bytes());
        output.extend_from_slice(&self.connection_generation.to_be_bytes());
        output.extend_from_slice(self.lease_id.as_bytes());
        output.extend_from_slice(&self.stream_id.to_be_bytes());
        output.extend_from_slice(&self.sequence.to_be_bytes());
        output.extend_from_slice(&0_u32.to_be_bytes());
        output.extend_from_slice(payload);
        output
    }
}

pub(crate) fn control_type(value: &serde_json::Value) -> Option<&str> {
    value.get("type")?.as_str()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn theme_set_serializes_the_exact_wire_contract() {
        let value = serde_json::to_value(ThemeSet {
            r#type: "theme.set",
            protocol_major: PROTOCOL_MAJOR,
            state_version: 42,
            theme_id: BodyTheme::SolarCrown,
        })
        .expect("serialize theme.set");

        assert_eq!(
            value,
            serde_json::json!({
                "type": "theme.set",
                "protocol_major": 1,
                "state_version": 42,
                "theme_id": "solar-crown",
            })
        );
    }

    #[test]
    fn hello_negotiates_only_a_supported_minor() {
        let hello = |protocol_major, protocol_minor| BodyHello {
            protocol_major,
            protocol_minor,
            device_id: "body".into(),
            boot_id: "0".repeat(32),
            firmware: "1".into(),
            hardware: "1".into(),
            capabilities: crate::body::capabilities::BodyCapabilities::default(),
            limits: crate::body::capabilities::BodyLimits {
                max_control_bytes: 4096,
                max_playback_credit_ms: 300,
            },
        };
        assert_eq!(negotiate_protocol_minor(&hello(1, 1)), Some(1));
        assert_eq!(negotiate_protocol_minor(&hello(1, 2)), Some(2));
        assert_eq!(negotiate_protocol_minor(&hello(1, 0)), None);
        assert_eq!(negotiate_protocol_minor(&hello(1, 3)), None);
        assert_eq!(negotiate_protocol_minor(&hello(2, 1)), None);
    }

    #[test]
    fn adapter_media_header_carries_the_route() {
        let lease_id = Uuid::new_v4();
        let encoded = AdapterMediaHeader {
            kind: AdapterMediaKind::MicrophonePcm,
            connection_generation: 7,
            lease_id,
            stream_id: 42,
            sequence: 3,
        }
        .encode(&[1, 2, 3]);
        assert_eq!(encoded.len(), ADAPTER_MEDIA_HEADER_LEN + 3);
        assert_eq!(encoded[..4], [PROTOCOL_MAJOR, 1, 0, 0]);
        assert_eq!(encoded[4..12], 7_u64.to_be_bytes());
        assert_eq!(encoded[12..28], *lease_id.as_bytes());
        assert_eq!(encoded[28..32], 42_u32.to_be_bytes());
        assert_eq!(encoded[32..36], 3_u32.to_be_bytes());
        assert_eq!(encoded[36..40], [0; 4]);
        assert_eq!(encoded[40..], [1, 2, 3]);
    }

    #[test]
    fn update_state_bounds_fields_without_rejecting_additive_phases() {
        let known = UpdateStateReport {
            phase: "accepted".into(),
            progress_percent: 0,
            reason: None,
        };
        assert!(known.fields_are_valid());
        assert!(known.has_known_phase());

        let additive = UpdateStateReport {
            phase: "installing".into(),
            progress_percent: 50,
            reason: None,
        };
        assert!(additive.fields_are_valid());
        assert!(!additive.has_known_phase());

        let oversized = UpdateStateReport {
            phase: "x".repeat(33),
            progress_percent: 101,
            reason: Some("x".repeat(65)),
        };
        assert!(!oversized.fields_are_valid());
    }

    #[test]
    fn body_media_rejects_wrong_size_and_zero_stream() {
        let mut frame = vec![0_u8; MEDIA_HEADER_LEN + MIC_PCM_BYTES];
        frame[0] = PROTOCOL_MAJOR;
        frame[1] = 0x01;
        assert!(MediaHeader::validate_microphone_frame(&frame).is_none());
        frame[7] = 1;
        assert!(MediaHeader::validate_microphone_frame(&frame).is_some());
        frame[3] = 0x02;
        assert!(MediaHeader::validate_microphone_frame(&frame).is_none());
        frame[3] = 0;
        frame.pop();
        assert!(MediaHeader::validate_microphone_frame(&frame).is_none());
    }

    #[test]
    fn body_media_accepts_only_microphone_and_speaker_kinds() {
        let mut frame = vec![0_u8; MEDIA_HEADER_LEN];
        frame[0] = PROTOCOL_MAJOR;
        frame[7] = 1;
        for (kind, accepted) in [
            (0x00, false),
            (0x01, true),
            (0x02, true),
            (0x03, false),
            (0x04, false),
        ] {
            frame[1] = kind;
            assert_eq!(
                MediaHeader::parse(&frame).is_some(),
                accepted,
                "kind {kind:#04x}"
            );
        }
    }
}
