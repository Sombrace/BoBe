use std::sync::Arc;
use std::time::Instant;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::app_state::AppState;
use crate::speech::SpeechPlan;
use crate::voice::spoken::SpokenScript;

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
    pub(crate) last_final_utterance: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaybackReport {
    Heard,
    Unheard,
}

pub(crate) struct TurnInFlight {
    pub(crate) turn_id: String,
    pub(crate) join: JoinHandle<()>,
    pub(crate) playback_complete: Option<oneshot::Sender<PlaybackReport>>,
    pub(crate) playback_started: bool,
    pub(crate) script: Arc<SpokenScript>,
}

impl TurnInFlight {
    pub(crate) fn new(
        turn_id: String,
        join: JoinHandle<()>,
        playback_complete: Option<oneshot::Sender<PlaybackReport>>,
        script: Arc<SpokenScript>,
    ) -> Self {
        Self {
            turn_id,
            join,
            playback_complete,
            playback_started: false,
            script,
        }
    }

    fn answers_to(&self, turn_id: Option<&str>) -> bool {
        turn_id.is_none_or(|named| named == self.turn_id)
    }
}

pub(crate) struct AbortedTurn {
    pub(crate) turn: TurnInFlight,
    pub(crate) overlap: Option<PendingBargeIn>,
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

    pub(crate) fn abort_turn(&mut self) -> Option<AbortedTurn> {
        let turn = self.current_turn.take()?;
        let overlap = self
            .pending_barge_in
            .take()
            .filter(|overlap| overlap.is_live(Instant::now()));
        self.last_partial_text.clear();
        self.last_acked_played_ms = 0;
        Some(AbortedTurn { turn, overlap })
    }

    fn evidence_turn(&mut self, turn_id: Option<&str>) -> Option<&mut TurnInFlight> {
        self.current_turn
            .as_mut()
            .filter(|turn| turn.answers_to(turn_id))
    }

    pub(crate) fn accepts_overlap(&self, turn_id: Option<&str>) -> bool {
        self.current_turn
            .as_ref()
            .is_some_and(|turn| turn.answers_to(turn_id))
    }

    pub(crate) fn note_playback_started(&mut self, turn_id: &str) -> bool {
        let Some(turn) = self.evidence_turn(Some(turn_id)) else {
            return false;
        };
        turn.playback_started = true;
        true
    }

    pub(crate) fn note_playback_ack(&mut self, turn_id: Option<&str>, played_ms: u64) -> bool {
        let Some(turn) = self.evidence_turn(turn_id) else {
            return false;
        };
        turn.playback_started |= played_ms > 0;
        self.last_acked_played_ms = self.last_acked_played_ms.max(played_ms);
        true
    }

    pub(crate) fn report_playback_complete(&mut self, turn_id: &str) -> Option<PlaybackReport> {
        let turn = self.evidence_turn(Some(turn_id))?;
        let report = if turn.playback_started {
            PlaybackReport::Heard
        } else {
            PlaybackReport::Unheard
        };
        let sender = turn.playback_complete.take()?;
        if sender.send(report).is_err() {
            debug!(turn_id, "voice.playback_report_receiver_dropped");
        }
        Some(report)
    }

    pub(crate) fn note_final_utterance(&mut self, utterance_id: &str) {
        self.last_final_utterance = Some(utterance_id.to_owned());
    }

