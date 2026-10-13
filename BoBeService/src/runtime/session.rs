use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use arc_swap::ArcSwap;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::SqliteCooldownRepo;
use crate::runtime::conversation_service::ConversationService;
use crate::runtime::message_handler::MessageHandler;
use crate::runtime::state::Decision;
use crate::runtime::triggers::{CaptureTrigger, CheckinTrigger, GoalTrigger};
use crate::runtime::turn_admission::{TurnAdmission, TurnSource};
use crate::util::sse::body_requests::BodyAction;
use crate::util::sse::connection_manager::SseConnectionManager;
use crate::util::sse::event_queue::EventQueue;
use crate::util::sse::types::IndicatorType;

pub(crate) struct RuntimeSession {
    checkin_trigger: Mutex<CheckinTrigger>,
    goal_trigger: Arc<GoalTrigger>,
    capture_trigger: Mutex<CaptureTrigger>,
    message_handler: Arc<MessageHandler>,
    conversation: Arc<ConversationService>,
    cooldown_repo: Arc<SqliteCooldownRepo>,
    event_queue: Arc<EventQueue>,
    config: Arc<ArcSwap<Config>>,
    running: std::sync::atomic::AtomicBool,
    connections: Arc<SseConnectionManager>,
    capture_wake: tokio::sync::Notify,
    turn_admission: Arc<TurnAdmission>,
    pub(crate) runs: Arc<crate::runtime::run_registry::RunRegistry>,
}

pub(crate) type UserMessageGuard = crate::runtime::turn_admission::TurnPermit;

#[derive(Debug, serde::Serialize)]
pub(crate) struct RuntimeStatus {
    pub(crate) indicator: IndicatorType,
    pub(crate) capturing: bool,
    pub(crate) accepting_user_messages: bool,
}

