use std::sync::Arc;

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::config::Config;
use crate::db::SqliteCooldownRepo;
use crate::runtime::conversation_service::ConversationService;
use crate::runtime::proactive_generator::ProactiveGenerator;
use crate::runtime::state::Decision;
use crate::runtime::triggers::checkin_scheduler::CheckinScheduler;
use crate::runtime::turn_admission::TurnSource;

pub(crate) struct CheckinTrigger {
    scheduler: CheckinScheduler,
    generator: Arc<ProactiveGenerator>,
    conversation: Arc<ConversationService>,
    cooldown_repo: Arc<SqliteCooldownRepo>,
    config: Arc<ArcSwap<Config>>,
    personal_data_lifecycle: Arc<tokio::sync::RwLock<()>>,
}

impl CheckinTrigger {
    pub(crate) fn new(
        scheduler: CheckinScheduler,
        generator: Arc<ProactiveGenerator>,
        conversation: Arc<ConversationService>,
        cooldown_repo: Arc<SqliteCooldownRepo>,
        config: Arc<ArcSwap<Config>>,
        personal_data_lifecycle: Arc<tokio::sync::RwLock<()>>,
    ) -> Self {
        Self {
            scheduler,
            generator,
            conversation,
            cooldown_repo,
            config,
            personal_data_lifecycle,
        }
    }

    pub(crate) fn reconfigure(&mut self, config: &Config) {
        self.scheduler.reconfigure(
            config.checkin_times_vec(),
            config.checkin.interval_minutes,
            config.checkin.jitter_minutes,
            config.checkin.enabled,
        );
    }

    pub(crate) async fn fire(&mut self) -> Decision {
        let _personal_data = self.personal_data_lifecycle.read().await;
        if !self.scheduler.should_checkin() {
            return Decision::Idle;
        }

        let cfg = self.config.load();

        if let Ok(Some(existing)) = self.conversation.get_pending_or_active().await {
            let last_user_at = self
                .conversation
                .last_user_turn_at(existing.id)
                .await
                .ok()
                .flatten();
            if !existing.is_stale_since(cfg.conversation.auto_close_minutes as i64, last_user_at) {
                debug!(reason = "active_conversation", "checkin_trigger.skipped");
                self.scheduler.mark_checkin_done();
                return Decision::Idle;
            }
        }

        if let Some(cooldown) = self.cooldown_repo.check_cooldown(
            cfg.decision.cooldown_minutes,
            cfg.decision.extended_cooldown_minutes,
        ) {
            debug!(
                remaining_s = cooldown.remaining.num_seconds(),
                cooldown_type = %cooldown.cooldown_type,
                "checkin_trigger.cooldown_skipped"
            );
            self.scheduler.mark_checkin_done();
            return Decision::Idle;
        }

        info!("checkin_trigger.started");
        let decision = self
            .generator
            .generate_proactive_response(TurnSource::Checkin, Some("Scheduled check-in".into()))
            .await;
        self.scheduler.mark_checkin_done();
        info!("checkin_trigger.complete");

        decision
    }

    pub(crate) fn get_next_checkin_time(&mut self) -> Option<DateTime<Utc>> {
        self.scheduler.get_next_checkin_time()
    }
}
