use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use sha2::{Digest, Sha256};
use tracing::{debug, error, info, warn};

const VISION_FAILURE_BREAKER_THRESHOLD: u32 = 3;

const VISION_FAILURE_COOLDOWN: Duration = Duration::from_mins(3);

const SCREEN_READ_DEADLINE: Duration = Duration::from_secs(20);

use crate::config::Config;
use crate::db::SqliteCooldownRepo;
use crate::runtime::capture_learner::CaptureLearner;
use crate::runtime::proactive_generator::ProactiveGenerator;
use crate::runtime::state::Decision;
use crate::util::sse::body_requests::{BodyAction, BodyRequestError, ScreenReading};
use crate::util::sse::connection_manager::SseConnectionManager;
use crate::util::sse::event_queue::EventQueue;
use crate::util::sse::factories::trigger_error_event;

use crate::runtime::turn_admission::TurnSource;

pub(crate) struct CaptureTrigger {
    connections: Arc<SseConnectionManager>,
    capture_learner: Arc<CaptureLearner>,
    generator: Arc<ProactiveGenerator>,
    cooldown_repo: Arc<SqliteCooldownRepo>,
    event_queue: Arc<EventQueue>,
    config: Arc<ArcSwap<Config>>,
    personal_data_lifecycle: Arc<tokio::sync::RwLock<()>>,
    context_count: usize,
    previous_capture: Option<[u8; 32]>,
    vision_failure_count: u32,
    vision_breaker_tripped_at: Option<Instant>,
    vision_pause_announced: bool,
}

impl CaptureTrigger {
    pub(crate) fn new(
        connections: Arc<SseConnectionManager>,
        capture_learner: Arc<CaptureLearner>,
        generator: Arc<ProactiveGenerator>,
        cooldown_repo: Arc<SqliteCooldownRepo>,
        event_queue: Arc<EventQueue>,
        config: Arc<ArcSwap<Config>>,
        personal_data_lifecycle: Arc<tokio::sync::RwLock<()>>,
    ) -> Self {
        Self {
            connections,
            capture_learner,
            generator,
            cooldown_repo,
            event_queue,
            config,
            personal_data_lifecycle,
            context_count: 0,
            previous_capture: None,
            vision_failure_count: 0,
            vision_breaker_tripped_at: None,
            vision_pause_announced: false,
        }
    }

    fn announce_vision_recovered(&mut self) {
        self.vision_failure_count = 0;
        self.vision_breaker_tripped_at = None;
        if self.vision_pause_announced {
            self.vision_pause_announced = false;
            self.event_queue.push(trigger_error_event(
                "vision",
                "Screen awareness restored.",
                true,
            ));
        }
    }

    fn vision_breaker_open(&mut self) -> bool {
        let Some(tripped_at) = self.vision_breaker_tripped_at else {
            return false;
        };
        if tripped_at.elapsed() >= VISION_FAILURE_COOLDOWN {
            self.vision_breaker_tripped_at = None;
            info!("capture_trigger.vision_breaker_reopened");
            false
        } else {
            true
        }
    }

    pub(crate) async fn fire(&mut self) -> Decision {
        let _personal_data = Arc::clone(&self.personal_data_lifecycle).read_owned().await;
        let Some(description) = self.run_capture_cycle().await else {
            return Decision::Idle;
        };

        let cfg = self.config.load();

        match self
            .cooldown_repo
            .check_cooldown(
                cfg.decision.cooldown_minutes,
                cfg.decision.extended_cooldown_minutes,
            )
            .await
        {
            Ok(None) => {}
            Ok(Some(cooldown)) => {
                debug!(
                    remaining_s = cooldown.remaining.num_seconds(),
                    cooldown_type = %cooldown.cooldown_type,
                    "capture_trigger.cooldown_active"
                );
                return Decision::Idle;
            }
            Err(error) => {
                warn!(%error, "capture_trigger.cooldown_check_failed");
                return Decision::Idle;
            }
        }

        self.generator
            .generate_proactive_response(TurnSource::Capture, Some(description))
            .await
    }

    async fn run_capture_cycle(&mut self) -> Option<String> {
        if self.vision_breaker_open() {
            debug!("capture_trigger.vision_breaker_open_skipping_cycle");
            return None;
        }

        let cycle_num = self.context_count + 1;
        info!(cycle = cycle_num, "capture_trigger.cycle_start");

        let reading = match self
            .connections
            .request(BodyAction::ScreenRead, SCREEN_READ_DEADLINE)
            .await
            .and_then(ScreenReading::from_result)
        {
            Ok(reading) => reading,
            Err(BodyRequestError::NoBody) => {
                debug!(cycle = cycle_num, "capture_trigger.no_screen_body");
                return None;
            }
            Err(BodyRequestError::Declined(reason)) => {
                info!(cycle = cycle_num, %reason, "capture_trigger.screen_declined");
                return None;
            }
            Err(e) => {
                error!(error = %e, cycle = cycle_num, "capture_trigger.screen_read_failed");
                return None;
            }
        };
        let mut digest = Sha256::new();
        digest.update(reading.text.as_bytes());
        digest.update([0]);
        digest.update(
            reading
                .application
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        digest.update([0]);
        digest.update(
            reading
                .window_title
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        let fingerprint: [u8; 32] = digest.finalize().into();
        if self.previous_capture == Some(fingerprint) {
            debug!("capture_trigger.unchanged_screen");
            return None;
        }
        let result = self
            .capture_learner
            .learn(
                &reading.text,
                reading.application.as_deref(),
                reading.window_title.as_deref(),
            )
            .await;

        match result {
            Ok(description) => {
                self.previous_capture = Some(fingerprint);
                self.announce_vision_recovered();
                if description.is_empty() {
                    debug!(cycle = cycle_num, "capture_trigger.empty_description");
                    None
                } else {
                    self.context_count += 1;
                    debug!(cycle = cycle_num, "capture_trigger.cycle_complete");
                    Some(description)
                }
            }
            Err(e) => {
                self.vision_failure_count += 1;
                if self.vision_failure_count >= VISION_FAILURE_BREAKER_THRESHOLD {
                    self.vision_breaker_tripped_at = Some(Instant::now());
                    if !self.vision_pause_announced {
                        self.vision_pause_announced = true;
                        self.event_queue.push(trigger_error_event(
                            "vision",
                            "Screen awareness is paused because the screen could not be read.",
                            true,
                        ));
                    }
                    warn!(
                        cycle = cycle_num,
                        consecutive_failures = self.vision_failure_count,
                        cooldown_secs = VISION_FAILURE_COOLDOWN.as_secs(),
                        error = %e,
                        "capture_trigger.vision_breaker_tripped"
                    );
                } else {
                    warn!(
                        cycle = cycle_num,
                        consecutive_failures = self.vision_failure_count,
                        error = %e,
                        "capture_trigger.cycle_failed"
                    );
                }
                None
            }
        }
    }
}
