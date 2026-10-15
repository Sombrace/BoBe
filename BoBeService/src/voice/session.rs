use std::time::Instant;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::app_state::AppState;
use crate::speech::SpeechPlan;

#[derive(Clone)]
pub(crate) struct SessionVoiceConfig {
    pub(crate) speed: f32,
    pub(crate) speech_plan: SpeechPlan,
}

impl SessionVoiceConfig {
    pub(crate) fn new(
        speed: Option<f32>,
        speech_plan: SpeechPlan,
        defaults: &VoiceDefaults,
    ) -> Self {
        Self {
            speed: speed.unwrap_or(defaults.speed).clamp(0.5, 2.0),
            speech_plan,
        }
    }
}

#[derive(Clone)]
pub(crate) struct VoiceDefaults {
    pub(crate) enabled: bool,
    pub(crate) persona: String,
    pub(crate) speed: f32,
}

impl VoiceDefaults {
    pub(crate) fn from_state(state: &AppState) -> Self {
        let cfg = state.config();
        Self::from_config(&cfg)
    }

    pub(crate) fn from_config(config: &crate::config::Config) -> Self {
        Self {
            enabled: config.voice.enabled,
            persona: config.voice.persona.clone(),
            speed: config.voice.speed,
        }
    }
}

pub(crate) struct VoiceSession {
    pub(crate) session_id: String,
    pub(crate) current_turn: Option<TurnInFlight>,
    pub(crate) muted: bool,
    pub(crate) last_acked_played_ms: u64,
    pub(crate) voice_cfg: SessionVoiceConfig,
    pub(crate) last_partial_text: String,
    pub(crate) capture: Option<VoiceCaptureState>,
    pub(crate) pending_barge_in: Option<PendingBargeIn>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingBargeIn {
    pub(crate) heard_at: Instant,
    pub(crate) played_ms: u64,
    pub(crate) playhead_chunk: u64,
}

impl PendingBargeIn {
    const WINDOW: std::time::Duration = std::time::Duration::from_millis(2_000);

    pub(crate) fn is_live(&self, now: Instant) -> bool {
        now.duration_since(self.heard_at) <= Self::WINDOW
    }
}

pub(crate) struct VoiceCaptureState {
    pub(crate) stream_id: u32,
    pub(crate) next_sequence: u32,
    pub(crate) credit_frames: u32,
    pub(crate) turn_id: String,
    pub(crate) stt: Box<dyn crate::speech::SttStream>,
}

pub(crate) struct TurnInFlight {
    pub(crate) turn_id: String,
    pub(crate) join: JoinHandle<()>,
    pub(crate) playback_complete: Option<oneshot::Sender<()>>,
    pub(crate) script: std::sync::Arc<crate::voice::spoken::SpokenScript>,
}

impl VoiceSession {
    pub(crate) fn begin_turn(&mut self, turn: TurnInFlight) {
        self.last_partial_text.clear();
        self.last_acked_played_ms = 0;
        self.pending_barge_in = None;
        self.current_turn = Some(turn);
    }

    pub(crate) fn finish_turn(&mut self) -> Option<TurnInFlight> {
        self.pending_barge_in = None;
        self.current_turn.take()
    }

    pub(crate) fn new(session_id: String, voice_cfg: SessionVoiceConfig) -> Self {
        Self {
            session_id,
            current_turn: None,
            muted: false,
            last_acked_played_ms: 0,
            voice_cfg,
            last_partial_text: String::new(),
            capture: None,
            pending_barge_in: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{PendingBargeIn, SessionVoiceConfig, TurnInFlight, VoiceDefaults, VoiceSession};

    fn session() -> VoiceSession {
        let defaults = VoiceDefaults {
            enabled: true,
            persona: "alba".into(),
            speed: 1.0,
        };
        let plan = crate::speech::SpeechPlan::body_v1(&defaults);
        VoiceSession::new(
            "session".into(),
            SessionVoiceConfig::new(None, plan, &defaults),
        )
    }

    fn turn(turn_id: &str) -> TurnInFlight {
        TurnInFlight {
            turn_id: turn_id.into(),
            join: tokio::spawn(async {}),
            playback_complete: None,
            script: std::sync::Arc::default(),
        }
    }

    #[tokio::test]
    async fn an_overlap_never_outlives_the_turn_it_was_heard_in() {
        let overlap = PendingBargeIn {
            heard_at: Instant::now(),
            played_ms: 900,
            playhead_chunk: 4,
        };
        let mut session = session();
        session.begin_turn(turn("voice_one"));
        session.pending_barge_in = Some(overlap);
        assert!(session.finish_turn().is_some());
        assert!(session.pending_barge_in.is_none());

        session.pending_barge_in = Some(overlap);
        session.last_partial_text = "yeah".into();
        session.begin_turn(turn("voice_two"));
        assert!(session.pending_barge_in.is_none());
        assert!(session.last_partial_text.is_empty());
    }

    #[test]
    fn unresolved_overlap_expires() {
        let heard_at = Instant::now();
        let pending = PendingBargeIn {
            heard_at,
            played_ms: 1_200,
            playhead_chunk: 0,
        };
        assert!(pending.is_live(heard_at + Duration::from_millis(1_500)));
        assert!(!pending.is_live(heard_at + Duration::from_millis(2_500)));
    }
}
