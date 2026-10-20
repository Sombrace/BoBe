use crate::error::AppError;
use crate::models::conversation::{Conversation, ConversationTurn};
use crate::models::ids::{ConversationChannelId, ConversationId, ConversationTurnId};
use crate::models::types::{ConversationState, TurnRole};
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::{debug, info, warn};

pub(crate) struct SqliteConversationRepo {
    pool: SqlitePool,
}

impl SqliteConversationRepo {
    pub(crate) async fn acknowledge_presentation(
        &self,
        turn: ConversationTurnId,
        channel: ConversationChannelId,
    ) -> Result<(), AppError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let complete:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_turns t JOIN conversations c ON c.id=t.conversation_id WHERE t.id=?1 AND c.channel_id=?2 AND t.role='assistant' AND t.is_complete=1)")
            .bind(turn).bind(channel).fetch_one(&mut *transaction).await?;
        if !complete {
            return Err(AppError::NotFound(
                "complete response was not found in this channel".into(),
            ));
        }
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        sqlx::query("INSERT OR IGNORE INTO conversation_presentations (turn_id,channel_id,presented_at) VALUES (?1,?2,?3)")
            .bind(turn).bind(channel).bind(&now).execute(&mut *transaction).await?;
        sqlx::query("UPDATE agent_jobs SET delivery_status='presented' WHERE delivery_id=?1 AND channel_id=?2 AND delivery_status IN ('delivering','committed')")
            .bind(turn).bind(channel).execute(&mut *transaction).await?;
        sqlx::query("UPDATE owner_input_presentations SET presented_at=COALESCE(presented_at,?3) WHERE turn_id=?1 AND channel_id=?2 AND EXISTS(SELECT 1 FROM owner_inputs WHERE id=input_id AND status='pending' AND expires_at>?3)")
            .bind(turn).bind(channel).bind(now).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn unpresented_updates(
        &self,
        channel: ConversationChannelId,
    ) -> Result<Vec<ConversationTurn>, AppError> {
        Ok(sqlx::query_as("SELECT t.* FROM conversation_turns t JOIN conversations c ON c.id=t.conversation_id
            WHERE c.channel_id=?1 AND t.role='assistant' AND t.is_complete=1
            AND (EXISTS(SELECT 1 FROM agent_jobs j WHERE j.delivery_id=t.id AND j.delivery_status='committed')
              OR EXISTS(SELECT 1 FROM owner_input_presentations p JOIN owner_inputs i ON i.id=p.input_id WHERE p.turn_id=t.id AND i.status='pending' AND p.presented_at IS NULL))
            AND NOT EXISTS(SELECT 1 FROM conversation_presentations p WHERE p.turn_id=t.id AND p.channel_id=?1)
            ORDER BY t.created_at,t.rowid LIMIT 100")
            .bind(channel).fetch_all(&self.pool).await?)
    }

    pub(crate) async fn recall(
        &self,
        params: crate::provider::history::HistoryRecallParams,
    ) -> Result<Vec<crate::provider::history::HistoryMatch>, AppError> {
        let query = params.query.as_deref().unwrap_or("").trim();
        let limit = params.limit.unwrap_or(12);
        if query.len() > 256 || !(1..=30).contains(&limit) {
            return Err(AppError::Validation(
                "History recall accepts a query of at most 256 bytes and a limit of 1-30".into(),
            ));
        }
        let channel = params
            .channel_id
            .as_deref()
            .map(str::parse::<ConversationChannelId>)
            .transpose()
            .map_err(|error| AppError::Validation(format!("invalid history channel: {error}")))?;
        let mut matches = sqlx::query_as::<_, crate::provider::history::HistoryMatch>(
            "SELECT c.channel_id,t.conversation_id,t.id AS turn_id,t.role,
             substr(t.content,max(1,instr(lower(t.content),lower(?1))-200),2000) AS content,t.is_complete,t.created_at
             FROM conversation_turns t JOIN conversations c ON c.id=t.conversation_id
             WHERE (?1='' OR instr(lower(t.content),lower(?1))>0) AND (?3 IS NULL OR c.channel_id=?3)
             ORDER BY t.created_at DESC,t.id DESC LIMIT ?2",
        ).bind(query).bind(limit).bind(channel).fetch_all(&self.pool).await?;
        let endpoints: Vec<(String, String)> = sqlx::query_as("SELECT id,label FROM endpoints")
            .fetch_all(&self.pool)
            .await?;
        for matched in &mut matches {
            matched.channel_label = if matched.channel_id == ConversationChannelId::OWNER {
                Some("Owner channel".into())
            } else {
                endpoints
                    .iter()
                    .find(|(id, _)| ConversationChannelId::for_endpoint(id) == matched.channel_id)
                    .map(|(_, label)| label.clone())
            };
        }
        Ok(matches)
    }

    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn save(&self, conversation: &Conversation) -> Result<Conversation, AppError> {
        sqlx::query(
            r"INSERT INTO conversations (id, state, closed_at, created_at, updated_at, channel_id)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)
               ON CONFLICT(id) DO UPDATE SET
                   state = excluded.state,
                   closed_at = excluded.closed_at,
                   updated_at = excluded.updated_at",
        )
        .bind(conversation.id)
        .bind(conversation.state)
        .bind(conversation.closed_at)
        .bind(conversation.created_at)
        .bind(conversation.updated_at)
        .bind(conversation.channel_id)
        .execute(&self.pool)
        .await?;

        debug!(conversation_id = %conversation.id, state = %conversation.state, "conversation_repo.saved");
        Ok(conversation.clone())
    }

    pub(crate) async fn get_by_id(
        &self,
        id: ConversationId,
    ) -> Result<Option<Conversation>, AppError> {
        Ok(
            sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id = ?1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub(crate) async fn get_pending_or_active(&self) -> Result<Option<Conversation>, AppError> {
        self.get_open_in_channel(None).await
    }

    pub(crate) async fn get_open_in_channel(
        &self,
        channel: Option<ConversationChannelId>,
    ) -> Result<Option<Conversation>, AppError> {
        Ok(sqlx::query_as::<_, Conversation>(
            "SELECT * FROM conversations WHERE state IN (?1, ?2) AND (?3 IS NULL OR channel_id=?3) ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(ConversationState::Pending.as_str())
        .bind(ConversationState::Active.as_str())
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub(crate) async fn get_last_closed_with_turns(
        &self,
        channel: Option<ConversationChannelId>,
    ) -> Result<Option<Conversation>, AppError> {
        Ok(sqlx::query_as::<_, Conversation>(
            "SELECT c.* FROM conversations c
             WHERE c.state = ?1
               AND (?2 IS NULL OR c.channel_id=?2)
               AND EXISTS (
                   SELECT 1 FROM conversation_turns t
                   WHERE t.conversation_id = c.id
               )
             ORDER BY c.closed_at DESC
             LIMIT 1",
        )
        .bind(ConversationState::Closed.as_str())
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub(crate) async fn delete_if_empty_pending(
        &self,
        conversation_id: ConversationId,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            "DELETE FROM conversations
             WHERE id = ?1
               AND state = ?2
               AND NOT EXISTS (
                   SELECT 1 FROM conversation_turns
                   WHERE conversation_id = ?1
               )",
        )
        .bind(conversation_id)
        .bind(ConversationState::Pending.as_str())
        .execute(&self.pool)
        .await?;

        let deleted = result.rows_affected() > 0;
        if deleted {
            debug!(
                conversation_id = %conversation_id,
                "conversation_repo.empty_pending_deleted"
            );
        }
        Ok(deleted)
    }

    pub(crate) async fn update_state(
        &self,
        id: ConversationId,
        state: ConversationState,
    ) -> Result<Option<Conversation>, AppError> {
        let now = Utc::now();
        let closed_at = if state == ConversationState::Closed {
            Some(now)
        } else {
            None
        };

        let result = sqlx::query(
            r"UPDATE conversations SET state = ?1,
               closed_at = COALESCE(?2, closed_at), updated_at = ?3 WHERE id = ?4",
        )
        .bind(state.as_str())
        .bind(closed_at)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            warn!(conversation_id = %id, "conversation_repo.update_state.not_found");
            return Ok(None);
        }

        info!(
            conversation_id = %id,
            to_state = %state.as_str(),
            "conversation_repo.updated_state"
        );
        self.get_by_id(id).await
    }

    pub(crate) async fn add_turn(
        &self,
        turn: &ConversationTurn,
    ) -> Result<ConversationTurn, AppError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let conv_state: Option<(String,)> =
            sqlx::query_as("SELECT state FROM conversations WHERE id = ?1")
                .bind(turn.conversation_id)
                .fetch_optional(&mut *tx)
                .await?;

        match conv_state {
            None => {
                warn!(conversation_id = %turn.conversation_id, "conversation_repo.add_turn_not_found");
                return Err(AppError::NotFound(format!(
                    "Conversation {} not found",
                    turn.conversation_id
                )));
            }
            Some((state,)) if state == ConversationState::Closed.as_str() => {
                warn!(conversation_id = %turn.conversation_id, role = %turn.role, "conversation_repo.add_turn_closed");
                return Err(AppError::Validation(format!(
                    "Cannot add turn to closed conversation {}",
                    turn.conversation_id
                )));
            }
            _ => {}
        }

        sqlx::query(
            r"INSERT INTO conversation_turns
                  (id, role, content, is_complete, conversation_id, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(turn.id)
        .bind(turn.role)
        .bind(&turn.content)
        .bind(turn.is_complete)
        .bind(turn.conversation_id)
        .bind(turn.created_at)
        .bind(turn.updated_at)
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE conversations SET updated_at = ?1 WHERE id = ?2")
            .bind(Utc::now())
            .bind(turn.conversation_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        debug!(
            conversation_id = %turn.conversation_id,
            turn_id = %turn.id,
            role = %turn.role,
            content_length = turn.content.len(),
            "conversation_repo.add_turn"
        );
        Ok(turn.clone())
    }

    pub(crate) async fn update_turn_content_and_completion(
        &self,
        turn_id: ConversationTurnId,
        content: &str,
        is_complete: bool,
    ) -> Result<Option<ConversationTurn>, AppError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let now = Utc::now();

        let conversation_id: Option<(ConversationId,)> =
            sqlx::query_as("SELECT conversation_id FROM conversation_turns WHERE id = ?1")
                .bind(turn_id)
                .fetch_optional(&mut *tx)
                .await?;

        let Some((conversation_id,)) = conversation_id else {
            warn!(turn_id = %turn_id, "conversation_repo.update_turn_content.not_found");
            return Ok(None);
        };

        sqlx::query(
            r"UPDATE conversation_turns
              SET content = ?1, is_complete = ?2, updated_at = ?3
              WHERE id = ?4",
        )
        .bind(content)
        .bind(is_complete)
        .bind(now)
        .bind(turn_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE conversations SET updated_at = ?1 WHERE id = ?2")
            .bind(now)
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(
            sqlx::query_as::<_, ConversationTurn>("SELECT * FROM conversation_turns WHERE id = ?1")
                .bind(turn_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    #[cfg(test)]
    pub(crate) async fn get_turns(
        &self,
        conversation_id: ConversationId,
        limit: i64,
    ) -> Result<Vec<ConversationTurn>, AppError> {
        Ok(sqlx::query_as::<_, ConversationTurn>(
            "SELECT * FROM conversation_turns WHERE conversation_id = ?1 ORDER BY created_at ASC LIMIT ?2",
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub(crate) async fn get_recent_turns(
        &self,
        conversation_id: ConversationId,
        limit: i64,
    ) -> Result<Vec<ConversationTurn>, AppError> {
        Ok(sqlx::query_as::<_, ConversationTurn>(
            "SELECT * FROM (
                SELECT * FROM conversation_turns
                WHERE conversation_id = ?1
                ORDER BY created_at DESC, id DESC
                LIMIT ?2
             )
             ORDER BY created_at ASC, id ASC",
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub(crate) async fn last_user_turn_at(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Option<chrono::DateTime<Utc>>, AppError> {
        let row: Option<(Option<chrono::DateTime<Utc>>,)> = sqlx::query_as(
            "SELECT MAX(created_at) FROM conversation_turns WHERE conversation_id = ?1 AND role = ?2",
        )
        .bind(conversation_id)
        .bind(TurnRole::User.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(ts,)| ts))
    }

    pub(crate) async fn latest_user_turn_at(
        &self,
    ) -> Result<Option<chrono::DateTime<Utc>>, AppError> {
        let row: Option<(Option<chrono::DateTime<Utc>>,)> =
            sqlx::query_as("SELECT MAX(created_at) FROM conversation_turns WHERE role = ?1")
                .bind(TurnRole::User.as_str())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(ts,)| ts))
    }

    pub(crate) async fn count_turns(
        &self,
        conversation_id: ConversationId,
    ) -> Result<i64, AppError> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversation_turns WHERE conversation_id = ?1")
                .bind(conversation_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    pub(crate) async fn list_open(&self) -> Result<Vec<Conversation>, AppError> {
        Ok(sqlx::query_as("SELECT * FROM conversations WHERE state IN ('active','pending') ORDER BY updated_at,id")
            .fetch_all(&self.pool).await?)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on precondition failures")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn conversation_writes_wait_for_other_durable_work() {
        let root =
            std::env::temp_dir().join(format!("bobe-conversation-writer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create database directory");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .min_connections(2)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(root.join("state.sqlite3"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .busy_timeout(std::time::Duration::from_secs(2)),
            )
            .await
            .expect("open disk-backed pool");
        sqlx::raw_sql(include_str!("../../migrations/schema.sql"))
            .execute(&pool)
            .await
            .expect("initialize schema");
        let repo = SqliteConversationRepo::new(pool.clone());
        let conversation = repo
            .save(&Conversation::new_active())
            .await
            .expect("save conversation");
        let mut outcomes = Vec::new();
        for update in [false, true] {
            let turn = ConversationTurn::new(conversation.id, TurnRole::Assistant, "draft".into());
            if update {
                repo.add_turn(&turn).await.expect("save draft turn");
            }
            let writer = pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .expect("reserve competing writer");
            let other = SqliteConversationRepo::new(pool.clone());
            let (started, start) = tokio::sync::oneshot::channel();
            let mutation = tokio::spawn(async move {
                started.send(()).expect("notify mutation started");
                if update {
                    other
                        .update_turn_content_and_completion(turn.id, "complete", true)
                        .await
                        .map(|turn| turn.expect("updated turn exists"))
                } else {
                    other.add_turn(&turn).await
                }
            });
            start.await.expect("mutation started");
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            let waited = !mutation.is_finished();
            writer.commit().await.expect("release competing writer");
            outcomes.push((waited, mutation.await.expect("join mutation")));
        }
        pool.close().await;
        std::fs::remove_dir_all(root).expect("remove database directory");
        for (waited, result) in outcomes {
            assert!(
                waited,
                "conversation mutation must wait for the existing writer instead of attempting a snapshot upgrade"
            );
            result.expect("conversation mutation succeeds after writer commits");
        }
    }

    #[tokio::test]
    async fn latest_user_turn_queries_use_covering_indexes() {
        let pool = in_memory_pool().await;
        let global_plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN SELECT MAX(created_at) FROM conversation_turns WHERE role = ?1",
        )
        .bind(TurnRole::User.as_str())
        .fetch_all(&pool)
        .await
        .expect("global query plan");
        assert!(
            global_plan
                .iter()
                .any(|row| { row.3.contains("ix_conversation_turns_role_created_at") })
        );

        let conversation_plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN SELECT MAX(created_at) FROM conversation_turns WHERE conversation_id = ?1 AND role = ?2",
        )
        .bind(ConversationId::new())
        .bind(TurnRole::User.as_str())
        .fetch_all(&pool)
        .await
        .expect("conversation query plan");
        assert!(conversation_plan.iter().any(|row| {
            row.3
                .contains("ix_conversation_turns_conversation_role_created_at")
        }));
    }
    use crate::db::test_helpers::in_memory_pool;

    #[tokio::test]
    async fn save_then_get_by_id_roundtrip() {
        let pool = in_memory_pool().await;
        let repo = SqliteConversationRepo::new(pool);
        let conv = Conversation::new_pending();
        let saved = repo.save(&conv).await.expect("save");
        let fetched = repo
            .get_by_id(saved.id)
            .await
            .expect("get_by_id")
            .expect("conversation exists");
        assert_eq!(fetched.id, saved.id);
        assert_eq!(fetched.state, ConversationState::Pending);
    }

    #[tokio::test]
    async fn pending_lookup_finds_pending() {
        let pool = in_memory_pool().await;
        let repo = SqliteConversationRepo::new(pool);
        let pending = repo
            .save(&Conversation::new_pending())
            .await
            .expect("save pending");
        let found = repo
            .get_pending_or_active()
            .await
            .expect("get_pending_or_active")
            .expect("found pending");
        assert_eq!(found.id, pending.id);
        assert_eq!(found.state, ConversationState::Pending);
    }

    #[tokio::test]
    async fn update_state_transitions_pending_to_active() {
        let pool = in_memory_pool().await;
        let repo = SqliteConversationRepo::new(pool);
        let conv = repo.save(&Conversation::new_pending()).await.expect("save");
        let updated = repo
            .update_state(conv.id, ConversationState::Active)
            .await
            .expect("update_state")
            .expect("conversation found");
        assert_eq!(updated.state, ConversationState::Active);
    }

    #[tokio::test]
    async fn closed_lookup_returns_most_recent_closed_with_turns() {
        let pool = in_memory_pool().await;
        let repo = SqliteConversationRepo::new(pool);
        let c = repo.save(&Conversation::new_active()).await.expect("save");
        let turn = ConversationTurn::new(c.id, TurnRole::Assistant, "answer".into());
        repo.add_turn(&turn).await.expect("add turn");
        repo.update_state(c.id, ConversationState::Closed)
            .await
            .expect("close");
        let empty = repo
            .save(&Conversation::new_active())
            .await
            .expect("save empty conversation");
        repo.update_state(empty.id, ConversationState::Closed)
            .await
            .expect("close empty conversation");
        let last = repo
            .get_last_closed_with_turns(None)
            .await
            .expect("get_last_closed_with_turns")
            .expect("has closed conversation");
        assert_eq!(last.id, c.id);
    }

    #[tokio::test]
    async fn add_turn_and_get_turns_in_order() {
        let pool = in_memory_pool().await;
        let repo = SqliteConversationRepo::new(pool);
        let conv = repo.save(&Conversation::new_active()).await.expect("save");
        let t1 = ConversationTurn::new(conv.id, TurnRole::User, "hello".into());
        let t2 = ConversationTurn::new(conv.id, TurnRole::Assistant, "world".into());
        repo.add_turn(&t1).await.expect("add turn 1");
        repo.add_turn(&t2).await.expect("add turn 2");
        let turns = repo.get_turns(conv.id, 10).await.expect("get_turns");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "hello");
        assert_eq!(turns[1].content, "world");
    }

    #[tokio::test]
    async fn recent_turns_returns_newest_window_in_chronological_order() {
        let pool = in_memory_pool().await;
        let repo = SqliteConversationRepo::new(pool);
        let conv = repo.save(&Conversation::new_active()).await.expect("save");
        let base = Utc::now();
        for index in 0..5 {
            let mut turn = ConversationTurn::new(conv.id, TurnRole::User, format!("turn-{index}"));
            turn.created_at = base + chrono::Duration::seconds(index);
            turn.updated_at = turn.created_at;
            repo.add_turn(&turn).await.expect("add turn");
        }

        let turns = repo
            .get_recent_turns(conv.id, 3)
            .await
            .expect("get recent turns");
        let contents = turns
            .into_iter()
            .map(|turn| turn.content)
            .collect::<Vec<_>>();
        assert_eq!(contents, ["turn-2", "turn-3", "turn-4"]);
    }

    #[tokio::test]
    async fn update_turn_content_and_completion_overwrites_both() {
        let pool = in_memory_pool().await;
        let repo = SqliteConversationRepo::new(pool);
        let conv = repo.save(&Conversation::new_active()).await.expect("save");
        let turn = ConversationTurn::new(conv.id, TurnRole::Assistant, "draft".into());
        repo.add_turn(&turn).await.expect("add turn");
        let updated = repo
            .update_turn_content_and_completion(turn.id, "interrupted", false)
            .await
            .expect("update turn")
            .expect("turn exists");
        assert_eq!(updated.content, "interrupted");
        assert!(!updated.is_complete);
    }
}
