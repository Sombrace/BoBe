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
const STOPPED_UNEXPECTEDLY: &str = "Background work stopped unexpectedly; external effects may be incomplete. Inspect before resuming.";

#[derive(Default)]
struct ActiveJobs {
    tasks: tokio::task::JoinSet<()>,
    jobs: std::collections::HashMap<tokio::task::Id, (Uuid, CancellationToken)>,
}

impl ActiveJobs {
    fn spawn(
        &mut self,
        id: Uuid,
        cancellation: CancellationToken,
        work: impl Future<Output = ()> + Send + 'static,
    ) {
        let task = self.tasks.spawn(work);
        self.jobs.insert(task.id(), (id, cancellation));
    }

    async fn settle(
        &mut self,
        repo: &AgentJobRepo,
        completed: Result<(tokio::task::Id, ()), tokio::task::JoinError>,
    ) {
        let task = match &completed {
            Ok((task, ())) => *task,
            Err(error) => error.id(),
        };
        let Some((id, _)) = self.jobs.remove(&task) else {
            return;
        };
        if let Err(error) = completed {
            tracing::error!(%error, job_id=%id, "background.task_panicked");
            if let Err(error) = repo
                .finish(
                    id,
                    JobStatus::Failed,
                    None,
                    None,
                    Some(STOPPED_UNEXPECTEDLY),
                )
                .await
            {
                tracing::error!(%error,job_id=%id,"background.result_persistence_failed");
            }
        }
    }
}

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
        let mut active = ActiveJobs::default();
        while !shutdown.is_cancelled() {
            if self.enabled
                && *self.initialization.borrow()
                    == crate::db::runtime_initialization_repo::InitializationState::Ready
            {
                if let Err(error) = self.queue_schedules().await {
                    tracing::error!(%error,"background.scheduled_admission_failed");
                }
                while active.tasks.len() < MAX_ACTIVE {
                    match self.start_next(&mut active).await {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) => {
                            tracing::error!(%error, "background.admission_failed");
                            break;
                        }
                    }
                }
            }
            for (id, cancellation) in active.jobs.values() {
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
                completed = active.tasks.join_next_with_id(), if !active.tasks.is_empty() => {
                    if let Some(completed) = completed {
                        active.settle(&self.jobs, completed).await;
                    }
                }
            }
        }
        for (_, cancel) in active.jobs.values() {
            cancel.cancel();
        }
        while let Some(completed) = active.tasks.join_next_with_id().await {
            active.settle(&self.jobs, completed).await;
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

    async fn start_next(&self, active: &mut ActiveJobs) -> Result<bool, AppError> {
        let Some(id) = self.jobs.next_queued().await? else {
            return Ok(false);
        };
        let Ok(personal) = Arc::clone(&self.personal_data).try_read_owned() else {
            return Ok(false);
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
            return Ok(false);
        }
        let cancel = CancellationToken::new();
        let registration = self.runs.register_cancellation(
            id,
            cancel.clone(),
            RunKind::Background,
            candidate.channel_id,
        )?;
        if !self.jobs.claim(id).await? {
            return Ok(false);
        }
        let job = self.jobs.get(id).await?;
        let runtime = Arc::clone(&self.runtime);
        let jobs = Arc::clone(&self.jobs);
        let work_cancel = cancel.clone();
        active.spawn(id, cancel, async move {
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
        });
        Ok(true)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_panicking_job_is_released_and_settles_as_failed() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let route = crate::provider::WorkloadRoute {
            profile: crate::provider::ProfileId::from_static("test"),
            model: Some("model".into()),
            reasoning_effort: None,
        };
        let input = || crate::db::agent_job_repo::JobInput {
            title: "Bounded work".into(),
            task: "Inspect the workspace".into(),
            ..crate::db::agent_job_repo::JobInput::default()
        };
        let panicking = repo.submit("panics", input(), &route).await.unwrap();
        let finishing = repo.submit("finishes", input(), &route).await.unwrap();
        assert!(repo.claim(panicking.id).await.unwrap());
        assert!(repo.claim(finishing.id).await.unwrap());

        let mut active = ActiveJobs::default();
        active.spawn(panicking.id, CancellationToken::new(), async {
            panic!("background work panicked");
        });
        let finished = Arc::clone(&repo);
        active.spawn(finishing.id, CancellationToken::new(), async move {
            finished
                .finish(finishing.id, JobStatus::Completed, Some("done"), None, None)
                .await
                .unwrap();
        });
        while let Some(completed) = active.tasks.join_next_with_id().await {
            active.settle(&repo, completed).await;
        }

        assert!(active.jobs.is_empty());
        let panicked = repo.get(panicking.id).await.unwrap();
        assert_eq!(panicked.status, JobStatus::Failed);
        assert_eq!(panicked.error.as_deref(), Some(STOPPED_UNEXPECTEDLY));
        let finished = repo.get(finishing.id).await.unwrap();
        assert_eq!(finished.status, JobStatus::Completed);
        assert_eq!(finished.error, None);
        pool.close().await;
    }
}