    pub(crate) fn accepts_partial(&self, utterance_id: &str) -> bool {
        self.last_final_utterance.as_deref() != Some(utterance_id)
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
            last_final_utterance: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        PendingBargeIn, PlaybackReport, SessionVoiceConfig, TurnInFlight, VoiceDefaults,
        VoiceSession,
    };
    use tokio::sync::oneshot;

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
        TurnInFlight::new(
            turn_id.into(),
            tokio::spawn(async {}),
            None,
            std::sync::Arc::default(),
        )
    }

    fn reporting_turn(turn_id: &str) -> (TurnInFlight, oneshot::Receiver<PlaybackReport>) {
        let (sender, receiver) = oneshot::channel();
        let turn = TurnInFlight::new(
            turn_id.into(),
            tokio::spawn(async {}),
            Some(sender),
            std::sync::Arc::default(),
        );
        (turn, receiver)
    }

    #[tokio::test]
    async fn a_reply_that_never_started_playing_is_not_a_presentation_receipt() {
        let mut session = session();
        let (turn, mut receiver) = reporting_turn("voice_one");
        session.begin_turn(turn);
        assert!(session.note_playback_ack(Some("voice_one"), 0));
        assert_eq!(
            session.report_playback_complete("voice_one"),
            Some(PlaybackReport::Unheard)
        );
        assert_eq!(receiver.try_recv().unwrap(), PlaybackReport::Unheard);
    }

    #[tokio::test]
    async fn client_speech_that_started_playing_is_a_presentation_receipt() {
        let mut session = session();
        let (turn, mut receiver) = reporting_turn("voice_one");
        session.begin_turn(turn);
        assert!(!session.note_playback_started("voice_zero"));
        assert!(session.note_playback_started("voice_one"));
        assert_eq!(
            session.report_playback_complete("voice_one"),
            Some(PlaybackReport::Heard)
        );
        assert_eq!(receiver.try_recv().unwrap(), PlaybackReport::Heard);
    }

    #[tokio::test]
    async fn daemon_audio_counts_as_started_once_the_client_reports_playing_it() {
        let mut session = session();
        let (turn, mut receiver) = reporting_turn("voice_one");
        session.begin_turn(turn);
        assert!(session.note_playback_ack(None, 480));
        assert_eq!(session.last_acked_played_ms, 480);
        assert_eq!(
            session.report_playback_complete("voice_one"),
            Some(PlaybackReport::Heard)
        );
        assert_eq!(receiver.try_recv().unwrap(), PlaybackReport::Heard);
    }

    #[tokio::test]
    async fn evidence_named_for_an_earlier_reply_cannot_touch_the_current_one() {
        let mut session = session();
        let (turn, mut receiver) = reporting_turn("voice_two");
        session.begin_turn(turn);
        assert!(!session.accepts_overlap(Some("voice_one")));
        assert!(session.accepts_overlap(Some("voice_two")));
        assert!(session.accepts_overlap(None));
        assert!(!session.note_playback_ack(Some("voice_one"), 9_000));
        assert_eq!(session.last_acked_played_ms, 0);
        assert_eq!(session.report_playback_complete("voice_one"), None);
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            session.report_playback_complete("voice_two"),
            Some(PlaybackReport::Unheard)
        );
    }

    #[tokio::test]
    async fn evidence_without_a_reply_in_flight_is_ignored() {
        let mut session = session();
        assert!(!session.accepts_overlap(None));
        assert!(!session.note_playback_ack(None, 900));
        assert!(!session.note_playback_started("voice_one"));
        assert_eq!(session.report_playback_complete("voice_one"), None);
    }

    #[test]
    fn a_partial_that_arrives_after_its_final_is_stale() {
        let mut session = session();
        assert!(session.accepts_partial("voice_one"));
        session.note_final_utterance("voice_one");
        assert!(!session.accepts_partial("voice_one"));
        assert!(session.accepts_partial("voice_two"));
    }

    #[tokio::test]
    async fn an_aborted_session_keeps_listening_for_the_next_utterance() {
        let mut session = session();
        session.begin_turn(turn("voice_one"));
        session.pending_barge_in = Some(PendingBargeIn {
            heard_at: Instant::now(),
            played_ms: 300,
            playhead_chunk: 2,
        });
        session.last_partial_text = "wait".into();
        session.last_acked_played_ms = 300;
        let aborted = session.abort_turn().expect("a turn was in flight");
        assert_eq!(aborted.turn.turn_id, "voice_one");
        assert!(aborted.overlap.is_some());
        assert!(session.current_turn.is_none());
        assert!(session.pending_barge_in.is_none());
        assert!(session.last_partial_text.is_empty());
        assert_eq!(session.last_acked_played_ms, 0);
        assert!(!session.muted);
        assert!(session.abort_turn().is_none());
        session.begin_turn(turn("voice_two"));
        assert_eq!(
            session
                .current_turn
                .as_ref()
                .map(|turn| turn.turn_id.as_str()),
            Some("voice_two")
        );
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
