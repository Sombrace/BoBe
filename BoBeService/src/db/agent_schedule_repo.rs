use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::agent_job_repo::{AgentJob, AgentJobRepo, JobInput, TaskBudget};
use crate::error::AppError;
use crate::models::ids::{ConversationChannelId, GoalId};
use crate::provider::WorkloadRoute;

const MAX_ENABLED: i64 = 64;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub(crate) struct AgentSchedule {
    pub(crate) id: Uuid,
    pub(crate) title: String,
    pub(crate) task: String,
    pub(crate) context: String,
    pub(crate) channel_id: ConversationChannelId,
    pub(crate) goal_id: Option<GoalId>,
    pub(crate) next_run_at: String,
    pub(crate) every_seconds: Option<u32>,
    pub(crate) enabled: bool,
    pub(crate) generation: i64,
    pub(crate) last_job_id: Option<Uuid>,
    pub(crate) last_error: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    #[serde(skip)]
    definition_json: String,
    #[serde(skip)]
    budget_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScheduleInput {
    pub(crate) title: String,
    pub(crate) task: String,
    #[serde(default)]
    pub(crate) context: String,
    pub(crate) at: DateTime<Utc>,
    pub(crate) every_seconds: Option<u32>,
    pub(crate) goal_id: Option<GoalId>,
    #[serde(default)]
    pub(crate) budget: TaskBudget,
}

fn timestamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(super) fn next_due(
    due: DateTime<Utc>,
    interval: u32,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, AppError> {
    if interval == 0 {
        return Err(AppError::Validation(
            "schedule interval must be positive".into(),
        ));
    }
    let skipped = now.signed_duration_since(due).num_seconds().max(0) / i64::from(interval);
    due.checked_add_signed(chrono::Duration::seconds(
        (skipped + 1) * i64::from(interval),
    ))
    .ok_or_else(|| AppError::Validation("schedule time is out of range".into()))
}

impl AgentJobRepo {
    pub(crate) async fn create_schedule(
        &self,
        source: &str,
        input: ScheduleInput,
        channel: ConversationChannelId,
    ) -> Result<AgentSchedule, AppError> {
        if input.title.trim().is_empty()
            || input.title.len() > 160
            || input.task.trim().is_empty()
            || input.task.len() > 32768
            || input.context.len() > 12000
            || input
                .every_seconds
                .is_some_and(|seconds| seconds == 0 || seconds > 31_536_000)
        {
            return Err(AppError::Validation(
                "schedule requires a bounded title, task, context, UTC time and positive interval"
                    .into(),
            ));
        }
        input.budget.resolve(&self.limits)?;
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(existing) =
            sqlx::query_as::<_, AgentSchedule>("SELECT * FROM agent_schedules WHERE source_key=?1")
                .bind(source)
                .fetch_optional(&mut *transaction)
                .await?
        {
            if serde_json::from_str::<ScheduleInput>(&existing.definition_json)? != input
                || existing.channel_id != channel
            {
                return Err(AppError::Conflict(
                    "schedule identity was reused for different work".into(),
                ));
            }
            return Ok(existing);
        }
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM agent_work_tombstones WHERE source_key=?1)",
        )
        .bind(format!("schedule:{source}"))
        .fetch_one(&mut *transaction)
        .await?
        {
            return Err(AppError::RequestReplayUnsafe(
                "This schedule was removed and cannot be recreated by retrying its old request."
                    .into(),
            ));
        }
        let enabled: i64 =
            sqlx::query_scalar("SELECT count(*) FROM agent_schedules WHERE enabled=1 OR every_seconds IS NOT NULL OR last_job_id IS NULL")
                .fetch_one(&mut *transaction)
                .await?;
        if enabled >= MAX_ENABLED {
            return Err(AppError::Conflict("active schedule limit reached".into()));
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO agent_schedules (id,source_key,title,task,context,channel_id,goal_id,next_run_at,every_seconds,budget_json,definition_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)")
            .bind(id).bind(source).bind(&input.title).bind(&input.task).bind(&input.context)
            .bind(channel).bind(input.goal_id).bind(timestamp(input.at)).bind(input.every_seconds)
            .bind(serde_json::to_string(&input.budget)?).bind(serde_json::to_string(&input)?)
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        self.schedule(id).await
    }

    pub(crate) async fn schedule(&self, id: Uuid) -> Result<AgentSchedule, AppError> {
        sqlx::query_as("SELECT * FROM agent_schedules WHERE id=?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("schedule was not found".into()))
    }

    pub(crate) async fn schedules(&self) -> Result<Vec<AgentSchedule>, AppError> {
        Ok(sqlx::query_as(
            "SELECT * FROM agent_schedules ORDER BY enabled DESC,next_run_at,id LIMIT 600",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub(crate) async fn due_schedules(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<AgentSchedule>, AppError> {
        Ok(sqlx::query_as("SELECT * FROM agent_schedules WHERE enabled=1 AND next_run_at<=?1 ORDER BY next_run_at,id LIMIT 64")
            .bind(timestamp(now)).fetch_all(&self.pool).await?)
    }

    pub(crate) async fn set_schedule_enabled(
        &self,
        id: Uuid,
        enabled: bool,
    ) -> Result<AgentSchedule, AppError> {
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let current: AgentSchedule = sqlx::query_as("SELECT * FROM agent_schedules WHERE id=?1")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AppError::NotFound("schedule was not found".into()))?;
        if enabled
            && !current.enabled
            && current.every_seconds.is_none()
            && current.last_job_id.is_some()
        {
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM agent_schedules WHERE enabled=1 OR every_seconds IS NOT NULL OR last_job_id IS NULL")
                    .fetch_one(&mut *transaction)
                    .await?;
            if count >= MAX_ENABLED {
                return Err(AppError::Conflict("active schedule limit reached".into()));
            }
        }
        sqlx::query("UPDATE agent_schedules SET enabled=?2,generation=generation+1,last_error=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND enabled!=?2")
            .bind(id).bind(enabled).execute(&mut *transaction).await?;
        if !enabled {
            sqlx::query("UPDATE agent_jobs SET status='cancelled',delivery_status='suppressed',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE schedule_id=?1 AND status='queued'")
                .bind(id).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        self.wake.notify_waiters();
        self.schedule(id).await
    }

    pub(crate) async fn reschedule(
        &self,
        id: Uuid,
        generation: i64,
        at: DateTime<Utc>,
    ) -> Result<AgentSchedule, AppError> {
        let changed = sqlx::query("UPDATE agent_schedules SET next_run_at=?3,generation=generation+1,last_error=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND generation=?2")
            .bind(id).bind(generation).bind(timestamp(at)).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(AppError::Conflict(
                "schedule changed; read its current state before rescheduling".into(),
            ));
        }
        self.wake.notify_waiters();
        self.schedule(id).await
    }

    pub(crate) async fn delete_schedule(&self, id: Uuid) -> Result<(), AppError> {
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let source: String =
            sqlx::query_scalar("SELECT source_key FROM agent_schedules WHERE id=?1")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| AppError::NotFound("schedule was not found".into()))?;
        sqlx::query("INSERT OR IGNORE INTO agent_work_tombstones (source_key) VALUES (?1)")
            .bind(format!("schedule:{source}"))
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE agent_jobs SET status='cancelled',delivery_status='suppressed' WHERE schedule_id=?1 AND status='queued'")
            .bind(id).execute(&mut *transaction).await?;
        sqlx::query("DELETE FROM agent_schedules WHERE id=?1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        Ok(())
    }

    pub(crate) async fn schedule_error(&self, id: Uuid, error: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE agent_schedules SET last_error=?2 WHERE id=?1")
            .bind(id)
            .bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn enqueue_schedule(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
        route: &WorkloadRoute,
    ) -> Result<Option<AgentJob>, AppError> {
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some(schedule) = sqlx::query_as::<_, AgentSchedule>(
            "SELECT * FROM agent_schedules WHERE id=?1 AND enabled=1 AND next_run_at<=?2",
        )
        .bind(id)
        .bind(timestamp(now))
        .fetch_optional(&mut *transaction)
        .await?
        else {
            return Ok(None);
        };
        let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_jobs WHERE schedule_id=?1 AND status IN ('queued','running','cancel_requested','awaiting_input'))")
            .bind(id).fetch_one(&mut *transaction).await?;
        let due = schedule
            .next_run_at
            .parse::<DateTime<Utc>>()
            .map_err(|error| {
                AppError::Internal(format!("invalid retained schedule time: {error}"))
            })?;
        if active {
            if let Some(interval) = schedule.every_seconds {
                sqlx::query("UPDATE agent_schedules SET next_run_at=?2 WHERE id=?1")
                    .bind(id)
                    .bind(timestamp(next_due(due, interval, now)?))
                    .execute(&mut *transaction)
                    .await?;
            }
            transaction.commit().await?;
            return Ok(None);
        }
        let mut budget: TaskBudget = serde_json::from_str(&schedule.budget_json)?;
        budget.timeout_seconds = budget
            .timeout_seconds
            .map(|value| value.min(self.limits.task_timeout_seconds));
        budget.max_turns = budget
            .max_turns
            .map(|value| value.min(self.limits.task_max_turns));
        let job = self.enqueue_in(&mut transaction,
            &format!("schedule:{}:{}:{}",schedule.id,schedule.generation,schedule.next_run_at),
            JobInput {
                title: schedule.title,
                task: schedule.task,
                context: format!("{}\n{}",crate::provider::event_block("bobe.scheduled_work",serde_json::json!({
                    "intended_at":schedule.next_run_at,"woke_at":timestamp(now),"previous_job_id":schedule.last_job_id,
                })),schedule.context),
                budget,channel_id: Some(schedule.channel_id),goal_id:schedule.goal_id,schedule_id:Some(id),
            },route,
        ).await?;
        let next = schedule
            .every_seconds
            .map(|interval| next_due(due, interval, now))
            .transpose()?;
        sqlx::query("UPDATE agent_schedules SET enabled=?2,next_run_at=?3,last_job_id=?4,last_error=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1")
            .bind(id).bind(next.is_some()).bind(next.map(timestamp).unwrap_or(schedule.next_run_at))
            .bind(job.id).execute(&mut *transaction).await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        Ok(Some(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(at: DateTime<Utc>, interval: Option<u32>) -> ScheduleInput {
        ScheduleInput {
            title: "Future work".into(),
            task: "Inspect the workspace".into(),
            context: String::new(),
            at,
            every_seconds: interval,
            goal_id: None,
            budget: TaskBudget::default(),
        }
    }

    fn route() -> WorkloadRoute {
        WorkloadRoute {
            profile: crate::provider::ProfileId::from_static("test"),
            model: None,
            reasoning_effort: None,
        }
    }

    #[tokio::test]
    async fn restart_coalesces_missed_intervals_and_once_jobs_are_not_replayed() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let due = "2026-01-01T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let later = due + chrono::Duration::hours(10);
        for interval in [None, Some(60)] {
            let schedule = repo
                .create_schedule(
                    &Uuid::new_v4().to_string(),
                    input(due, interval),
                    ConversationChannelId::OWNER,
                )
                .await
                .unwrap();
            let restarted = AgentJobRepo::new(pool.clone());
            let job = restarted
                .enqueue_schedule(schedule.id, later, &route())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(job.schedule_id, Some(schedule.id));
            let marker = job
                .context
                .strip_prefix("[bobe.scheduled_work]\n")
                .and_then(|rest| rest.split_once("\n[/bobe.scheduled_work]"))
                .map(|(data, _)| data)
                .expect("scheduled work starts with one daemon event block");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(marker).unwrap(),
                serde_json::json!({
                    "intended_at": schedule.next_run_at,
                    "woke_at": timestamp(later),
                    "previous_job_id": null,
                })
            );
            assert!(
                restarted
                    .enqueue_schedule(schedule.id, later, &route())
                    .await
                    .unwrap()
                    .is_none()
            );
            let retained = restarted.schedule(schedule.id).await.unwrap();
            assert_eq!(retained.enabled, interval.is_some());
            if interval.is_some() {
                assert_eq!(
                    retained.next_run_at,
                    (later + chrono::Duration::minutes(1))
                        .to_rfc3339_opts(SecondsFormat::Millis, true)
                );
            }
        }
        assert_eq!(repo.list().await.unwrap().len(), 2);
        pool.close().await;
    }

    #[tokio::test]
    async fn failed_enqueue_does_not_advance_the_clock_and_removed_intents_do_not_reappear() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let now = Utc::now();
        let schedule = repo
            .create_schedule("once", input(now, None), ConversationChannelId::OWNER)
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fail_job BEFORE INSERT ON agent_jobs BEGIN SELECT RAISE(ABORT,'job insert failed'); END").execute(&pool).await.unwrap();
        assert!(
            repo.enqueue_schedule(schedule.id, now, &route())
                .await
                .is_err()
        );
        let retained = repo.schedule(schedule.id).await.unwrap();
        assert!(retained.enabled);
        assert!(retained.last_job_id.is_none());
        sqlx::query("DROP TRIGGER fail_job")
            .execute(&pool)
            .await
            .unwrap();
        repo.delete_schedule(schedule.id).await.unwrap();
        assert!(
            repo.create_schedule("once", input(now, None), ConversationChannelId::OWNER)
                .await
                .is_err()
        );
        pool.close().await;
    }
}
