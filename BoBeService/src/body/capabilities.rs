use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BodyCapabilities {
    pub(crate) microphone: bool,
    pub(crate) speaker: bool,
    pub(crate) battery: bool,
    pub(crate) aec: bool,
    pub(crate) wake_word: bool,
    pub(crate) text_input: bool,
    pub(crate) text_output: bool,
    pub(crate) actions: bool,
    pub(crate) sensors: bool,
    pub(crate) display: Option<BodyDisplayCapability>,
    pub(crate) text_display: Option<TextDisplayCapability>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextDisplayCapability {
    pub(crate) columns: u16,
    pub(crate) rows: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyDisplayCapability {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) face_schema: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyLimits {
    pub(crate) max_control_bytes: u32,
    pub(crate) max_playback_credit_ms: u32,
}

impl BodyCapabilities {
    pub(crate) fn authorized_by(&self, minor: u8, approved: &serde_json::Value) -> bool {
        if minor < 2 && approved.as_object().is_some_and(serde_json::Map::is_empty) {
            return true;
        }
        [
            ("microphone", self.microphone),
            ("speaker", self.speaker),
            ("text_input", self.text_input),
            ("text_output", self.text_output),
            ("actions", self.actions),
            ("sensors", self.sensors),
        ]
        .into_iter()
        .all(|(name, requested)| {
            !requested || approved.get(name).and_then(serde_json::Value::as_bool) == Some(true)
        })
    }
    pub(crate) fn validates_registration(&self, minor: u8, limits: &BodyLimits) -> bool {
        if !(1024..=crate::body::protocol::MAX_CONTROL_BYTES as u32)
            .contains(&limits.max_control_bytes)
            || limits.max_playback_credit_ms > crate::body::protocol::MAX_PLAYBACK_CREDIT_MS
            || ((self.aec || self.wake_word) && !self.microphone)
            || (self.speaker && limits.max_playback_credit_ms < 20)
            || self.display.as_ref().is_some_and(|display| {
                display.width == 0
                    || display.height == 0
                    || display.width > 4096
                    || display.height > 4096
                    || display.face_schema != 1
            })
            || self.text_display.as_ref().is_some_and(|display| {
                !self.text_output
                    || !(1..=240).contains(&display.columns)
                    || !(1..=240).contains(&display.rows)
            })
        {
            return false;
        }
        if minor < 2 {
            return !self.text_input
                && !self.text_output
                && !self.actions
                && !self.sensors
                && self.text_display.is_none()
                && self.validates_audio_turn(limits);
        }
        self.validates_audio_turn(limits)
            || self.text_input
            || self.text_output
            || self.actions
            || self.sensors
    }

    pub(crate) fn validates_audio_turn(&self, limits: &BodyLimits) -> bool {
        self.microphone
            && self.speaker
            && limits.max_control_bytes >= 1_024
            && limits.max_playback_credit_ms
                >= crate::body::protocol::AUDIO_FRAME_DURATION_MS.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_body() -> BodyCapabilities {
        BodyCapabilities {
            microphone: true,
            speaker: true,
            ..BodyCapabilities::default()
        }
    }

    fn voice_limits() -> BodyLimits {
        BodyLimits {
            max_control_bytes: crate::body::protocol::MAX_CONTROL_BYTES as u32,
            max_playback_credit_ms: crate::body::protocol::MAX_PLAYBACK_CREDIT_MS,
        }
    }

    #[test]
    fn non_audio_surfaces_require_minor_two_and_explicit_capabilities() {
        let caps = BodyCapabilities {
            text_input: true,
            text_output: true,
            text_display: Some(TextDisplayCapability {
                columns: 20,
                rows: 4,
            }),
            ..BodyCapabilities::default()
        };
        let limits = BodyLimits {
            max_control_bytes: 4096,
            max_playback_credit_ms: 0,
        };
        assert!(caps.validates_registration(2, &limits));
        assert!(!caps.validates_registration(1, &limits));
        assert!(!caps.validates_audio_turn(&limits));
        assert!(!BodyCapabilities::default().validates_registration(2, &limits));
        let sensor = BodyCapabilities {
            sensors: true,
            ..BodyCapabilities::default()
        };
        assert!(sensor.validates_registration(2, &limits));
    }

    #[test]
    fn a_voice_body_registers_and_impossible_claims_do_not() {
        assert!(voice_body().validates_registration(1, &voice_limits()));
        let impossible = BodyCapabilities {
            sensors: true,
            aec: true,
            ..BodyCapabilities::default()
        };
        assert!(!impossible.validates_registration(2, &voice_limits()));
    }

    #[test]
    fn sensor_enrollment_does_not_grant_agent_input() {
        let approved = serde_json::json!({"sensors":true});
        let mut declared = BodyCapabilities {
            sensors: true,
            ..BodyCapabilities::default()
        };
        assert!(declared.authorized_by(2, &approved));
        declared.text_input = true;
        assert!(!declared.authorized_by(2, &approved));
        declared.text_input = false;
        declared.actions = true;
        assert!(!declared.authorized_by(2, &approved));
        assert!(!voice_body().authorized_by(1, &approved));
        assert!(voice_body().authorized_by(1, &serde_json::json!({})));
        assert!(
            voice_body()
                .authorized_by(1, &serde_json::json!({"microphone": true, "speaker": true}))
        );
    }
}
