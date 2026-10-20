use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::PiRuntime;
use crate::db::agent_job_repo::{AgentJobRepo, JobStatus};
use crate::error::AppError;
use crate::runtime::run_registry::{RunKind, RunRegistry};
use crate::runtime::session::RuntimeSession;

const MAX_ACTIVE: usize = 2;

pub(crate) struct BackgroundWork {
    pub(crate) jobs: Arc<AgentJobRepo>,
    runtime: Arc<PiRuntime>,
    session: Arc<RuntimeSession>,
    personal_data: Arc<RwLock<()>>,
    initialization:
        tokio::sync::watch::Sender<crate::db::runtime_initialization_repo::InitializationState>,
    runs: Arc<RunRegistry>,
    connections: Arc<crate::util::sse::connection_manager::SseConnectionManager>,
    goals: Arc<crate::services::goals::goals_service::GoalsService>,
    config: Arc<arc_swap::ArcSwap<crate::config::Config>>,
    enabled: bool,
    schedule_route: crate::provider::WorkloadRoute,
}

impl BackgroundWork {
    async fn goal_update_allowed(
        &self,
        goal: Option<crate::models::ids::GoalId>,
    ) -> Result<bool, AppError> {
        let Some(id) = goal else {
            return Ok(true);
        };
        Ok(self.config.load().goals.enabled
            && self.goals.get(id).await?.is_some_and(|goal| {
                matches!(
                    goal.status,
                    crate::models::types::GoalStatus::Active
                        | crate::models::types::GoalStatus::Completed
                )
            }))
    }

    pub(crate) fn new(
        jobs: Arc<AgentJobRepo>,
        runtime: Arc<PiRuntime>,
        session: Arc<RuntimeSession>,
        personal_data: Arc<RwLock<()>>,
        initialization: tokio::sync::watch::Sender<
            crate::db::runtime_initialization_repo::InitializationState,
        >,
        runs: Arc<RunRegistry>,
        connections: Arc<crate::util::sse::connection_manager::SseConnectionManager>,
        goals: Arc<crate::services::goals::goals_service::GoalsService>,
        config: Arc<arc_swap::ArcSwap<crate::config::Config>>,
        enabled: bool,
    ) -> Arc<Self> {
        let schedule_route = config.load().routes.goals.clone();
        Arc::new(Self {
            jobs,
            runtime,
            session,
            personal_data,
            initialization,
            runs,
            connections,
            goals,
            config,
            enabled,
            schedule_route,
        })
    }

    pub(crate) async fn run(self: Arc<Self>, shutdown: CancellationToken) {
        tokio::join!(
            self.run_jobs(shutdown.clone()),
            self.deliver_results(shutdown)
        );
    }

