use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::error::AppError;
use crate::provider::WorkloadRoute;

pub(super) const MAX_OUTSTANDING: i64 = 16;
const MAX_RETAINED: i64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub(crate) enum JobStatus {
    Queued,
    Running,
    CancelRequested,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    AwaitingInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub(crate) struct AgentJob {
    pub(crate) id: Uuid,
    pub(crate) channel_id: Option<crate::models::ids::ConversationChannelId>,
    pub(crate) goal_id: Option<crate::models::ids::GoalId>,
    pub(crate) schedule_id: Option<Uuid>,
    pub(crate) title: String,
    pub(crate) task: String,
    pub(crate) context: String,
    pub(crate) resume_context: String,
    pub(crate) timeout_seconds: u32,
    pub(crate) max_turns: u32,
    #[serde(skip)]
    budget_json: String,
    pub(crate) profile: String,
    pub(crate) model: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) status: JobStatus,
    pub(crate) attempt: i64,
    pub(crate) session_id: Option<String>,
    pub(crate) output: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) delivery_id: Uuid,
    pub(crate) delivery_status: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobInput {
    pub(crate) title: String,
    pub(crate) task: String,
    #[serde(default)]
    pub(crate) context: String,
    #[serde(default)]
    pub(crate) budget: TaskBudget,
    #[serde(skip)]
    pub(crate) channel_id: Option<crate::models::ids::ConversationChannelId>,
    #[serde(skip)]
    pub(crate) goal_id: Option<crate::models::ids::GoalId>,
    #[serde(skip)]
    pub(crate) schedule_id: Option<Uuid>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TaskBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timeout_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_turns: Option<u32>,
}

impl TaskBudget {
    pub(super) fn resolve(
        &self,
        limits: &crate::config::ExecutionLimits,
    ) -> Result<(u32, u32), AppError> {
        let timeout = self
            .timeout_seconds
            .unwrap_or(1800.min(limits.task_timeout_seconds));
        let turns = self.max_turns.unwrap_or(64.min(limits.task_max_turns));
        if timeout == 0
            || timeout > limits.task_timeout_seconds
            || turns == 0
            || turns > limits.task_max_turns
        {
            return Err(AppError::Validation(format!(
                "task budget must fit owner ceilings: {} seconds and {} turns",
                limits.task_timeout_seconds, limits.task_max_turns
            )));
        }
        Ok((timeout, turns))
    }
}

pub(crate) struct AgentJobRepo {
    pub(super) pool: SqlitePool,
    pub(crate) wake: Notify,
    pub(super) mutation: tokio::sync::Mutex<()>,
    pub(super) limits: crate::config::ExecutionLimits,
}

impl AgentJobRepo {
    pub(crate) async fn queue_has_capacity(&self) -> Result<bool, AppError> {
        let count:i64 = sqlx::query_scalar("SELECT count(*) FROM agent_jobs WHERE status IN ('queued','running','cancel_requested','awaiting_input')")
            .fetch_one(&self.pool).await?;
        Ok(count < MAX_OUTSTANDING)
    }

    pub(crate) async fn defer_delivery(
        &self,
        id: Uuid,
        delivery_id: Uuid,
        seconds: u32,
    ) -> Result<(), AppError> {
        let due =
            chrono::Utc::now() + chrono::Duration::seconds(i64::from(seconds.clamp(30, 3600)));
        sqlx::query("UPDATE agent_jobs SET delivery_status='pending',delivery_after=?3 WHERE id=?1 AND delivery_id=?2 AND delivery_status IN ('pending','delivering')")
            .bind(id).bind(delivery_id).bind(due.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).execute(&self.pool).await?;
        Ok(())
    }