impl RuntimeSession {
    pub(crate) async fn bind_owner_answer(
        &self,
        channel: crate::models::ids::ConversationChannelId,
        text: &str,
        received_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<uuid::Uuid>, crate::error::AppError> {
        self.message_handler
            .bind_owner_answer(channel, text, received_at)
            .await
    }

    pub(crate) async fn answer_bound_input(
        &self,
        input: Option<uuid::Uuid>,
        response: uuid::Uuid,
        text: &str,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<(), crate::error::AppError> {
        self.message_handler
            .answer_bound_input(input, response, text, channel)
            .await
    }

    pub(crate) async fn acknowledge_presentation(
        &self,
        turn: crate::models::ids::ConversationTurnId,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<(), crate::error::AppError> {
        self.conversation
            .acknowledge_presentation(turn, channel)
            .await
    }

    pub(crate) async fn unpresented_updates(
        &self,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<Vec<crate::models::conversation::ConversationTurn>, crate::error::AppError> {
        self.conversation.unpresented_updates(channel).await
    }

    pub(crate) fn new(
        checkin_trigger: CheckinTrigger,
        goal_trigger: Arc<GoalTrigger>,
        capture_trigger: CaptureTrigger,
        message_handler: Arc<MessageHandler>,
        conversation: Arc<ConversationService>,
        cooldown_repo: Arc<SqliteCooldownRepo>,
        event_queue: Arc<EventQueue>,
        config: Arc<ArcSwap<Config>>,
        connections: Arc<SseConnectionManager>,
        turn_admission: Arc<TurnAdmission>,
        runs: Arc<crate::runtime::run_registry::RunRegistry>,
    ) -> Self {
        Self {
            checkin_trigger: Mutex::new(checkin_trigger),
            goal_trigger,
            capture_trigger: Mutex::new(capture_trigger),
            message_handler,
            conversation,
            cooldown_repo,
            event_queue,
            config,
            running: std::sync::atomic::AtomicBool::new(false),
            connections,
            capture_wake: tokio::sync::Notify::new(),
            turn_admission,
            runs,
        }
    }

    pub(crate) fn fail_message(
        &self,
        turn_id: crate::models::ids::ConversationTurnId,
        channel: crate::models::ids::ConversationChannelId,
        reason: &str,
    ) {
        self.event_queue.push_in_channel(
            channel,
            crate::util::sse::factories::error_event(
                &crate::models::ids::message_id_for_turn(turn_id),
                reason,
                if reason == "turn_cancelled" {
                    "This turn was stopped. Completed changes remain saved."
                } else {
                    "BoBe could not finish this turn. Check saved results before repeating changes."
                },
                true,
            ),
        );
    }

    pub(crate) fn wake_capture(&self) {
        self.capture_wake.notify_one();
    }

    fn capture_active(&self) -> bool {
        self.config.load().capture.enabled && self.connections.serves(BodyAction::ScreenRead)
    }

    pub(crate) async fn reconfigure_checkins(&self) {
        let config = self.config.load_full();
        self.checkin_trigger.lock().await.reconfigure(&config);
    }

    pub(crate) async fn start(&self) {
        self.running
            .store(true, std::sync::atomic::Ordering::Release);

        if let Err(e) = self.cooldown_repo.load_or_create().await {
            warn!(error = %e, "runtime_session.cooldown_load_failed");
        }

        info!("runtime_session.started");
    }

    pub(crate) async fn stop(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Release);
        info!("runtime_session.stopped");
    }

    pub(crate) async fn run(&self) {
        self.start().await;

        {
            let mut checkin = self.checkin_trigger.lock().await;
            let next = checkin.get_next_checkin_time();
            info!(
                next_checkin = ?next.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()),
                "runtime_session.scheduler_status"
            );
        }

        tokio::join!(
            self.heartbeat_loop(),
            self.checkin_loop(),
            self.stale_conversation_loop(),
            self.goal_loop(),
            self.capture_loop(),
        );

        self.stop().await;
    }

    async fn heartbeat_loop(&self) {
        let mut heartbeat = 0_u64;
        while self.running.load(Ordering::Acquire) {
            tokio::time::sleep(std::time::Duration::from_mins(5)).await;
            if !self.running.load(Ordering::Acquire) {
                return;
            }
            heartbeat = heartbeat.saturating_add(1);
            self.log_heartbeat(heartbeat).await;
        }
    }

    async fn checkin_loop(&self) {
        while self.running.load(Ordering::Acquire) {
            Box::pin(run_trigger(
                "checkin",
                std::time::Duration::from_mins(1),
                async {
                    let mut checkin = self.checkin_trigger.lock().await;
                    checkin.fire().await
                },
            ))
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    }

    async fn stale_conversation_loop(&self) {
        while self.running.load(Ordering::Acquire) {
            if let Err(e) = self.close_stale_conversation_if_needed().await {
                warn!(error = %e, "runtime_session.stale_check_failed");
            }
            tokio::time::sleep(std::time::Duration::from_mins(1)).await;
        }
    }

    async fn goal_loop(&self) {
        while self.running.load(Ordering::Acquire) {
            if !self.config.load().goals.enabled {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
            let seconds = {
                let configured = self.config.load().goals.check_interval_seconds;
                if configured.is_finite() {
                    configured.max(1.0)
                } else {
                    30.0
                }
            };
            Box::pin(run_trigger(
                "goal",
                std::time::Duration::from_mins(5),
                self.goal_trigger.fire(),
            ))
            .await;
            tokio::time::sleep(std::time::Duration::from_secs_f64(seconds.min(30.0))).await;
        }
    }

    async fn capture_loop(&self) {
        let mut last_fire = Instant::now();
        while self.running.load(Ordering::Acquire) {
            let enabled = self.capture_active();
            let seconds = self.config.load().capture.interval_seconds.max(1);
            if enabled && last_fire.elapsed().as_secs() >= seconds {
                let timed_out = Box::pin(run_trigger(
                    "capture",
                    std::time::Duration::from_mins(5),
                    async {
                        let mut capture = self.capture_trigger.lock().await;
                        capture.fire().await
                    },
                ))
                .await;
                if timed_out {
                    self.push_error_event("capture_trigger", "Capture trigger timed out");
                }
                last_fire = Instant::now();
            }
            let sleep_seconds = if enabled { seconds.min(5) } else { 30 };
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(sleep_seconds)) => {}
                () = self.capture_wake.notified() => {}
            }
        }
    }

    async fn close_stale_conversation_if_needed(&self) -> Result<(), crate::error::AppError> {
        let cfg = self.config.load();
        for existing in self.conversation.list_open().await? {
            if let Some(turn_count) = self
                .conversation
                .close_if_stale(existing.id, cfg.conversation.auto_close_minutes as i64)
                .await?
            {
                info!(conversation_id=%existing.id, turn_count, "runtime_session.auto_closing_stale_conversation");
                self.event_queue.push_in_channel(
                    existing.channel_id,
                    crate::util::sse::factories::conversation_closed_event(
                        &existing.id.to_string(),
                        "inactivity_timeout",
                        u32::try_from(turn_count).unwrap_or(u32::MAX),
                    ),
                );
            }
        }
        Ok(())
    }

    async fn log_heartbeat(&self, loop_counter: u64) {
        let mut checkin = self.checkin_trigger.lock().await;
        let next_checkin = checkin.get_next_checkin_time();
        info!(
            loop_count = loop_counter,
            capture = self.capture_active(),
            next_checkin = ?next_checkin.map(|t| t.format("%H:%M:%S").to_string()),
            "runtime_session.heartbeat"
        );
    }

    pub(crate) async fn handle_user_message(
        &self,
        content: &str,
        message_id: &str,
        assistant_turn_id: crate::models::ids::ConversationTurnId,
        channel: crate::models::ids::ConversationChannelId,
    ) -> bool {
        self.message_handler
            .handle_message(content, message_id, assistant_turn_id, channel)
            .await
    }

    pub(crate) async fn settle_interrupted_turn(
        &self,
        turn_id: crate::models::ids::ConversationTurnId,
    ) {
        if let Err(error) = self.message_handler.settle_run(turn_id.into()).await {
            tracing::error!(%error,%turn_id,"runtime_session.run_settlement_failed");
        }
        if let Err(error) = self.conversation.settle_interrupted_turn(turn_id).await {
            tracing::error!(%error, %turn_id, "runtime_session.interrupted_turn_persist_failed");
        }
    }

    pub(crate) fn note_interrupted_reply(
        &self,
        channel: crate::models::ids::ConversationChannelId,
        reply: crate::provider::InterruptedReply,
    ) {
        self.message_handler.note_interrupted_reply(channel, reply);
    }

    pub(crate) fn try_begin_background_result(
        &self,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Option<UserMessageGuard> {
        self.turn_admission.try_admit_channel(
            crate::runtime::turn_admission::TurnSource::Background,
            channel,
        )
    }

    pub(crate) async fn handle_background_result(
        &self,
        job: &crate::db::agent_job_repo::AgentJob,
    ) -> bool {
        self.message_handler.handle_background_result(job).await
    }

    pub(crate) async fn evaluate_background_update(
        &self,
        job: &crate::db::agent_job_repo::AgentJob,
    ) -> Result<crate::runtime::message_handler::BackgroundOffer, crate::error::AppError> {
        self.message_handler.evaluate_background_update(job).await
    }

    pub(crate) async fn update_channel(
        &self,
    ) -> Result<crate::models::ids::ConversationChannelId, crate::error::AppError> {
        Ok(self
            .conversation
            .get_latest_readable_conversation(1, None)
            .await?
            .map_or(
                crate::models::ids::ConversationChannelId::OWNER,
                |(conversation, _)| conversation.channel_id,
            ))
    }

    pub(crate) async fn can_offer_update(&self) -> bool {
        self.turn_admission.is_idle()
            && crate::runtime::proactive_generator::activation_is_timely(
                &self.conversation,
                &self.cooldown_repo,
                &self.config.load_full(),
            )
            .await
    }

    pub(crate) async fn handle_user_message_with_observer<F, Fut>(
        &self,
        content: &str,
        message_id: &str,
        assistant_turn_id: crate::models::ids::ConversationTurnId,
        delivery: crate::runtime::response_streamer::StreamDelivery,
        channel: crate::models::ids::ConversationChannelId,
        on_text_delta: F,
    ) -> bool
    where
        F: FnMut(String) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        self.message_handler
            .handle_message_with_observer(
                content,
                message_id,
                assistant_turn_id,
                true,
                delivery,
                channel,
                on_text_delta,
            )
            .await
    }

    pub(crate) async fn handle_endpoint_text_with_observer<F, Fut>(
        &self,
        content: &str,
        message_id: &str,
        assistant_turn_id: crate::models::ids::ConversationTurnId,
        channel: crate::models::ids::ConversationChannelId,
        on_text_delta: F,
    ) -> bool
    where
        F: FnMut(String) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        self.message_handler
            .handle_message_with_observer(
                content,
                message_id,
                assistant_turn_id,
                false,
                crate::runtime::response_streamer::StreamDelivery::EndpointOnly,
                channel,
                on_text_delta,
            )
            .await
    }

    pub(crate) fn try_begin_user_message(
        &self,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<UserMessageGuard, &'static str> {
        self.turn_admission
            .try_admit_channel(TurnSource::User, channel)
            .ok_or("BoBe is finishing the previous turn in this channel")
    }

    pub(crate) async fn begin_user_message(
        &self,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<UserMessageGuard, crate::error::AppError> {
        self.turn_admission.admit_user(channel).await
    }

    pub(crate) fn queue_user_message(
        &self,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<crate::runtime::turn_admission::QueuedTurn, crate::error::AppError> {
        self.turn_admission.queue_user(channel)
    }

    pub(crate) fn try_begin_privacy_purge(&self) -> Result<UserMessageGuard, &'static str> {
        self.turn_admission
            .try_admit(TurnSource::Maintenance)
            .ok_or("Cannot purge data while a turn is active")
    }

    pub(crate) fn try_begin_personal_data_access(&self) -> Result<UserMessageGuard, &'static str> {
        self.turn_admission
            .try_admit(TurnSource::Maintenance)
            .ok_or("Personal data is busy with another operation")
    }

    pub(crate) fn get_status(
        &self,
        channel: crate::models::ids::ConversationChannelId,
    ) -> RuntimeStatus {
        let indicator = self.event_queue.channel_indicator(channel);
        RuntimeStatus {
            indicator,
            capturing: self.capture_active(),
            accepting_user_messages: self.turn_admission.accepts_messages(),
        }
    }

    pub(crate) async fn current_conversation(
        &self,
        limit: i64,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<
        Option<(
            crate::models::conversation::Conversation,
            Vec<crate::models::conversation::ConversationTurn>,
        )>,
        crate::error::AppError,
    > {
        self.conversation
            .get_latest_readable_conversation(limit, Some(channel))
            .await
    }

    fn push_error_event(&self, trigger: &str, message: &str) {
        error!(trigger, message, "runtime_session.trigger_error");
        self.event_queue
            .push(crate::util::sse::factories::trigger_error_event(
                trigger, message, true,
            ));
    }
}

async fn run_trigger(
    name: &'static str,
    timeout: std::time::Duration,
    fut: impl std::future::Future<Output = Decision>,
) -> bool {
    match tokio::time::timeout(timeout, fut).await {
        Ok(Decision::Engage) => {
            info!(trigger = name, "runtime_session.reach_out");
            false
        }
        Ok(_) => false,
        Err(_) => {
            warn!(trigger = name, "runtime_session.trigger_timeout");
            true
        }
    }
}