    async fn run_jobs(&self, shutdown: CancellationToken) {
        let mut active = tokio::task::JoinSet::new();
        let mut cancellations = std::collections::HashMap::<Uuid, CancellationToken>::new();
        while !shutdown.is_cancelled() {
            if self.enabled
                && *self.initialization.borrow()
                    == crate::db::runtime_initialization_repo::InitializationState::Ready
            {
                if let Err(error) = self.queue_schedules().await {
                    tracing::error!(%error,"background.scheduled_admission_failed");
                }
                while active.len() < MAX_ACTIVE {
                    match self.start_next(&mut active).await {
                        Ok(Some((id, cancellation))) => {
                            cancellations.insert(id, cancellation);
                        }

                        Ok(None) => break,
                        Err(error) => {
                            tracing::error!(%error, "background.admission_failed");
                            break;
                        }
                    }
                }
            }
            for (id, cancellation) in &cancellations {
                match self.jobs.get(*id).await {
                    Ok(job) if job.status == JobStatus::CancelRequested => cancellation.cancel(),
                    Ok(job) => {
                        if let Some(goal_id) = job.goal_id {
                            match self.goals.get(goal_id).await {
                                Ok(goal)
                                    if !self.config.load().goals.enabled
                                        || goal.as_ref().is_none_or(|goal| {
                                            matches!(
                                                goal.status,
                                                crate::models::types::GoalStatus::Paused
                                                    | crate::models::types::GoalStatus::Archived
                                            )
                                        }) =>
                                {
                                    if let Err(error) = self.jobs.cancel(*id).await {
                                        tracing::error!(%error,job_id=%id,"background.goal_cancel_failed");
                                    }
                                    cancellation.cancel();
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    tracing::error!(%error,job_id=%id,"background.goal_state_failed");
                                    cancellation.cancel();
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, job_id=%id, "background.state_unavailable");
                        cancellation.cancel();
                    }
                }
            }
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = self.jobs.wake.notified() => {}
                () = tokio::time::sleep(Duration::from_secs(1)) => {}
                completed = active.join_next(), if !active.is_empty() => {
                    match completed {
                        Some(Ok(id)) => { cancellations.remove(&id); }
                        Some(Err(error)) => tracing::error!(%error, "background.task_panicked"),
                        None => {}
                    }
                }
            }
        }
        for cancel in cancellations.values() {
            cancel.cancel();
        }
        while let Some(result) = active.join_next().await {
            if let Err(error) = result {
                tracing::error!(%error,"background.shutdown_failed");
            }
        }
    }

    async fn queue_schedules(&self) -> Result<(), AppError> {
        let Ok(_personal) = Arc::clone(&self.personal_data).try_read_owned() else {
            return Ok(());
        };
        self.jobs.expire_owner_inputs().await?;
        self.jobs
            .release_owner_answers(&self.schedule_route)
            .await?;
        let now = chrono::Utc::now();
        for schedule in self.jobs.due_schedules(now).await? {
            if !self.jobs.queue_has_capacity().await? {
                break;
            }
            if let Some(goal) = schedule.goal_id {
                let Some(goal) = self.goals.get(goal).await? else {
                    self.jobs.set_schedule_enabled(schedule.id, false).await?;
                    self.jobs
                        .schedule_error(schedule.id, "The associated goal was removed.")
                        .await?;
                    continue;
                };
                if !self.config.load().goals.enabled
                    || goal.status != crate::models::types::GoalStatus::Active
                {
                    continue;
                }
            }
            match self
                .jobs
                .enqueue_schedule(schedule.id, now, &self.schedule_route)
                .await
            {
                Ok(Some(job)) => {
                    tracing::info!(schedule_id=%schedule.id,job_id=%job.id,"background.scheduled_work_queued");
                }
                Ok(None) => {}
                Err(error) => {
                    self.jobs
                        .schedule_error(schedule.id, &error.to_string())
                        .await?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    async fn start_next(
        &self,
        active: &mut tokio::task::JoinSet<Uuid>,
    ) -> Result<Option<(Uuid, CancellationToken)>, AppError> {
        let Some(id) = self.jobs.next_queued().await? else {
            return Ok(None);
        };
        let Ok(personal) = Arc::clone(&self.personal_data).try_read_owned() else {
            return Ok(None);
        };
        let candidate = self.jobs.get(id).await?;
        if let Some(goal_id) = candidate.goal_id
            && (!self.config.load().goals.enabled
                || self
                    .goals
                    .get(goal_id)
                    .await?
                    .is_none_or(|goal| goal.status != crate::models::types::GoalStatus::Active))
        {
            self.jobs.cancel(id).await?;
            return Ok(None);
        }
        let cancel = CancellationToken::new();
        let registration = self.runs.register_cancellation(
            id,
            cancel.clone(),
            RunKind::Background,
            candidate.channel_id,
        )?;
        if !self.jobs.claim(id).await? {
            return Ok(None);
        }
        let job = self.jobs.get(id).await?;
        let runtime = Arc::clone(&self.runtime);
        let jobs = Arc::clone(&self.jobs);
        let work_cancel = cancel.clone();
        active.spawn(async move {
            let _registration = registration;
            let _personal = personal;
            let result = runtime.run_background(&job, work_cancel.clone()).await;
            let persistence = match result {
                Ok(result) => {
                    jobs.finish(
                        id,
                        JobStatus::Completed,
                        Some(&result.output),
                        Some(&result.session_id),
                        None,
                    )
                    .await
                }
                Err(error) => {
                    jobs.finish(
                        id,
                        if work_cancel.is_cancelled() {
                            JobStatus::Interrupted
                        } else {
                            JobStatus::Failed
                        },
                        None,
                        None,
                        Some(&error.to_string()),
                    )
                    .await
                }
            };
            if let Err(error) = persistence {
                tracing::error!(%error,job_id=%id,"background.result_persistence_failed");
            }
            if let Err(error) = jobs.prune().await {
                tracing::warn!(%error,"background.retention_failed");
            }
            id
        });
        Ok(Some((id, cancel)))
    }

    async fn deliver_results(&self, shutdown: CancellationToken) {
        while !shutdown.is_cancelled() {
            if *self.initialization.borrow()
                == crate::db::runtime_initialization_repo::InitializationState::Ready
                && let Err(error) = self.deliver_one(&shutdown).await
            {
                tracing::error!(%error,"background.delivery_failed");
            }
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = self.jobs.wake.notified() => {}
                () = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }

    async fn deliver_one(&self, shutdown: &CancellationToken) -> Result<(), AppError> {
        let Some(id) = self.jobs.next_delivery().await? else {
            return Ok(());
        };
        let mut candidate = self.jobs.get(id).await?;
        if !self.goal_update_allowed(candidate.goal_id).await? {
            self.jobs
                .suppress_delivery(id, candidate.delivery_id)
                .await?;
            return Ok(());
        }
        let channel = match candidate.channel_id {
            Some(channel) => channel,
            None => self.session.update_channel().await?,
        };
        let channel = if candidate.goal_id.is_some() {
            let Some(channel) = self.connections.offer_channel(channel, false) else {
                self.jobs
                    .defer_delivery(id, candidate.delivery_id, 30)
                    .await?;
                return Ok(());
            };
            channel
        } else {
            channel
        };
        if candidate.goal_id.is_some()
            && (!self.connections.has_channel(channel) || !self.session.can_offer_update().await)
        {
            self.jobs
                .defer_delivery(id, candidate.delivery_id, 30)
                .await?;
            return Ok(());
        }
        let Ok(personal) = Arc::clone(&self.personal_data).try_read_owned() else {
            return Ok(());
        };
        if candidate.goal_id.is_some() {
            candidate.channel_id = Some(channel);
            use crate::runtime::message_handler::BackgroundOffer;
            let Some(decision) = shutdown
                .run_until_cancelled(self.session.evaluate_background_update(&candidate))
                .await
            else {
                return Ok(());
            };
            match decision {
                Ok(BackgroundOffer::Offer) => {}
                Ok(BackgroundOffer::Later { after_seconds }) => {
                    self.jobs
                        .defer_delivery(id, candidate.delivery_id, after_seconds)
                        .await?;
                    return Ok(());
                }
                Ok(BackgroundOffer::Silent) => {
                    self.jobs
                        .suppress_delivery(id, candidate.delivery_id)
                        .await?;
                    return Ok(());
                }
                Err(error) => {
                    self.jobs
                        .defer_delivery(id, candidate.delivery_id, 300)
                        .await?;
                    return Err(error);
                }
            }
            if !self.goal_update_allowed(candidate.goal_id).await? {
                self.jobs
                    .suppress_delivery(id, candidate.delivery_id)
                    .await?;
                return Ok(());
            }
            if !self.session.can_offer_update().await || !self.connections.has_channel(channel) {
                self.jobs
                    .defer_delivery(id, candidate.delivery_id, 30)
                    .await?;
                return Ok(());
            }
        }
        let Some(admission) = self.session.try_begin_background_result(channel) else {
            self.jobs
                .defer_delivery(id, candidate.delivery_id, 30)
                .await?;
            return Ok(());
        };
        let Some(job) = self
            .jobs
            .claim_delivery(id, candidate.delivery_id, channel)
            .await?
        else {
            return Ok(());
        };
        let session = Arc::clone(&self.session);
        let delivery = job.delivery_id;
        let cancellation = admission.cancellation();
        let _cancel_on_drop = cancellation.drop_guard_ref();
        let registration = self.runs.register_cancellation(
            delivery,
            cancellation.clone(),
            RunKind::Delivery,
            Some(channel),
        )?;
        let jobs = Arc::clone(&self.jobs);
        let work_cancel = cancellation.clone();
        let mut task = tokio::spawn(async move {
            let _registration = registration;
            let _admission = admission;
            let _personal = personal;
            let success = work_cancel
                .run_until_cancelled(session.handle_background_result(&job))
                .await
                .unwrap_or(false);
            session
                .settle_interrupted_turn(crate::models::ids::ConversationTurnId::from(delivery))
                .await;
            jobs.finish_delivery(id, delivery, success).await
        });
        let result = tokio::select! {
            result = &mut task => result,
            () = shutdown.cancelled() => { cancellation.cancel(); task.await }
        };
        match result {
            Ok(result) => result?,
            Err(error) => {
                if !error.is_cancelled() {
                    tracing::error!(%error,job_id=%id,"background.delivery_task_failed");
                }
                self.jobs.finish_delivery(id, delivery, false).await?;
            }
        }
        Ok(())
    }
}