    pub(crate) async fn suppress_delivery(
        &self,
        id: Uuid,
        delivery_id: Uuid,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE agent_jobs SET delivery_status='suppressed' WHERE id=?1 AND delivery_id=?2 AND delivery_status='pending'")
            .bind(id).bind(delivery_id).execute(&self.pool).await?;
        self.prune().await?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn new(pool: SqlitePool) -> Arc<Self> {
        Self::with_limits(pool, crate::config::ExecutionLimits::default())
    }

    pub(crate) fn with_limits(
        pool: SqlitePool,
        limits: crate::config::ExecutionLimits,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            wake: Notify::new(),
            mutation: tokio::sync::Mutex::new(()),
            limits,
        })
    }

    pub(crate) async fn submit(
        &self,
        source_key: &str,
        input: JobInput,
        route: &WorkloadRoute,
    ) -> Result<AgentJob, AppError> {
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let job = self
            .enqueue_in(&mut transaction, source_key, input, route)
            .await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        Ok(job)
    }

    pub(super) async fn enqueue_in(
        &self,
        connection: &mut sqlx::SqliteConnection,
        source_key: &str,
        input: JobInput,
        route: &WorkloadRoute,
    ) -> Result<AgentJob, AppError> {
        if input.title.trim().is_empty()
            || input.title.len() > 160
            || input.task.trim().is_empty()
            || input.task.len() > 32 * 1024
            || input.context.len() > 16 * 1024
        {
            return Err(AppError::Validation("background work requires a title (1-160 bytes), task (1-32768 bytes) and bounded context (16384 bytes)".into()));
        }
        if let Some(existing) =
            sqlx::query_as::<_, AgentJob>("SELECT * FROM agent_jobs WHERE source_key=?1")
                .bind(source_key)
                .fetch_optional(&mut *connection)
                .await?
        {
            if existing.task != input.task
                || existing.context != input.context
                || existing.title != input.title
                || existing.channel_id != input.channel_id
                || existing.goal_id != input.goal_id
                || existing.schedule_id != input.schedule_id
                || serde_json::from_str::<TaskBudget>(&existing.budget_json)? != input.budget
            {
                return Err(AppError::Conflict(
                    "background request identity was reused with different work".into(),
                ));
            }
            return Ok(existing);
        }
        let (timeout_seconds, max_turns) = input.budget.resolve(&self.limits)?;
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM agent_work_tombstones WHERE source_key=?1)",
        )
        .bind(source_key)
        .fetch_one(&mut *connection)
        .await?
        {
            return Err(AppError::RequestReplayUnsafe(
                "This task was retired and its old creation request cannot be repeated.".into(),
            ));
        }
        let budget_json = serde_json::to_string(&input.budget)?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_jobs WHERE status IN ('queued','running','cancel_requested','awaiting_input')")
            .fetch_one(&mut *connection).await?;
        if count >= MAX_OUTSTANDING {
            return Err(AppError::Conflict(
                "the background work queue is full; finish or cancel work before starting more"
                    .into(),
            ));
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO agent_jobs (id,source_key,title,task,context,profile,model,reasoning_effort,delivery_id,channel_id,goal_id,timeout_seconds,max_turns,budget_json,schedule_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)")
            .bind(id).bind(source_key).bind(input.title).bind(input.task).bind(input.context)
            .bind(route.profile.as_str()).bind(&route.model).bind(&route.reasoning_effort).bind(Uuid::new_v4())
            .bind(input.channel_id).bind(input.goal_id).bind(timeout_seconds).bind(max_turns).bind(budget_json).bind(input.schedule_id).execute(&mut *connection).await?;
        Ok(sqlx::query_as("SELECT * FROM agent_jobs WHERE id=?1")
            .bind(id)
            .fetch_one(connection)
            .await?)
    }

    pub(crate) async fn get(&self, id: Uuid) -> Result<AgentJob, AppError> {
        sqlx::query_as::<_, AgentJob>("SELECT * FROM agent_jobs WHERE id=?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("background job {id} was not found")))
    }

    pub(crate) async fn list(&self) -> Result<Vec<AgentJob>, AppError> {
        Ok(sqlx::query_as::<_, AgentJob>(
            "SELECT * FROM agent_jobs ORDER BY created_at DESC,id DESC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub(crate) async fn next_queued(&self) -> Result<Option<Uuid>, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM agent_jobs WHERE status='queued' ORDER BY rowid LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    pub(crate) async fn claim(&self, id: Uuid) -> Result<bool, AppError> {
        Ok(sqlx::query("UPDATE agent_jobs SET status='running',timeout_seconds=MIN(COALESCE(json_extract(budget_json,'$.timeout_seconds'),1800),?2),max_turns=MIN(COALESCE(json_extract(budget_json,'$.max_turns'),64),?3),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status='queued'")
            .bind(id).bind(self.limits.task_timeout_seconds).bind(self.limits.task_max_turns).execute(&self.pool).await?.rows_affected() == 1)
    }

    pub(crate) async fn cancel(&self, id: Uuid) -> Result<AgentJob, AppError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE agent_jobs SET status=CASE WHEN status IN ('queued','awaiting_input') THEN 'cancelled' ELSE 'cancel_requested' END,
            delivery_status='suppressed',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
            WHERE id=?1 AND status IN ('queued','running','awaiting_input')",
        ).bind(id).execute(&mut *transaction).await?;
        sqlx::query("UPDATE owner_inputs SET status='cancelled' WHERE job_id=?1 AND status IN ('pending','answered') AND resumed_job_id IS NULL")
            .bind(id).execute(&mut *transaction).await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        let job = self.get(id).await?;
        self.prune().await?;
        Ok(job)
    }

    pub(crate) async fn resume(&self, id: Uuid) -> Result<AgentJob, AppError> {
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let job = self.resume_in(&mut transaction, id, None).await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        Ok(job)
    }

    pub(super) async fn resume_in(
        &self,
        connection: &mut sqlx::SqliteConnection,
        id: Uuid,
        source: Option<&str>,
    ) -> Result<AgentJob, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_jobs WHERE status IN ('queued','running','cancel_requested','awaiting_input')")
            .fetch_one(&mut *connection).await?;
        if count >= MAX_OUTSTANDING {
            return Err(AppError::Conflict(
                "the background work queue is full".into(),
            ));
        }
        if source.is_some() {
            sqlx::query("INSERT OR IGNORE INTO agent_work_tombstones (source_key) SELECT source_key FROM agent_jobs WHERE id=?1 AND source_key!=?2")
                .bind(id).bind(source).execute(&mut *connection).await?;
        }
        let changed = sqlx::query(
            "UPDATE agent_jobs SET status='queued',attempt=attempt+1,error=NULL,output=NULL,delivery_status='pending',delivery_after=NULL,delivery_id=?2,
            source_key=COALESCE(?3,source_key),
            channel_id=CASE WHEN goal_id IS NOT NULL THEN NULL ELSE channel_id END,
            updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status IN ('interrupted','failed') AND delivery_status!='delivering'
            AND NOT EXISTS(SELECT 1 FROM owner_inputs WHERE job_id=?1 AND status IN ('pending','answered') AND resumed_job_id IS NULL)",
        ).bind(id).bind(Uuid::new_v4()).bind(source).execute(&mut *connection).await?.rows_affected();
        if changed != 1 {
            return Err(AppError::Conflict(
                "only failed or interrupted background work with no result delivery in progress can be resumed".into(),
            ));
        }
        Ok(sqlx::query_as("SELECT * FROM agent_jobs WHERE id=?1")
            .bind(id)
            .fetch_one(connection)
            .await?)
    }

    pub(crate) async fn finish(
        &self,
        id: Uuid,
        status: JobStatus,
        output: Option<&str>,
        session_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), AppError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE agent_jobs SET
              status=CASE WHEN status='cancel_requested' THEN 'cancelled'
                WHEN EXISTS(SELECT 1 FROM owner_inputs WHERE job_id=?1 AND status IN ('pending','answered') AND resumed_job_id IS NULL) THEN 'awaiting_input'
                WHEN EXISTS(SELECT 1 FROM owner_inputs WHERE job_id=?1 AND origin_settled=0 AND status IN ('declined','cancelled','expired')) THEN 'cancelled'
                ELSE ?2 END,
              output=?3,session_id=COALESCE(?4,session_id),error=?5,
              updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id=?1 AND status IN ('running','cancel_requested')",
        )
        .bind(id)
        .bind(status)
        .bind(output)
        .bind(session_id)
        .bind(error)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE owner_inputs SET origin_settled=1 WHERE origin_run_id=?1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE agent_jobs SET delivery_status='suppressed' WHERE id=?1 AND (status='cancelled' OR EXISTS(SELECT 1 FROM owner_inputs WHERE job_id=?1 AND status='answered' AND resumed_job_id IS NULL))")
            .bind(id).execute(&mut *transaction).await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        Ok(())
    }

    pub(crate) async fn recover(&self) -> Result<(), AppError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("UPDATE owner_inputs SET origin_settled=1 WHERE origin_settled=0")
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "UPDATE agent_jobs SET status=CASE WHEN status='cancel_requested' THEN 'cancelled'
             WHEN EXISTS(SELECT 1 FROM owner_inputs WHERE job_id=agent_jobs.id AND status IN ('pending','answered') AND resumed_job_id IS NULL) THEN 'awaiting_input' ELSE 'interrupted' END,
             error='Daemon stopped during execution; external effects may be incomplete. Inspect before resuming.',
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE status IN ('running','cancel_requested')",
        ).execute(&mut *transaction).await?;
        sqlx::query("UPDATE owner_inputs SET status='cancelled' WHERE status IN ('pending','answered') AND resumed_job_id IS NULL AND EXISTS(SELECT 1 FROM agent_jobs WHERE id=job_id AND status='cancelled')")
            .execute(&mut *transaction).await?;
        sqlx::query(
            "UPDATE agent_jobs SET delivery_status=CASE WHEN EXISTS(
              SELECT 1 FROM conversation_turns WHERE conversation_turns.id=agent_jobs.delivery_id AND is_complete=1
             ) THEN 'committed' ELSE 'interrupted' END WHERE delivery_status='delivering'",
        ).execute(&mut *transaction).await?;
        sqlx::query("UPDATE agent_jobs SET delivery_status='presented' WHERE delivery_status='committed' AND EXISTS(SELECT 1 FROM conversation_presentations WHERE turn_id=delivery_id AND channel_id=agent_jobs.channel_id)")
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        self.expire_owner_inputs().await?;
        self.prune().await?;
        Ok(())
    }

    pub(crate) async fn next_delivery(&self) -> Result<Option<Uuid>, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM agent_jobs WHERE status IN ('completed','failed','interrupted','awaiting_input') AND delivery_status='pending' AND (delivery_after IS NULL OR delivery_after<=strftime('%Y-%m-%dT%H:%M:%fZ','now')) ORDER BY updated_at,id LIMIT 1",
        ).fetch_optional(&self.pool).await?)
    }

    pub(crate) async fn claim_delivery(
        &self,
        id: Uuid,
        delivery_id: Uuid,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<Option<AgentJob>, AppError> {
        Ok(sqlx::query_as::<_, AgentJob>(
            "UPDATE agent_jobs SET delivery_status='delivering',channel_id=CASE WHEN goal_id IS NOT NULL THEN ?3 ELSE COALESCE(channel_id,?3) END WHERE id=?1 AND delivery_id=?2 AND delivery_status='pending' AND status IN ('completed','failed','interrupted','awaiting_input')
             RETURNING *",
        ).bind(id).bind(delivery_id).bind(channel).fetch_optional(&self.pool).await?)
    }

    pub(crate) async fn finish_delivery(
        &self,
        id: Uuid,
        delivery_id: Uuid,
        succeeded: bool,
    ) -> Result<(), AppError> {
        let changed = sqlx::query("UPDATE agent_jobs SET delivery_status=CASE
            WHEN EXISTS(SELECT 1 FROM conversation_presentations WHERE turn_id=?2 AND channel_id=agent_jobs.channel_id) THEN 'presented'
            WHEN EXISTS(SELECT 1 FROM conversation_turns WHERE id=?2 AND is_complete=1) THEN 'committed'
            WHEN ?3 THEN 'suppressed' ELSE 'interrupted' END,
            updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND delivery_id=?2 AND delivery_status IN ('delivering','presented')")
            .bind(id).bind(delivery_id).bind(succeeded).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(AppError::Conflict(
                "background result delivery no longer belongs to this attempt".into(),
            ));
        }
        self.prune().await?;
        Ok(())
    }

    pub(crate) async fn prune(&self) -> Result<(), AppError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("INSERT OR IGNORE INTO agent_work_tombstones (source_key) SELECT 'input:'||source_key FROM owner_inputs WHERE status!='pending' AND (status!='answered' OR resumed_job_id IS NOT NULL) ORDER BY created_at DESC LIMIT -1 OFFSET ?1")
            .bind(MAX_RETAINED).execute(&mut *transaction).await?;
        sqlx::query("DELETE FROM owner_inputs WHERE 'input:'||source_key IN (SELECT source_key FROM agent_work_tombstones) AND status!='pending' AND (status!='answered' OR resumed_job_id IS NOT NULL)")
            .execute(&mut *transaction).await?;
        sqlx::query("INSERT OR IGNORE INTO agent_work_tombstones (source_key) SELECT source_key FROM agent_jobs WHERE delivery_status IN ('committed','presented','suppressed') AND status IN ('completed','failed','cancelled','interrupted') ORDER BY updated_at DESC LIMIT -1 OFFSET ?1")
            .bind(MAX_RETAINED).execute(&mut *transaction).await?;
        sqlx::query("DELETE FROM agent_jobs WHERE source_key IN (SELECT source_key FROM agent_work_tombstones) AND delivery_status IN ('committed','presented','suppressed') AND status IN ('completed','failed','cancelled','interrupted')")
            .execute(&mut *transaction).await?;
        sqlx::query("INSERT OR IGNORE INTO agent_work_tombstones (source_key) SELECT 'schedule:'||source_key FROM agent_schedules WHERE enabled=0 AND every_seconds IS NULL AND last_job_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM agent_jobs WHERE id=last_job_id AND status IN ('queued','running','cancel_requested')) ORDER BY updated_at DESC LIMIT -1 OFFSET ?1")
            .bind(MAX_RETAINED).execute(&mut *transaction).await?;
        sqlx::query("DELETE FROM agent_schedules WHERE enabled=0 AND 'schedule:'||source_key IN (SELECT source_key FROM agent_work_tombstones)")
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ids::ConversationChannelId;

    #[tokio::test]
    async fn chosen_task_limits_are_durable_idempotent_and_below_owner_ceilings() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let limits = crate::config::ExecutionLimits {
            task_timeout_seconds: 90,
            task_max_turns: 8,
            ..Default::default()
        };
        let repo = AgentJobRepo::with_limits(pool.clone(), limits);
        let make = || JobInput {
            budget: TaskBudget {
                timeout_seconds: Some(30),
                max_turns: Some(3),
            },
            ..input()
        };
        let job = repo.submit("limited", make(), &route()).await.unwrap();
        assert_eq!((job.timeout_seconds, job.max_turns), (30, 3));
        assert_eq!(
            repo.submit("limited", make(), &route()).await.unwrap().id,
            job.id
        );
        assert!(repo.submit("limited", input(), &route()).await.is_err());
        assert!(
            repo.submit(
                "oversized",
                JobInput {
                    budget: TaskBudget {
                        timeout_seconds: Some(91),
                        ..Default::default()
                    },
                    ..input()
                },
                &route()
            )
            .await
            .is_err()
        );
        let restarted = AgentJobRepo::with_limits(
            pool.clone(),
            crate::config::ExecutionLimits {
                task_timeout_seconds: 20,
                task_max_turns: 2,
                ..Default::default()
            },
        );
        restarted.recover().await.unwrap();
        assert!(restarted.claim(job.id).await.unwrap());
        let running = restarted.get(job.id).await.unwrap();
        assert_eq!((running.timeout_seconds, running.max_turns), (20, 2));
        assert_eq!(
            restarted
                .submit("limited", make(), &route())
                .await
                .unwrap()
                .id,
            job.id
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn committed_updates_replay_saved_content_until_the_correct_channel_acknowledges_them() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let jobs = AgentJobRepo::new(pool.clone());
        let conversations = crate::db::SqliteConversationRepo::new(pool.clone());
        let channel = ConversationChannelId::for_endpoint("owner-mac");
        let job = jobs.submit("update", input(), &route()).await.unwrap();
        jobs.claim(job.id).await.unwrap();
        jobs.finish(job.id, JobStatus::Completed, Some("verified"), None, None)
            .await
            .unwrap();
        jobs.claim_delivery(job.id, job.delivery_id, channel)
            .await
            .unwrap()
            .unwrap();
        let conversation =
            crate::models::conversation::Conversation::new_active().in_channel(channel);
        conversations.save(&conversation).await.unwrap();
        let turn = crate::models::conversation::ConversationTurn::new_with_id(
            crate::models::ids::ConversationTurnId::from(job.delivery_id),
            conversation.id,
            crate::models::types::TurnRole::Assistant,
            "Saved useful update".into(),
        );
        conversations.add_turn(&turn).await.unwrap();
        jobs.finish_delivery(job.id, job.delivery_id, true)
            .await
            .unwrap();
        assert_eq!(jobs.get(job.id).await.unwrap().delivery_status, "committed");
        jobs.recover().await.unwrap();
        assert!(jobs.next_delivery().await.unwrap().is_none());
        assert_eq!(
            conversations.unpresented_updates(channel).await.unwrap()[0].content,
            "Saved useful update"
        );
        assert!(
            conversations
                .acknowledge_presentation(turn.id, ConversationChannelId::OWNER)
                .await
                .is_err()
        );
        conversations
            .acknowledge_presentation(turn.id, channel)
            .await
            .unwrap();
        conversations
            .acknowledge_presentation(turn.id, channel)
            .await
            .unwrap();
        assert_eq!(jobs.get(job.id).await.unwrap().delivery_status, "presented");
        assert!(
            conversations
                .unpresented_updates(channel)
                .await
                .unwrap()
                .is_empty()
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn resumed_goal_outcomes_can_follow_presence_but_user_jobs_keep_their_origin() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let first = ConversationChannelId::for_endpoint("first");
        let next = ConversationChannelId::for_endpoint("next");
        for goal_id in [None, Some(crate::models::ids::GoalId::new())] {
            let job = repo
                .submit(
                    &uuid::Uuid::new_v4().to_string(),
                    JobInput {
                        channel_id: Some(first),
                        goal_id,
                        ..input()
                    },
                    &route(),
                )
                .await
                .unwrap();
            repo.claim(job.id).await.unwrap();
            repo.finish(job.id, JobStatus::Failed, None, None, Some("interrupted"))
                .await
                .unwrap();
            let delivered = repo
                .claim_delivery(job.id, job.delivery_id, first)
                .await
                .unwrap()
                .unwrap();
            repo.finish_delivery(job.id, delivered.delivery_id, true)
                .await
                .unwrap();
            let resumed = repo.resume(job.id).await.unwrap();
            assert_eq!(
                resumed.channel_id,
                if goal_id.is_some() { None } else { Some(first) }
            );
            repo.claim(job.id).await.unwrap();
            repo.finish(job.id, JobStatus::Completed, Some("done"), None, None)
                .await
                .unwrap();
            let delivered = repo
                .claim_delivery(job.id, resumed.delivery_id, next)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                delivered.channel_id,
                Some(if goal_id.is_some() { next } else { first })
            );
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn an_update_decision_cannot_claim_a_resumed_attempt() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let original = repo.submit("goal-update", input(), &route()).await.unwrap();
        repo.claim(original.id).await.unwrap();
        repo.finish(
            original.id,
            JobStatus::Failed,
            None,
            None,
            Some("retry safely"),
        )
        .await
        .unwrap();
        let next = repo.resume(original.id).await.unwrap();
        assert_ne!(next.delivery_id, original.delivery_id);
        assert!(
            repo.claim_delivery(
                original.id,
                original.delivery_id,
                ConversationChannelId::OWNER
            )
            .await
            .unwrap()
            .is_none()
        );
        repo.suppress_delivery(original.id, original.delivery_id)
            .await
            .unwrap();
        assert_eq!(
            repo.get(original.id).await.unwrap().delivery_status,
            "pending"
        );
        pool.close().await;
    }

    fn input() -> JobInput {
        JobInput {
            title: "Bounded work".into(),
            task: "Inspect the workspace".into(),
            context: String::new(),
            ..JobInput::default()
        }
    }

    fn route() -> WorkloadRoute {
        WorkloadRoute {
            profile: crate::provider::ProfileId::from_static("test"),
            model: Some("model".into()),
            reasoning_effort: None,
        }
    }

    #[tokio::test]
    async fn jobs_are_idempotent_and_interrupted_effects_never_replay_automatically() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let first = repo.submit("source-call", input(), &route()).await.unwrap();
        assert_eq!(
            first.id,
            repo.submit("source-call", input(), &route())
                .await
                .unwrap()
                .id
        );
        assert!(repo.claim(first.id).await.unwrap());
        assert!(!repo.claim(first.id).await.unwrap());
        repo.recover().await.unwrap();
        assert_eq!(
            repo.get(first.id).await.unwrap().status,
            JobStatus::Interrupted
        );
        assert!(repo.next_queued().await.unwrap().is_none());
        assert_eq!(repo.resume(first.id).await.unwrap().attempt, 2);
        assert_eq!(repo.next_queued().await.unwrap(), Some(first.id));
        pool.close().await;
    }

    #[tokio::test]
    async fn queue_admits_exactly_its_bound_without_rejecting_idempotent_retries() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let first = repo.submit("first", input(), &route()).await.unwrap();
        for index in 1..MAX_OUTSTANDING {
            repo.submit(&format!("queued-{index}"), input(), &route())
                .await
                .unwrap();
        }
        assert!(repo.submit("overflow", input(), &route()).await.is_err());
        assert_eq!(
            repo.submit("first", input(), &route()).await.unwrap().id,
            first.id
        );
        repo.cancel(first.id).await.unwrap();
        assert_eq!(
            repo.submit("overflow", input(), &route())
                .await
                .unwrap()
                .status,
            JobStatus::Queued
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn cancellation_wins_over_late_success_and_suppresses_delivery() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let job = repo.submit("call", input(), &route()).await.unwrap();
        repo.claim(job.id).await.unwrap();
        assert_eq!(
            repo.cancel(job.id).await.unwrap().status,
            JobStatus::CancelRequested
        );
        repo.finish(
            job.id,
            JobStatus::Completed,
            Some("late result"),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(repo.get(job.id).await.unwrap().status, JobStatus::Cancelled);
        assert!(repo.next_delivery().await.unwrap().is_none());
        pool.close().await;
    }

    #[tokio::test]
    async fn uncertain_completion_delivery_is_not_replayed_after_restart() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let job = repo.submit("call", input(), &route()).await.unwrap();
        repo.claim(job.id).await.unwrap();
        repo.finish(job.id, JobStatus::Completed, Some("result"), None, None)
            .await
            .unwrap();
        assert!(
            repo.claim_delivery(job.id, job.delivery_id, ConversationChannelId::OWNER)
                .await
                .unwrap()
                .is_some()
        );
        repo.recover().await.unwrap();
        assert_eq!(
            repo.get(job.id).await.unwrap().delivery_status,
            "interrupted"
        );
        assert!(repo.next_delivery().await.unwrap().is_none());
        pool.close().await;
    }

    #[tokio::test]
    async fn delivery_claim_pins_the_attempt_until_settlement() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let job = repo.submit("call", input(), &route()).await.unwrap();
        repo.claim(job.id).await.unwrap();
        repo.finish(job.id, JobStatus::Failed, None, None, Some("failed"))
            .await
            .unwrap();
        let delivery = repo
            .claim_delivery(job.id, job.delivery_id, ConversationChannelId::OWNER)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.delivery_id, job.delivery_id);
        assert_eq!(delivery.attempt, 1);
        assert_eq!(delivery.status, JobStatus::Failed);
        assert!(repo.resume(job.id).await.is_err());
        assert!(
            repo.claim_delivery(job.id, job.delivery_id, ConversationChannelId::OWNER)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            repo.finish_delivery(job.id, Uuid::new_v4(), true)
                .await
                .is_err()
        );
        repo.finish_delivery(job.id, delivery.delivery_id, true)
            .await
            .unwrap();
        let resumed = repo.resume(job.id).await.unwrap();
        assert_eq!(resumed.attempt, 2);
        assert_ne!(resumed.delivery_id, delivery.delivery_id);
        assert!(
            repo.finish_delivery(job.id, delivery.delivery_id, true)
                .await
                .is_err()
        );
        assert!(
            repo.claim_delivery(job.id, job.delivery_id, ConversationChannelId::OWNER)
                .await
                .unwrap()
                .is_none()
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn cancelling_queued_work_still_bounds_retained_job_metadata() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        for index in 0..MAX_RETAINED + 3 {
            let job = repo
                .submit(&format!("cancel-{index}"), input(), &route())
                .await
                .unwrap();
            assert_eq!(
                repo.cancel(job.id).await.unwrap().status,
                JobStatus::Cancelled
            );
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, MAX_RETAINED);
        pool.close().await;
    }
}
