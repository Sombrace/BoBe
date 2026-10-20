use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::agent_job_repo::{AgentJobRepo, JobInput, TaskBudget};
use crate::error::AppError;
use crate::models::ids::{ConversationChannelId, GoalId};
use crate::provider::WorkloadRoute;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub(crate) enum InputKind {
    Approval,
    Information,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub(crate) enum InputStatus {
    Pending,
    Answered,
    Declined,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputChoice {
    pub(crate) value: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AskOwner {
    pub(crate) kind: InputKind,
    pub(crate) question: String,
    pub(crate) action: String,
    pub(crate) task: String,
    #[serde(default)]
    pub(crate) context: String,
    #[serde(default)]
    pub(crate) choices: Vec<InputChoice>,
    #[serde(default = "default_expiry")]
    pub(crate) expires_seconds: u32,
    #[serde(default)]
    pub(crate) budget: TaskBudget,
}

fn default_expiry() -> u32 {
    86400
}
fn timestamp(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Nanos, true)
}
fn normalized(text: &str) -> String {
    text.trim()
        .trim_end_matches(['.', '!', '?'])
        .trim()
        .to_lowercase()
}

#[derive(Clone, Copy)]
pub(crate) struct InputOrigin {
    pub(crate) run_id: Uuid,
    pub(crate) job_id: Option<Uuid>,
    pub(crate) channel: ConversationChannelId,
    pub(crate) goal_id: Option<GoalId>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub(crate) struct OwnerInput {
    pub(crate) id: Uuid,
    pub(crate) origin_run_id: Uuid,
    pub(crate) origin_settled: bool,
    pub(crate) job_id: Option<Uuid>,
    pub(crate) resumed_job_id: Option<Uuid>,
    pub(crate) channel_id: ConversationChannelId,
    pub(crate) goal_id: Option<GoalId>,
    pub(crate) kind: InputKind,
    pub(crate) question: String,
    pub(crate) action: String,
    pub(crate) prompt: String,
    pub(crate) status: InputStatus,
    pub(crate) expires_at: String,
    pub(crate) response: Option<String>,
    pub(crate) response_id: Option<Uuid>,
    pub(crate) created_at: String,
    #[serde(skip)]
    choices_json: String,
    #[serde(skip)]
    definition_json: String,
}

#[derive(Serialize)]
pub(crate) struct InputView {
    #[serde(flatten)]
    input: OwnerInput,
    choices: Vec<InputChoice>,
}

impl OwnerInput {
    pub(crate) fn view(self) -> Result<InputView, AppError> {
        let choices = serde_json::from_str(&self.choices_json)?;
        Ok(InputView {
            input: self,
            choices,
        })
    }

    pub(crate) fn visible_in(&self, channel: ConversationChannelId) -> bool {
        self.channel_id == channel || self.goal_id.is_some()
    }

    fn answer_value(&self, response: &str) -> Result<Option<String>, AppError> {
        let choices: Vec<InputChoice> = serde_json::from_str(&self.choices_json)?;
        if self.kind == InputKind::Information && choices.is_empty() {
            return Ok(Some(response.trim().to_owned()));
        }
        let wanted = normalized(response);
        Ok(choices
            .into_iter()
            .find(|choice| {
                normalized(&choice.label) == wanted || normalized(&choice.value) == wanted
            })
            .map(|choice| choice.value))
    }
}

impl AgentJobRepo {
    pub(crate) async fn pending_job_input(
        &self,
        job: Uuid,
    ) -> Result<Option<OwnerInput>, AppError> {
        Ok(sqlx::query_as("SELECT * FROM owner_inputs WHERE job_id=?1 AND status='pending' AND expires_at>?2 ORDER BY rowid DESC LIMIT 1")
            .bind(job).bind(timestamp(Utc::now())).fetch_optional(&self.pool).await?)
    }
}

impl AgentJobRepo {
    pub(crate) async fn ask_owner(
        &self,
        source: &str,
        origin: InputOrigin,
        mut input: AskOwner,
    ) -> Result<OwnerInput, AppError> {
        if input.kind == InputKind::Approval && input.choices.is_empty() {
            input.choices = vec![
                InputChoice {
                    value: "approve".into(),
                    label: "Yes".into(),
                },
                InputChoice {
                    value: "decline".into(),
                    label: "No".into(),
                },
            ];
        }
        let values: std::collections::HashSet<_> = input
            .choices
            .iter()
            .map(|choice| normalized(&choice.value))
            .collect();
        let labels: std::collections::HashSet<_> = input
            .choices
            .iter()
            .map(|choice| normalized(&choice.label))
            .collect();
        if input.question.trim().is_empty()
            || input.question.len() > 2000
            || input.action.trim().is_empty()
            || input.action.len() > 2000
            || input.task.trim().is_empty()
            || input.task.len() > 32768
            || input.context.len() > 8000
            || !(60..=604_800).contains(&input.expires_seconds)
            || input.choices.len() > 8
            || values.len() != input.choices.len()
            || labels.len() != input.choices.len()
            || input.choices.iter().any(|choice| {
                choice.value.trim().is_empty()
                    || choice.value.len() > 128
                    || choice.label.trim().is_empty()
                    || choice.label.len() > 128
            })
            || input.choices.iter().enumerate().any(|(index, choice)| {
                input.choices.iter().enumerate().any(|(other, value)| {
                    index != other && normalized(&choice.value) == normalized(&value.label)
                })
            })
            || (input.kind == InputKind::Approval
                && (values.len() != 2
                    || !input.choices.iter().any(|choice| choice.value == "approve")
                    || !input.choices.iter().any(|choice| choice.value == "decline")))
        {
            return Err(AppError::Validation("owner input requires an important, bounded question, action, continuation and unambiguous choices".into()));
        }
        input.budget.resolve(&self.limits)?;
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(existing) =
            sqlx::query_as::<_, OwnerInput>("SELECT * FROM owner_inputs WHERE source_key=?1")
                .bind(source)
                .fetch_optional(&mut *transaction)
                .await?
        {
            if existing.origin_run_id != origin.run_id
                || serde_json::from_str::<AskOwner>(&existing.definition_json)? != input
            {
                return Err(AppError::Conflict(
                    "owner input identity was reused with different work".into(),
                ));
            }
            return Ok(existing);
        }
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM owner_inputs WHERE status='pending'")
                .fetch_one(&mut *transaction)
                .await?;
        let same_run: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM owner_inputs WHERE origin_run_id=?1 AND status='pending')",
        )
        .bind(origin.run_id)
        .fetch_one(&mut *transaction)
        .await?;
        if pending >= 8 || same_run {
            return Err(AppError::Conflict(
                "finish or cancel an existing owner question before asking another".into(),
            ));
        }
        let retired: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_work_tombstones WHERE source_key=?1)",
        )
        .bind(format!("input:{source}"))
        .fetch_one(&mut *transaction)
        .await?;
        if retired {
            return Err(AppError::RequestReplayUnsafe(
                "this owner question was retired and cannot be replayed".into(),
            ));
        }
        let id = Uuid::new_v4();
        let prompt = if input.question.contains(&input.action) {
            input.question.clone()
        } else {
            format!("{}\n\n{}", input.action, input.question)
        };
        let prompt = if input
            .choices
            .iter()
            .all(|choice| prompt.contains(&choice.label))
        {
            prompt
        } else {
            format!(
                "{prompt}\n\n{}",
                input
                    .choices
                    .iter()
                    .map(|choice| choice.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let expiry = Utc::now() + chrono::TimeDelta::seconds(i64::from(input.expires_seconds));
        sqlx::query("INSERT INTO owner_inputs (id,source_key,origin_run_id,job_id,channel_id,goal_id,kind,question,action,prompt,choices_json,definition_json,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)")
            .bind(id).bind(source).bind(origin.run_id).bind(origin.job_id).bind(origin.channel).bind(origin.goal_id)
            .bind(input.kind).bind(&input.question).bind(&input.action).bind(prompt).bind(serde_json::to_string(&input.choices)?)
            .bind(serde_json::to_string(&input)?).bind(timestamp(expiry)).execute(&mut *transaction).await?;
        if origin.job_id.is_none() {
            sqlx::query("INSERT INTO owner_input_presentations (input_id,channel_id,turn_id) VALUES (?1,?2,?3)")
                .bind(id).bind(origin.channel).bind(origin.run_id).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        self.wake.notify_waiters();
        self.owner_input(id).await
    }

    pub(crate) async fn owner_input(&self, id: Uuid) -> Result<OwnerInput, AppError> {
        sqlx::query_as("SELECT * FROM owner_inputs WHERE id=?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("owner question was not found".into()))
    }

    pub(crate) async fn owner_inputs(
        &self,
        channel: ConversationChannelId,
    ) -> Result<Vec<OwnerInput>, AppError> {
        Ok(sqlx::query_as("SELECT * FROM owner_inputs WHERE (channel_id=?1 OR goal_id IS NOT NULL) ORDER BY status='pending' DESC,created_at DESC LIMIT 32")
            .bind(channel).fetch_all(&self.pool).await?)
    }

    pub(crate) async fn owner_input_context(
        &self,
        channel: ConversationChannelId,
        goal: Option<GoalId>,
    ) -> Result<String, AppError> {
        let records=sqlx::query_as::<_,OwnerInput>("SELECT * FROM owner_inputs WHERE (channel_id=?1 OR goal_id IS NOT NULL) AND (?2 IS NULL OR goal_id=?2) ORDER BY status='pending' DESC,created_at DESC LIMIT 4")
            .bind(channel).bind(goal).fetch_all(&self.pool).await?;
        if records.is_empty() {
            return Ok(String::new());
        }
        let facts=records.iter().map(|record|serde_json::json!({
            "id":record.id,"status":record.status,"kind":record.kind,
            "action":crate::util::text::truncate_str(&record.action,400),
            "question":crate::util::text::truncate_str(&record.question,400),
            "response":record.response.as_deref().map(|response|crate::util::text::truncate_str(response,400)),
            "resumed_job_id":record.resumed_job_id,
        })).collect::<Vec<_>>();
        Ok(crate::provider::event_block(
            "bobe.owner_input",
            serde_json::Value::Array(facts),
        ))
    }

    pub(crate) async fn offer_owner_input(
        &self,
        id: Uuid,
        run: Uuid,
        channel: ConversationChannelId,
    ) -> Result<OwnerInput, AppError> {
        let input = self.owner_input(id).await?;
        if !input.visible_in(channel)
            || input.status != InputStatus::Pending
            || input.expires_at <= timestamp(Utc::now())
        {
            return Err(AppError::Conflict(
                "this question is not pending in this channel".into(),
            ));
        }
        sqlx::query("INSERT INTO owner_input_presentations (input_id,channel_id,turn_id) VALUES (?1,?2,?3) ON CONFLICT(input_id,channel_id) DO UPDATE SET turn_id=excluded.turn_id,presented_at=NULL")
            .bind(id).bind(channel).bind(run).execute(&self.pool).await?;
        Ok(input)
    }

    pub(crate) async fn present_owner_input(
        &self,
        id: Uuid,
        channel: ConversationChannelId,
    ) -> Result<(), AppError> {
        let input = self.owner_input(id).await?;
        if !input.visible_in(channel)
            || input.status != InputStatus::Pending
            || input.expires_at <= timestamp(Utc::now())
        {
            return Err(AppError::Conflict(
                "this question is no longer pending".into(),
            ));
        }
        sqlx::query("INSERT INTO owner_input_presentations (input_id,channel_id,presented_at) VALUES (?1,?2,?3) ON CONFLICT(input_id,channel_id) DO UPDATE SET presented_at=excluded.presented_at")
            .bind(id).bind(channel).bind(timestamp(Utc::now())).execute(&self.pool).await?;
        Ok(())
    }

    pub(crate) async fn finish_input_origin(&self, run: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE owner_inputs SET origin_settled=1 WHERE origin_run_id=?1")
            .bind(run)
            .execute(&self.pool)
            .await?;
        self.wake.notify_waiters();
        Ok(())
    }

    pub(crate) async fn answer_owner_input(
        &self,
        id: Uuid,
        response_id: Uuid,
        response: &str,
        channel: ConversationChannelId,
    ) -> Result<OwnerInput, AppError> {
        if response.trim().is_empty() || response.len() > 8192 {
            return Err(AppError::Validation(
                "answer requires 1-8192 bytes of text".into(),
            ));
        }
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let input: OwnerInput = sqlx::query_as("SELECT * FROM owner_inputs WHERE id=?1")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AppError::NotFound("owner question was not found".into()))?;
        if !input.visible_in(channel) {
            return Err(AppError::NotFound(
                "owner question is not available in this channel".into(),
            ));
        }
        let value = input.answer_value(response)?.ok_or_else(|| {
            AppError::Validation("answer must match one of the offered choices".into())
        })?;
        if input.response_id == Some(response_id) {
            if input.response.as_deref() == Some(&value) {
                return Ok(input);
            }
            return Err(AppError::Conflict(
                "response identity was reused with a different answer".into(),
            ));
        }
        if input.status != InputStatus::Pending || input.expires_at <= timestamp(Utc::now()) {
            return Err(AppError::Conflict(
                "owner question is no longer pending".into(),
            ));
        }
        let declined = input.kind == InputKind::Approval && value == "decline";
        sqlx::query("UPDATE owner_inputs SET status=?2,response=?3,response_id=?4 WHERE id=?1")
            .bind(id)
            .bind(if declined {
                InputStatus::Declined
            } else {
                InputStatus::Answered
            })
            .bind(value)
            .bind(response_id)
            .execute(&mut *transaction)
            .await?;
        if declined {
            sqlx::query("UPDATE agent_jobs SET status='cancelled',delivery_status='suppressed' WHERE id=?1 AND status='awaiting_input'")
                .bind(input.job_id).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        self.wake.notify_waiters();
        self.owner_input(id).await
    }

    pub(crate) async fn bind_presented_input(
        &self,
        channel: ConversationChannelId,
        text: &str,
        received_at: DateTime<Utc>,
    ) -> Result<Option<Uuid>, AppError> {
        let input=sqlx::query_as::<_,OwnerInput>(
            "SELECT i.* FROM owner_inputs i JOIN owner_input_presentations p ON p.input_id=i.id
             WHERE p.channel_id=?1 AND i.status='pending' AND i.expires_at>?2 AND p.presented_at>?3
             AND p.presented_at<=?2
             AND p.turn_id=(SELECT t.id FROM conversation_turns t JOIN conversations c ON c.id=t.conversation_id WHERE c.channel_id=?1 ORDER BY t.rowid DESC LIMIT 1)
             AND EXISTS(SELECT 1 FROM conversation_turns t WHERE t.id=p.turn_id AND t.role='assistant' AND t.is_complete=1)
             ORDER BY p.presented_at DESC LIMIT 1",
        ).bind(channel).bind(timestamp(received_at)).bind(timestamp(received_at-chrono::TimeDelta::minutes(2))).fetch_optional(&self.pool).await?;
        let Some(input) = input else {
            return Ok(None);
        };
        if input.answer_value(text)?.is_none() {
            return Ok(None);
        }
        Ok(Some(input.id))
    }

    pub(crate) async fn cancel_owner_input(
        &self,
        id: Uuid,
        channel: ConversationChannelId,
    ) -> Result<OwnerInput, AppError> {
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let input: OwnerInput = sqlx::query_as("SELECT * FROM owner_inputs WHERE id=?1")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AppError::NotFound("owner question was not found".into()))?;
        if !input.visible_in(channel) {
            return Err(AppError::NotFound(
                "owner question is not available in this channel".into(),
            ));
        }
        if input.status == InputStatus::Answered {
            return Err(AppError::Conflict(
                "this answer was already saved; stop its continuation task instead".into(),
            ));
        }
        sqlx::query("UPDATE owner_inputs SET status='cancelled' WHERE id=?1 AND status='pending'")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE agent_jobs SET status='cancelled',delivery_status='suppressed' WHERE id=?1 AND status='awaiting_input'")
            .bind(input.job_id).execute(&mut *transaction).await?;
        transaction.commit().await?;
        self.wake.notify_waiters();
        self.owner_input(id).await
    }

    pub(crate) async fn expire_owner_inputs(&self) -> Result<(), AppError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE owner_inputs SET status='expired' WHERE status='pending' AND expires_at<=?1",
        )
        .bind(timestamp(Utc::now()))
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE agent_jobs SET status='cancelled',delivery_status='suppressed' WHERE status='awaiting_input' AND EXISTS(SELECT 1 FROM owner_inputs WHERE job_id=agent_jobs.id AND status IN ('expired','cancelled','declined'))")
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn release_owner_answers(
        &self,
        route: &WorkloadRoute,
    ) -> Result<(), AppError> {
        let _lock = self.mutation.lock().await;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let answers=sqlx::query_as::<_,OwnerInput>("SELECT * FROM owner_inputs WHERE status='answered' AND origin_settled=1 AND resumed_job_id IS NULL ORDER BY created_at LIMIT 8")
            .fetch_all(&mut *transaction).await?;
        let mut released = false;
        for input in answers {
            let definition: AskOwner = serde_json::from_str(&input.definition_json)?;
            let answer = serde_json::json!({
                "request_id":input.id,"kind":input.kind,"question":input.question,"action":input.action,
                "owner_answer":input.response,
            });
            let job = if let Some(id) = input.job_id {
                let changed=sqlx::query("UPDATE agent_jobs SET status='queued',attempt=attempt+1,resume_context=?2,output=NULL,error=NULL,delivery_id=?3,delivery_status='pending',delivery_after=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status='awaiting_input' AND delivery_status!='delivering'")
                    .bind(id).bind(answer.to_string()).bind(Uuid::new_v4()).execute(&mut *transaction).await?.rows_affected();
                if changed == 0 {
                    continue;
                }
                id
            } else {
                let count:i64=sqlx::query_scalar("SELECT count(*) FROM agent_jobs WHERE status IN ('queued','running','cancel_requested','awaiting_input')")
                    .fetch_one(&mut *transaction).await?;
                if count >= super::agent_job_repo::MAX_OUTSTANDING {
                    break;
                }
                let mut budget = definition.budget;
                budget.timeout_seconds = budget
                    .timeout_seconds
                    .map(|seconds| seconds.min(self.limits.task_timeout_seconds));
                budget.max_turns = budget
                    .max_turns
                    .map(|turns| turns.min(self.limits.task_max_turns));
                self.enqueue_in(
                    &mut transaction,
                    &format!("owner-input:{}", input.id),
                    JobInput {
                        title: crate::util::text::truncate_str(&input.action, 160).to_owned(),
                        task: definition.task,
                        context: format!(
                            "{}\n{}",
                            crate::provider::event_block("bobe.owner_response", answer),
                            definition.context
                        ),
                        budget,
                        channel_id: Some(input.channel_id),
                        goal_id: input.goal_id,
                        ..Default::default()
                    },
                    route,
                )
                .await?
                .id
            };
            sqlx::query(
                "UPDATE owner_inputs SET resumed_job_id=?2 WHERE id=?1 AND resumed_job_id IS NULL",
            )
            .bind(input.id)
            .bind(job)
            .execute(&mut *transaction)
            .await?;
            released = true;
        }
        transaction.commit().await?;
        if released {
            self.wake.notify_waiters();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::agent_job_repo::JobStatus;
    use crate::models::ids::{ConversationId, ConversationTurnId};

    fn question() -> AskOwner {
        AskOwner {
            kind: InputKind::Approval,
            question: "Send this draft?".into(),
            action: "Email the reviewed draft to Pat.".into(),
            task: "Inspect the saved draft and send only the reviewed version if approved.".into(),
            context: String::new(),
            choices: Vec::new(),
            expires_seconds: 600,
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

    fn origin(run_id: Uuid, job: Option<Uuid>) -> InputOrigin {
        InputOrigin {
            run_id,
            job_id: job,
            channel: ConversationChannelId::OWNER,
            goal_id: None,
        }
    }

    #[tokio::test]
    async fn owner_input_context_is_one_data_block_or_nothing() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        assert_eq!(
            repo.owner_input_context(ConversationChannelId::OWNER, None)
                .await
                .unwrap(),
            ""
        );
        let input = repo
            .ask_owner("request", origin(Uuid::new_v4(), None), question())
            .await
            .unwrap();
        let context = repo
            .owner_input_context(ConversationChannelId::OWNER, None)
            .await
            .unwrap();
        let data = context
            .strip_prefix("[bobe.owner_input]\n")
            .and_then(|rest| rest.strip_suffix("\n[/bobe.owner_input]"))
            .expect("owner input context is one daemon event block");
        let facts: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(facts[0]["id"], serde_json::json!(input.id));
        assert_eq!(facts[0]["status"], "pending");
        assert_eq!(facts[0]["action"], "Email the reviewed draft to Pat.");
        pool.close().await;
    }

    #[tokio::test]
    async fn a_saved_answer_waits_for_origin_settlement_then_releases_exactly_one_continuation() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let run = Uuid::new_v4();
        let input = repo
            .ask_owner("request", origin(run, None), question())
            .await
            .unwrap();
        let answer = Uuid::new_v4();
        assert_eq!(
            repo.answer_owner_input(input.id, answer, "Yes", ConversationChannelId::OWNER)
                .await
                .unwrap()
                .status,
            InputStatus::Answered
        );
        repo.release_owner_answers(&route()).await.unwrap();
        assert!(repo.list().await.unwrap().is_empty());
        repo.finish_input_origin(run).await.unwrap();
        repo.release_owner_answers(&route()).await.unwrap();
        let restarted = AgentJobRepo::new(pool.clone());
        restarted.recover().await.unwrap();
        restarted.release_owner_answers(&route()).await.unwrap();
        assert_eq!(restarted.list().await.unwrap().len(), 1);
        assert!(
            restarted
                .answer_owner_input(input.id, answer, "No", ConversationChannelId::OWNER)
                .await
                .is_err()
        );
        assert!(
            restarted
                .answer_owner_input(
                    input.id,
                    Uuid::new_v4(),
                    "Yes",
                    ConversationChannelId::OWNER
                )
                .await
                .is_err()
        );
        assert_eq!(
            restarted
                .answer_owner_input(input.id, answer, "Yes", ConversationChannelId::OWNER)
                .await
                .unwrap()
                .status,
            InputStatus::Answered
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn a_background_question_survives_crash_without_replaying_or_bypassing_its_decision() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let job = repo
            .submit(
                "job",
                JobInput {
                    title: "Draft".into(),
                    task: "Prepare draft".into(),
                    ..Default::default()
                },
                &route(),
            )
            .await
            .unwrap();
        repo.claim(job.id).await.unwrap();
        let input = repo
            .ask_owner("question", origin(job.id, Some(job.id)), question())
            .await
            .unwrap();
        repo.recover().await.unwrap();
        assert_eq!(
            repo.get(job.id).await.unwrap().status,
            JobStatus::AwaitingInput
        );
        assert!(repo.next_queued().await.unwrap().is_none());
        assert!(repo.resume(job.id).await.is_err());
        repo.answer_owner_input(
            input.id,
            Uuid::new_v4(),
            "approve",
            ConversationChannelId::OWNER,
        )
        .await
        .unwrap();
        repo.release_owner_answers(&route()).await.unwrap();
        let resumed = repo.get(job.id).await.unwrap();
        assert_eq!(resumed.status, JobStatus::Queued);
        assert_eq!(resumed.attempt, 2);
        assert!(resumed.resume_context.contains("approve"));
        repo.release_owner_answers(&route()).await.unwrap();
        assert_eq!(repo.get(job.id).await.unwrap().attempt, 2);
        pool.close().await;
    }

    #[tokio::test]
    async fn implicit_yes_requires_this_channels_last_complete_actually_presented_question() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let run = Uuid::new_v4();
        let input = repo
            .ask_owner("question", origin(run, None), question())
            .await
            .unwrap();
        let received_before_question = Utc::now() - chrono::TimeDelta::seconds(1);
        assert!(
            repo.bind_presented_input(ConversationChannelId::OWNER, "yes", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
        let conversation = ConversationId::new();
        sqlx::query("INSERT INTO conversations (id,state) VALUES (?1,'active')")
            .bind(conversation)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversation_turns (id,conversation_id,role,content,is_complete) VALUES (?1,?2,'assistant',?3,1)")
            .bind(run).bind(conversation).bind(&input.prompt).execute(&pool).await.unwrap();
        assert!(
            repo.bind_presented_input(ConversationChannelId::OWNER, "yes", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
        crate::db::SqliteConversationRepo::new(pool.clone())
            .acknowledge_presentation(ConversationTurnId::from(run), ConversationChannelId::OWNER)
            .await
            .unwrap();
        assert!(
            repo.bind_presented_input(
                ConversationChannelId::OWNER,
                "yes",
                received_before_question
            )
            .await
            .unwrap()
            .is_none()
        );
        let other = ConversationChannelId::for_endpoint("other");
        assert!(
            repo.bind_presented_input(other, "yes", Utc::now())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            repo.bind_presented_input(
                ConversationChannelId::OWNER,
                "yes, but change the recipient",
                Utc::now(),
            )
            .await
            .unwrap()
            .is_none()
        );
        let bound = repo
            .bind_presented_input(ConversationChannelId::OWNER, "Yes.", Utc::now())
            .await
            .unwrap()
            .unwrap();
        let response = repo
            .answer_owner_input(bound, Uuid::new_v4(), "Yes.", ConversationChannelId::OWNER)
            .await
            .unwrap();
        assert_eq!(response.status, InputStatus::Answered);
        pool.close().await;
    }

    #[tokio::test]
    async fn decline_expiry_and_ambiguous_choices_cannot_release_work() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = AgentJobRepo::new(pool.clone());
        let input = repo
            .ask_owner("decline", origin(Uuid::new_v4(), None), question())
            .await
            .unwrap();
        repo.answer_owner_input(input.id, Uuid::new_v4(), "No", ConversationChannelId::OWNER)
            .await
            .unwrap();
        repo.finish_input_origin(input.origin_run_id).await.unwrap();
        let expires = repo
            .ask_owner("expire", origin(Uuid::new_v4(), None), question())
            .await
            .unwrap();
        sqlx::query("UPDATE owner_inputs SET expires_at='2000-01-01T00:00:00.000Z' WHERE id=?1")
            .bind(expires.id)
            .execute(&pool)
            .await
            .unwrap();
        repo.expire_owner_inputs().await.unwrap();
        assert_eq!(
            repo.owner_input(expires.id).await.unwrap().status,
            InputStatus::Expired
        );
        assert!(
            repo.answer_owner_input(
                expires.id,
                Uuid::new_v4(),
                "Yes",
                ConversationChannelId::OWNER
            )
            .await
            .is_err()
        );
        let mut ambiguous = question();
        ambiguous.choices = vec![
            InputChoice {
                value: "approve".into(),
                label: "decline".into(),
            },
            InputChoice {
                value: "decline".into(),
                label: "No".into(),
            },
        ];
        assert!(
            repo.ask_owner("ambiguous", origin(Uuid::new_v4(), None), ambiguous)
                .await
                .is_err()
        );
        repo.release_owner_answers(&route()).await.unwrap();
        assert!(repo.list().await.unwrap().is_empty());
        pool.close().await;
    }
}
