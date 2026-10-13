use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::voice::session::VoiceDefaults;

pub(crate) const ENDPOINT_STT_ENGINE: &str = "voice_v1_endpoint_stt";
pub(crate) const APPLE_SPEECH_ANALYZER_ENGINE: &str = "apple_speech_analyzer";
pub(crate) const FLUIDAUDIO_PARAKEET_ENGINE: &str = "fluidaudio_parakeet_eou";
pub(crate) const FLUIDAUDIO_NEMOTRON_ENGINE: &str = "fluidaudio_nemotron";
pub(crate) const SERVER_POCKET_ENGINE: &str = "server_pocket";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpeechEngineOffer {
    pub(crate) id: String,
    pub(crate) languages: Vec<String>,
    #[serde(default)]
    pub(crate) partials: bool,
    #[serde(default)]
    pub(crate) eou: bool,
    #[serde(default)]
    pub(crate) voices: Vec<String>,
    #[serde(default)]
    pub(crate) sample_rates: Vec<u32>,
    pub(crate) ready: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpeechPlacementOverride {
    #[default]
    Auto,
    Endpoint,
    Daemon,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpeechOverrides {
    #[serde(default)]
    pub(crate) stt: SpeechPlacementOverride,
    #[serde(default)]
    pub(crate) tts: SpeechPlacementOverride,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpeechOffer {
    pub(crate) endpoint_id: Uuid,
    pub(crate) endpoint_kind: String,
    pub(crate) language: String,
    pub(crate) playback_rates: Vec<u32>,
    pub(crate) endpoint_stt: Vec<SpeechEngineOffer>,
    pub(crate) endpoint_tts: Vec<SpeechEngineOffer>,
    #[serde(default)]
    pub(crate) audio_uplink: bool,
    #[serde(default)]
    pub(crate) overrides: SpeechOverrides,
    #[serde(default)]
    pub(crate) daemon_is_non_loopback: bool,
    #[serde(default)]
    pub(crate) remote_audio_consent: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpeechSite {
    Endpoint,
    Daemon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpeechUplink {
    Transcript,
    Audio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpeechDownlink {
    Text,
    Audio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpeechBoundaryOwner {
    Endpoint,
    Daemon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TtsTextDelivery {
    None,
    SentenceCaptions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpeechPlanReason {
    BodyV1DaemonSpeech,
    VoiceV2EndpointSpeech,
    VoiceV2EndpointSttDaemonTts,
    VoiceV2DaemonSttEndpointTts,
    VoiceV2DaemonSpeech,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SpeechPlan {
    pub(crate) plan_id: String,
    pub(crate) stt_site: SpeechSite,
    pub(crate) stt_engine_id: String,
    pub(crate) tts_site: SpeechSite,
    pub(crate) tts_engine_id: String,
    pub(crate) voice_id: String,
    pub(crate) uplink: SpeechUplink,
    pub(crate) downlink: SpeechDownlink,
    pub(crate) eou_owner: SpeechBoundaryOwner,
    pub(crate) wake_owner: SpeechBoundaryOwner,
    pub(crate) tts_text_delivery: TtsTextDelivery,
    pub(crate) reason: SpeechPlanReason,
}

impl SpeechPlan {
    pub(crate) fn resolve_voice_v2(
        offer: &SpeechOffer,
        defaults: &VoiceDefaults,
        voice_id: Option<String>,
        daemon_stt: Option<&crate::speech::SttDescriptor>,
        daemon_tts_ready: bool,
    ) -> Result<Self, SpeechPlanError> {
        if offer.endpoint_kind.is_empty() || offer.language.is_empty() {
            return Err(SpeechPlanError::InvalidOffer(
                "endpoint kind and language are required".into(),
            ));
        }
        if !offer
            .playback_rates
            .contains(&crate::constants::voice_wire::TTS_OUTPUT_SAMPLE_RATE)
        {
            return Err(SpeechPlanError::InvalidOffer(
                "24 kHz playback is required".into(),
            ));
        }
        let endpoint_stt = offer.endpoint_stt.iter().find(|engine| {
            is_allowed_endpoint_stt(&engine.id)
                && engine.ready
                && engine.partials
                && engine.eou
                && engine.languages.iter().any(|language| {
                    language == &offer.language
                        || language.split('-').next() == offer.language.split('-').next()
                })
        });
        let daemon_stt = daemon_stt.filter(|descriptor| {
            offer.audio_uplink
                && descriptor.languages.iter().any(|language| {
                    *language == offer.language
                        || language.split('-').next() == offer.language.split('-').next()
                })
        });
        let endpoint_tts = offer.endpoint_tts.iter().find(|engine| {
            engine.ready
                && engine.languages.iter().any(|language| {
                    language == &offer.language
                        || language.split('-').next() == offer.language.split('-').next()
                })
                && engine
                    .voices
                    .iter()
                    .any(|voice| matches!(voice.as_str(), "supertonic_f1" | "supertonic_m1"))
                && engine
                    .sample_rates
                    .contains(&crate::constants::voice_wire::TTS_OUTPUT_SAMPLE_RATE)
        });

        let (stt_site, stt_engine_id) = resolve_site(
            offer.overrides.stt,
            endpoint_stt.map(|engine| engine.id.as_str()),
            daemon_stt.map(|descriptor| descriptor.id),
            "STT",
        )?;
        if stt_site == SpeechSite::Daemon
            && offer.daemon_is_non_loopback
            && !offer.remote_audio_consent
        {
            return Err(SpeechPlanError::RemoteAudioConsentRequired);
        }
        let (tts_site, tts_engine_id) = resolve_site(
            offer.overrides.tts,
            endpoint_tts.map(|engine| engine.id.as_str()),
            (daemon_tts_ready && offer.language.split('-').next() == Some("en"))
                .then_some(SERVER_POCKET_ENGINE),
            "TTS",
        )?;
        let reason = match (stt_site, tts_site) {
            (SpeechSite::Endpoint, SpeechSite::Endpoint) => SpeechPlanReason::VoiceV2EndpointSpeech,
            (SpeechSite::Endpoint, SpeechSite::Daemon) => {
                SpeechPlanReason::VoiceV2EndpointSttDaemonTts
            }
            (SpeechSite::Daemon, SpeechSite::Endpoint) => {
                SpeechPlanReason::VoiceV2DaemonSttEndpointTts
            }
            (SpeechSite::Daemon, SpeechSite::Daemon) => SpeechPlanReason::VoiceV2DaemonSpeech,
        };
        let daemon_voice = crate::speech::providers::pocket::migrate_voice(
            voice_id.as_deref().unwrap_or(&defaults.persona),
        );
        if tts_site == SpeechSite::Daemon
            && !crate::speech::providers::pocket::VOICES.contains(&daemon_voice)
        {
            return Err(SpeechPlanError::InvalidOffer(format!(
                "Unknown Pocket TTS voice: {daemon_voice}"
            )));
        }
        let plan = Self {
            plan_id: Uuid::new_v4().to_string(),
            stt_site,
            stt_engine_id,
            tts_site,
            tts_engine_id,
            voice_id: if tts_site == SpeechSite::Endpoint {
                endpoint_tts
                    .and_then(|engine| engine.voices.first())
                    .cloned()
                    .ok_or_else(|| {
                        SpeechPlanError::InvalidOffer("endpoint TTS voice is required".into())
                    })?
            } else {
                daemon_voice.to_owned()
            },
            uplink: if stt_site == SpeechSite::Endpoint {
                SpeechUplink::Transcript
            } else {
                SpeechUplink::Audio
            },
            downlink: if tts_site == SpeechSite::Endpoint {
                SpeechDownlink::Text
            } else {
                SpeechDownlink::Audio
            },
            eou_owner: stt_site.into(),
            wake_owner: SpeechBoundaryOwner::Endpoint,
            tts_text_delivery: TtsTextDelivery::None,
            reason,
        };
        if !plan.is_coherent_v2() {
            return Err(SpeechPlanError::InvalidOffer(
                "resolved speech plan is incoherent".into(),
            ));
        }
        Ok(plan)
    }

    pub(crate) fn body_v1(defaults: &VoiceDefaults) -> Self {
        Self {
            plan_id: Uuid::new_v4().to_string(),
            stt_site: SpeechSite::Endpoint,
            stt_engine_id: ENDPOINT_STT_ENGINE.to_owned(),
            tts_site: SpeechSite::Daemon,
            tts_engine_id: SERVER_POCKET_ENGINE.to_owned(),
            voice_id: defaults.persona.clone(),
            uplink: SpeechUplink::Transcript,
            downlink: SpeechDownlink::Audio,
            eou_owner: SpeechBoundaryOwner::Endpoint,
            wake_owner: SpeechBoundaryOwner::Endpoint,
            tts_text_delivery: TtsTextDelivery::SentenceCaptions,
            reason: SpeechPlanReason::BodyV1DaemonSpeech,
        }
    }

    pub(crate) fn uses_endpoint_tts(&self) -> bool {
        self.tts_site == SpeechSite::Endpoint
    }

    fn is_coherent_v2(&self) -> bool {
        let stt = match self.stt_site {
            SpeechSite::Endpoint => {
                is_allowed_endpoint_stt(&self.stt_engine_id)
                    && self.uplink == SpeechUplink::Transcript
                    && self.eou_owner == SpeechBoundaryOwner::Endpoint
            }
            SpeechSite::Daemon => {
                self.uplink == SpeechUplink::Audio && self.eou_owner == SpeechBoundaryOwner::Daemon
            }
        };
        let tts = match self.tts_site {
            SpeechSite::Endpoint => self.downlink == SpeechDownlink::Text,
            SpeechSite::Daemon => self.downlink == SpeechDownlink::Audio,
        };
        !self.plan_id.is_empty()
            && !self.stt_engine_id.is_empty()
            && !self.tts_engine_id.is_empty()
            && !self.voice_id.is_empty()
            && self.wake_owner == SpeechBoundaryOwner::Endpoint
            && self.tts_text_delivery == TtsTextDelivery::None
            && stt
            && tts
    }
}

impl From<SpeechSite> for SpeechBoundaryOwner {
    fn from(site: SpeechSite) -> Self {
        match site {
            SpeechSite::Endpoint => Self::Endpoint,
            SpeechSite::Daemon => Self::Daemon,
        }
    }
}

fn is_allowed_endpoint_stt(engine_id: &str) -> bool {
    matches!(
        engine_id,
        APPLE_SPEECH_ANALYZER_ENGINE | FLUIDAUDIO_PARAKEET_ENGINE | FLUIDAUDIO_NEMOTRON_ENGINE
    )
}

fn resolve_site(
    requested: SpeechPlacementOverride,
    endpoint_engine: Option<&str>,
    daemon_engine: Option<&str>,
    capability: &str,
) -> Result<(SpeechSite, String), SpeechPlanError> {
    let selected = match requested {
        SpeechPlacementOverride::Auto => endpoint_engine
            .map(|engine| (SpeechSite::Endpoint, engine))
            .or_else(|| daemon_engine.map(|engine| (SpeechSite::Daemon, engine))),
        SpeechPlacementOverride::Endpoint => {
            endpoint_engine.map(|engine| (SpeechSite::Endpoint, engine))
        }
        SpeechPlacementOverride::Daemon => daemon_engine.map(|engine| (SpeechSite::Daemon, engine)),
    };
    selected
        .map(|(site, engine)| (site, engine.to_owned()))
        .ok_or_else(|| SpeechPlanError::NoSpeechPath(capability.to_owned()))
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SpeechPlanError {
    InvalidOffer(String),
    NoSpeechPath(String),
    RemoteAudioConsentRequired,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> VoiceDefaults {
        VoiceDefaults {
            enabled: true,
            persona: "alba".into(),
            speed: 1.0,
        }
    }

    fn endpoint_engine(
        id: &str,
        languages: &[&str],
        partials: bool,
        eou: bool,
        sample_rates: &[u32],
        voices: &[&str],
    ) -> SpeechEngineOffer {
        SpeechEngineOffer {
            id: id.into(),
            languages: languages.iter().map(ToString::to_string).collect(),
            partials,
            eou,
            voices: voices.iter().map(ToString::to_string).collect(),
            sample_rates: sample_rates.to_vec(),
            ready: true,
        }
    }

    fn offer() -> SpeechOffer {
        SpeechOffer {
            endpoint_id: Uuid::new_v4(),
            endpoint_kind: "mac_app".into(),
            language: "en".into(),
            playback_rates: vec![24_000],
            endpoint_stt: vec![endpoint_engine(
                FLUIDAUDIO_PARAKEET_ENGINE,
                &["en"],
                true,
                true,
                &[],
                &[],
            )],
            endpoint_tts: vec![endpoint_engine(
                "client_supertonic",
                &["en"],
                false,
                false,
                &[24_000],
                &["supertonic_f1"],
            )],
            audio_uplink: false,
            overrides: SpeechOverrides::default(),
            daemon_is_non_loopback: false,
            remote_audio_consent: false,
        }
    }

    fn daemon_stt() -> crate::speech::SttDescriptor {
        crate::speech::SttDescriptor {
            id: "daemon_zipformer",
            version: "1",
            languages: &["en"],
            sample_rate: 16_000,
            partials: true,
            eou: true,
        }
    }

    #[test]
    fn v2_auto_prefers_ready_endpoint_speech() {
        let plan =
            SpeechPlan::resolve_voice_v2(&offer(), &defaults(), None, Some(&daemon_stt()), true)
                .unwrap();
        assert_eq!(plan.stt_site, SpeechSite::Endpoint);
        assert_eq!(plan.tts_site, SpeechSite::Endpoint);
        assert_eq!(plan.uplink, SpeechUplink::Transcript);
        assert_eq!(plan.downlink, SpeechDownlink::Text);
    }

    #[test]
    fn v2_accepts_apple_speech_as_endpoint_only() {
        let mut offer = offer();
        offer.endpoint_stt[0].id = APPLE_SPEECH_ANALYZER_ENGINE.into();
        let plan = SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).unwrap();
        assert_eq!(plan.stt_site, SpeechSite::Endpoint);
        assert_eq!(plan.stt_engine_id, APPLE_SPEECH_ANALYZER_ENGINE);
        assert_eq!(plan.uplink, SpeechUplink::Transcript);
        assert_eq!(plan.eou_owner, SpeechBoundaryOwner::Endpoint);
    }

    #[test]
    fn v2_rejects_unrecognized_endpoint_stt_identity() {
        let mut offer = offer();
        offer.endpoint_stt[0].id = "invented_endpoint_stt".into();
        assert_eq!(
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).unwrap_err(),
            SpeechPlanError::NoSpeechPath("STT".into())
        );
    }

    #[test]
    fn v2_endpoint_tts_requires_matching_language() {
        let mut offer = offer();
        offer.language = "ja".into();
        offer.endpoint_stt[0].languages = vec!["ja".into()];
        offer.overrides.tts = SpeechPlacementOverride::Endpoint;
        assert_eq!(
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).unwrap_err(),
            SpeechPlanError::NoSpeechPath("TTS".into())
        );
    }

    #[test]
    fn v2_can_resolve_endpoint_stt_and_daemon_tts() {
        let mut offer = offer();
        offer.overrides.tts = SpeechPlacementOverride::Daemon;
        let plan = SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).unwrap();
        assert_eq!(plan.stt_site, SpeechSite::Endpoint);
        assert_eq!(plan.tts_site, SpeechSite::Daemon);
    }

    #[test]
    fn english_pack_does_not_advertise_uninstalled_languages() {
        let mut offer = offer();
        offer.language = "fr".into();
        offer.endpoint_stt[0].languages = vec!["fr".into()];
        offer.overrides.tts = SpeechPlacementOverride::Daemon;
        assert_eq!(
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).unwrap_err(),
            SpeechPlanError::NoSpeechPath("TTS".into())
        );
        offer.language = "en-GB".into();
        offer.endpoint_stt[0].languages = vec!["en".into()];
        assert!(SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).is_ok());
    }

    #[test]
    fn voice_negotiation_migrates_old_presets_and_rejects_excluded_voices() {
        let mut offer = offer();
        offer.overrides.tts = SpeechPlacementOverride::Daemon;
        let plan =
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), Some("af_bella".into()), None, true)
                .unwrap();
        assert_eq!(plan.voice_id, "alba");
        for voice in ["cosette", "jean", "unknown"] {
            assert!(matches!(
                SpeechPlan::resolve_voice_v2(&offer, &defaults(), Some(voice.into()), None, true),
                Err(SpeechPlanError::InvalidOffer(_))
            ));
        }
    }

    #[test]
    fn v2_daemon_stt_requires_audio_uplink_capability() {
        let mut offer = offer();
        offer.endpoint_stt.clear();
        assert_eq!(
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, Some(&daemon_stt()), true,)
                .unwrap_err(),
            SpeechPlanError::NoSpeechPath("STT".into())
        );
    }

    #[test]
    fn v2_loopback_daemon_stt_needs_no_remote_consent() {
        let mut offer = offer();
        offer.endpoint_stt.clear();
        offer.audio_uplink = true;
        let plan =
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, Some(&daemon_stt()), true)
                .unwrap();
        assert_eq!(plan.stt_site, SpeechSite::Daemon);
        assert_eq!(plan.uplink, SpeechUplink::Audio);
        assert_eq!(plan.eou_owner, SpeechBoundaryOwner::Daemon);
    }

    #[test]
    fn v2_remote_daemon_stt_requires_explicit_consent() {
        let mut offer = offer();
        offer.endpoint_stt.clear();
        offer.audio_uplink = true;
        offer.daemon_is_non_loopback = true;
        assert_eq!(
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, Some(&daemon_stt()), true,)
                .unwrap_err(),
            SpeechPlanError::RemoteAudioConsentRequired
        );
    }

    #[test]
    fn v2_remote_daemon_stt_accepts_explicit_consent() {
        let mut offer = offer();
        offer.endpoint_stt.clear();
        offer.audio_uplink = true;
        offer.daemon_is_non_loopback = true;
        offer.remote_audio_consent = true;
        let plan =
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, Some(&daemon_stt()), true)
                .unwrap();
        assert_eq!(plan.stt_site, SpeechSite::Daemon);
        assert_eq!(plan.uplink, SpeechUplink::Audio);
    }

    #[test]
    fn v2_remote_endpoint_stt_needs_no_audio_consent() {
        let mut offer = offer();
        offer.daemon_is_non_loopback = true;
        offer.remote_audio_consent = false;
        let plan = SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).unwrap();
        assert_eq!(plan.stt_site, SpeechSite::Endpoint);
        assert_eq!(plan.uplink, SpeechUplink::Transcript);
    }

    #[test]
    fn v2_invalid_override_fails_instead_of_downgrading() {
        let mut offer = offer();
        offer.overrides.stt = SpeechPlacementOverride::Daemon;
        assert_eq!(
            SpeechPlan::resolve_voice_v2(&offer, &defaults(), None, None, true).unwrap_err(),
            SpeechPlanError::NoSpeechPath("STT".into())
        );
    }

    #[test]
    fn body_plan_preserves_audio_and_caption_delivery() {
        let plan = SpeechPlan::body_v1(&defaults());
        assert_eq!(plan.tts_site, SpeechSite::Daemon);
        assert_eq!(plan.downlink, SpeechDownlink::Audio);
        assert_eq!(plan.tts_text_delivery, TtsTextDelivery::SentenceCaptions);
    }
}
