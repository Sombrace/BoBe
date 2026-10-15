use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::db::SqliteConversationRepo;
use crate::error::AppError;
use crate::models::conversation::{Conversation, ConversationTurn};
use crate::models::ids::{ConversationChannelId, ConversationId, ConversationTurnId};
use crate::models::types::{ConversationState, TurnRole};

#[derive(Debug, Clone)]
struct StreamingAssistantTurn {
    turn: ConversationTurn,
    persisted: bool,
}

pub(crate) struct ConversationService {
    repo: Arc<SqliteConversationRepo>,
    lifecycle_lock: Mutex<()>,
    streaming_assistant_turns: DashMap<ConversationId, StreamingAssistantTurn>,
}

impl ConversationService {
    pub(crate) async fn acknowledge_presentation(
        &self,
        turn: ConversationTurnId,
        channel: ConversationChannelId,
    ) -> Result<(), AppError> {
        self.repo.acknowledge_presentation(turn, channel).await
    }

    pub(crate) async fn unpresented_updates(
        &self,
        channel: ConversationChannelId,
    ) -> Result<Vec<ConversationTurn>, AppError> {
        self.repo.unpresented_updates(channel).await
    }

    pub(crate) fn new(repo: Arc<SqliteConversationRepo>) -> Self {
        Self {
            repo,
            lifecycle_lock: Mutex::new(()),
            streaming_assistant_turns: DashMap::new(),
        }
    }

    pub(crate) async fn append_user_turn_or_create_active(
        &self,
        user_message: &str,
        channel_id: ConversationChannelId,
    ) -> Result<Conversation, AppError> {
        let _guard = self.lifecycle_lock.lock().await;
        if let Some(conversation) = self.repo.get_open_in_channel(Some(channel_id)).await? {
            self.sync_streaming_assistant_turn_locked(conversation.id)
                .await?;
            let conversation = if conversation.is_pending() {
                self.repo
                    .update_state(conversation.id, ConversationState::Active)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!(
                            "Conversation {} not found while activating pending conversation",
                            conversation.id
                        ))
                    })?
            } else {
                conversation
            };

            let turn =
                ConversationTurn::new(conversation.id, TurnRole::User, user_message.to_owned());
            self.repo.add_turn(&turn).await?;
            info!(
                conversation_id = %conversation.id,
                state = %conversation.state,
                "conversation.user_turn_appended"
            );
            return Ok(conversation);
        }

        self.create_active_unlocked(user_message, channel_id).await
    }

    pub(crate) async fn begin_proactive_stream(
        &self,
        preferred_conversation: Option<&Conversation>,
    ) -> Result<(Conversation, ConversationTurnId), AppError> {
        let _guard = self.lifecycle_lock.lock().await;

        let conversation = if let Some(preferred) = preferred_conversation {
            match self.repo.get_by_id(preferred.id).await? {
                Some(current) if !current.is_closed() => current,
                _ => {
                    self.open_or_create_pending_locked(preferred.channel_id)
                        .await?
                }
            }
        } else {
            self.open_or_create_pending_locked(ConversationChannelId::OWNER)
                .await?
        };

        let turn_id = ConversationTurnId::new();
        self.begin_assistant_stream_locked(conversation.id, turn_id);
        Ok((conversation, turn_id))
    }

    pub(crate) async fn begin_assistant_stream(
        &self,
        conversation_id: ConversationId,
        turn_id: ConversationTurnId,
    ) -> Result<(), AppError> {
        let _guard = self.lifecycle_lock.lock().await;
        self.sync_streaming_assistant_turn_locked(conversation_id)
            .await?;
        self.begin_assistant_stream_locked(conversation_id, turn_id);
        Ok(())
    }

    pub(crate) async fn event_conversation(
        &self,
        channel: ConversationChannelId,
    ) -> Result<Conversation, AppError> {
        let _guard = self.lifecycle_lock.lock().await;
        self.open_or_create_pending_locked(channel).await
    }

    fn begin_assistant_stream_locked(
        &self,
        conversation_id: ConversationId,
        turn_id: ConversationTurnId,
    ) {
        let turn = ConversationTurn::new_with_completion(
            turn_id,
            conversation_id,
            TurnRole::Assistant,
            String::new(),
            false,
        );
        let replaced = self.streaming_assistant_turns.insert(
            conversation_id,
            StreamingAssistantTurn {
                turn,
                persisted: false,
            },
        );
        if replaced.is_some() {
            warn!(
                %conversation_id,
                "conversation.streaming_turn_replaced"
            );
        }
    }

    pub(crate) fn push_assistant_stream_delta(&self, conversation_id: ConversationId, delta: &str) {
        if delta.is_empty() {
            return;
        }

        if let Some(mut draft) = self.streaming_assistant_turns.get_mut(&conversation_id) {
            draft.turn.append_content(delta);
        }
    }

    pub(crate) async fn finalize_assistant_stream(
        &self,
        conversation_id: ConversationId,
        final_content: &str,
        is_complete: bool,
    ) -> Result<Option<ConversationTurn>, AppError> {
        let _guard = self.lifecycle_lock.lock().await;
        let Some((_, mut draft)) = self.streaming_assistant_turns.remove(&conversation_id) else {
            return Ok(None);
        };

        draft.turn.replace_content(final_content.to_owned());
        draft.turn.set_complete(is_complete);
        if draft.turn.content.is_empty() {
            return Ok(None);
        }

        if draft.persisted {
            self.repo
                .update_turn_content_and_completion(
                    draft.turn.id,
                    &draft.turn.content,
                    draft.turn.is_complete,
                )
                .await
        } else {
            self.repo.add_turn(&draft.turn).await.map(Some)
        }
    }

    pub(crate) async fn discard_proactive_stream(
        &self,
        conversation_id: ConversationId,
    ) -> Result<(), AppError> {
        let _guard = self.lifecycle_lock.lock().await;
        self.streaming_assistant_turns.remove(&conversation_id);
        self.repo.delete_if_empty_pending(conversation_id).await?;
        Ok(())
    }

    pub(crate) async fn settle_interrupted_turn(
        &self,
        turn_id: ConversationTurnId,
    ) -> Result<(), AppError> {
        let _guard = self.lifecycle_lock.lock().await;
        let conversation_id = self
            .streaming_assistant_turns
            .iter()
            .find(|entry| entry.turn.id == turn_id)
            .map(|entry| *entry.key());
        if let Some(conversation_id) = conversation_id {
            self.settle_interrupted_stream_locked(conversation_id)
                .await?;
        }
        Ok(())
    }

    async fn settle_interrupted_stream_locked(
        &self,
        conversation_id: ConversationId,
    ) -> Result<(), AppError> {
        let Some((_, mut draft)) = self.streaming_assistant_turns.remove(&conversation_id) else {
            return Ok(());
        };
        if draft.turn.content.is_empty() {
            self.repo.delete_if_empty_pending(conversation_id).await?;
            return Ok(());
        }
        draft.turn.set_complete(false);
        if draft.persisted {
            self.repo
                .update_turn_content_and_completion(draft.turn.id, &draft.turn.content, false)
                .await?;
        } else {
            self.repo.add_turn(&draft.turn).await?;
        }
        Ok(())
    }

    async fn open_or_create_pending_locked(
        &self,
        channel: ConversationChannelId,
    ) -> Result<Conversation, AppError> {
        if let Some(conversation) = self.repo.get_open_in_channel(Some(channel)).await? {
            return Ok(conversation);
        }

        let conversation = Conversation::new_pending().in_channel(channel);
        self.repo.save(&conversation).await
    }

    async fn sync_streaming_assistant_turn_locked(
        &self,
        conversation_id: ConversationId,
    ) -> Result<(), AppError> {
        let Some(snapshot) = self
            .streaming_assistant_turns
            .get(&conversation_id)
            .map(|draft| (draft.turn.clone(), draft.persisted))
        else {
            return Ok(());
        };

        let (turn, persisted) = snapshot;
        if turn.content.is_empty() {
            return Ok(());
        }

        if persisted {
            self.repo
                .update_turn_content_and_completion(turn.id, &turn.content, turn.is_complete)
                .await?;
        } else {
            self.repo.add_turn(&turn).await?;
            if let Some(mut draft) = self.streaming_assistant_turns.get_mut(&conversation_id)
                && draft.turn.id == turn.id
            {
                draft.persisted = true;
            }
        }

        Ok(())
    }

    async fn create_active_unlocked(
        &self,
        user_message: &str,
        channel: ConversationChannelId,
    ) -> Result<Conversation, AppError> {
        let conversation = Conversation::new_active().in_channel(channel);
        let saved = self.repo.save(&conversation).await?;

        let turn = ConversationTurn::new(saved.id, TurnRole::User, user_message.to_owned());
        self.repo.add_turn(&turn).await?;

        info!(
            conversation_id = %saved.id,
            state = %saved.state,
            "conversation.created_active"
        );
        Ok(saved)
    }

    pub(crate) async fn close_if_stale(
        &self,
        conversation_id: ConversationId,
        auto_close_minutes: i64,
    ) -> Result<Option<usize>, AppError> {
        let _guard = self.lifecycle_lock.lock().await;

        if self
            .streaming_assistant_turns
            .contains_key(&conversation_id)
        {
            return Ok(None);
        }

        let Some(conversation) = self.repo.get_by_id(conversation_id).await? else {
            return Ok(None);
        };

        let last_user_at = self.repo.last_user_turn_at(conversation_id).await?;
        if !conversation.is_stale_since(auto_close_minutes, last_user_at) {
            return Ok(None);
        }

        let updated = self
            .repo
            .update_state(conversation_id, ConversationState::Closed)
            .await?;
        if updated.is_some() {
            let turn_count = self.repo.count_turns(conversation_id).await?;
            info!(conversation_id = %conversation_id, "conversation.closed");
            return Ok(Some(usize::try_from(turn_count).unwrap_or(0)));
        }

        Ok(None)
    }

    pub(crate) async fn last_user_turn_at(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        self.repo.last_user_turn_at(conversation_id).await
    }

    pub(crate) async fn latest_user_turn_at(
        &self,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        self.repo.latest_user_turn_at().await
    }

    pub(crate) async fn get_latest_readable_conversation(
        &self,
        limit: i64,
        channel: Option<ConversationChannelId>,
    ) -> Result<Option<(Conversation, Vec<ConversationTurn>)>, AppError> {
        if let Some(open) = self.repo.get_open_in_channel(channel).await? {
            let turns = self.repo.get_recent_turns(open.id, limit).await?;
            if !turns.is_empty() {
                return Ok(Some((open, turns)));
            }

            if let Some(closed) = self.repo.get_last_closed_with_turns(channel).await? {
                let turns = self.repo.get_recent_turns(closed.id, limit).await?;
                return Ok(Some((closed, turns)));
            }

            return Ok(Some((open, turns)));
        }

        let Some(closed) = self.repo.get_last_closed_with_turns(channel).await? else {
            return Ok(None);
        };
        let turns = self.repo.get_recent_turns(closed.id, limit).await?;
        Ok(Some((closed, turns)))
    }

    pub(crate) async fn get_pending_or_active(&self) -> Result<Option<Conversation>, AppError> {
        self.repo.get_pending_or_active().await
    }

    pub(crate) async fn list_open(&self) -> Result<Vec<Conversation>, AppError> {
        self.repo.list_open().await
    }

    #[cfg(test)]
    pub(crate) async fn get_conversation_turns(
        &self,
        conversation_id: ConversationId,
        limit: i64,
    ) -> Result<Vec<ConversationTurn>, AppError> {
        self.repo.get_turns(conversation_id, limit).await
    }
}

impl std::fmt::Debug for ConversationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationService").finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::db::test_helpers::in_memory_pool;

    #[tokio::test]
    async fn a_closed_proactive_target_cannot_fall_back_to_another_channel() {
        let repo = Arc::new(SqliteConversationRepo::new(in_memory_pool().await));
        let service = ConversationService::new(Arc::clone(&repo));
        let first = ConversationChannelId::for_endpoint("first");
        let other = ConversationChannelId::for_endpoint("other");
        let preferred = service.event_conversation(first).await.unwrap();
        repo.update_state(preferred.id, ConversationState::Closed)
            .await
            .unwrap();
        service.event_conversation(other).await.unwrap();
        let (target, _) = service
            .begin_proactive_stream(Some(&preferred))
            .await
            .unwrap();
        assert_eq!(target.channel_id, first);
        assert_ne!(target.id, preferred.id);
    }

    #[tokio::test]
    async fn channel_history_is_separate_and_recall_is_explicit() {
        let pool = in_memory_pool().await;
        let repo = Arc::new(SqliteConversationRepo::new(pool.clone()));
        let service = ConversationService::new(Arc::clone(&repo));
        let mac = ConversationChannelId::for_endpoint("mac");
        let body = ConversationChannelId::for_endpoint("body");
        let first = service
            .append_user_turn_or_create_active("Mac-only garden note", mac)
            .await
            .unwrap();
        let second = service
            .append_user_turn_or_create_active("Body-only cooking question", body)
            .await
            .unwrap();
        assert_ne!(first.id, second.id);
        let (visible, turns) = service
            .get_latest_readable_conversation(10, Some(body))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(visible.channel_id, body);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "Body-only cooking question");
        let recalled = repo
            .recall(crate::provider::history::HistoryRecallParams {
                query: Some("garden".into()),
                channel_id: None,
                limit: Some(3),
            })
            .await
            .unwrap();
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].channel_id, mac);
        assert_eq!(recalled[0].content, "Mac-only garden note");
        let recent = repo
            .recall(crate::provider::history::HistoryRecallParams {
                query: None,
                channel_id: Some(body.to_string()),
                limit: Some(3),
            })
            .await
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].channel_id, body);
        pool.close().await;
    }

    #[tokio::test]
    async fn latest_readable_conversation_falls_back_from_empty_pending_to_closed() {
        let repo = Arc::new(SqliteConversationRepo::new(in_memory_pool().await));
        let closed = repo
            .save(&Conversation::new_active())
            .await
            .expect("save active conversation");
        let answer =
            ConversationTurn::new(closed.id, TurnRole::Assistant, "Durable answer".to_owned());
        repo.add_turn(&answer).await.expect("save answer");
        repo.update_state(closed.id, ConversationState::Closed)
            .await
            .expect("close conversation");
        let empty_closed = repo
            .save(&Conversation::new_active())
            .await
            .expect("save empty active conversation");
        repo.update_state(empty_closed.id, ConversationState::Closed)
            .await
            .expect("close empty conversation");
        repo.save(&Conversation::new_pending())
            .await
            .expect("save empty pending conversation");
        let service = ConversationService::new(repo);

        let (conversation, turns) = service
            .get_latest_readable_conversation(200, None)
            .await
            .expect("load latest readable conversation")
            .expect("conversation exists");

        assert_eq!(conversation.id, closed.id);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "Durable answer");
    }

    #[tokio::test]
    async fn discarding_proactive_stream_removes_empty_pending_placeholder() {
        let repo = Arc::new(SqliteConversationRepo::new(in_memory_pool().await));
        let service = ConversationService::new(Arc::clone(&repo));
        let (pending, _) = service
            .begin_proactive_stream(None)
            .await
            .expect("begin proactive stream");

        service
            .discard_proactive_stream(pending.id)
            .await
            .expect("discard proactive stream");

        assert!(
            repo.get_by_id(pending.id)
                .await
                .expect("load pending conversation")
                .is_none()
        );
    }

    #[tokio::test]
    async fn discarding_proactive_stream_preserves_populated_conversation() {
        let repo = Arc::new(SqliteConversationRepo::new(in_memory_pool().await));
        let pending = repo
            .save(&Conversation::new_pending())
            .await
            .expect("save pending conversation");
        let user_turn = ConversationTurn::new(pending.id, TurnRole::User, "Hello".to_owned());
        repo.add_turn(&user_turn).await.expect("save user turn");
        let service = ConversationService::new(Arc::clone(&repo));
        service
            .begin_proactive_stream(Some(&pending))
            .await
            .expect("begin proactive stream");

        service
            .discard_proactive_stream(pending.id)
            .await
            .expect("discard proactive stream");

        assert!(
            repo.get_by_id(pending.id)
                .await
                .expect("load pending conversation")
                .is_some()
        );
    }

    #[tokio::test]
    async fn cancelled_proactive_draft_is_persisted_as_interrupted() {
        let repo = Arc::new(SqliteConversationRepo::new(in_memory_pool().await));
        let service = ConversationService::new(Arc::clone(&repo));
        let (pending, turn_id) = service
            .begin_proactive_stream(None)
            .await
            .expect("begin proactive stream");
        service.push_assistant_stream_delta(pending.id, "cancelled partial");

        service
            .settle_interrupted_turn(turn_id)
            .await
            .expect("settle interrupted proactive stream");
        service
            .append_user_turn_or_create_active("real user turn", ConversationChannelId::OWNER)
            .await
            .expect("append user turn");

        let turns = repo
            .get_turns(pending.id, 10)
            .await
            .expect("load conversation turns");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, TurnRole::Assistant);
        assert_eq!(turns[0].content, "cancelled partial");
        assert!(!turns[0].is_complete);
        assert_eq!(turns[1].role, TurnRole::User);
        assert_eq!(turns[1].content, "real user turn");
    }

    #[tokio::test]
    async fn persisted_proactive_draft_stays_incomplete_until_finalized() {
        let repo = Arc::new(SqliteConversationRepo::new(in_memory_pool().await));
        let service = ConversationService::new(Arc::clone(&repo));
        let (pending, _) = service
            .begin_proactive_stream(None)
            .await
            .expect("begin proactive stream");
        service.push_assistant_stream_delta(pending.id, "partial");

        service
            .sync_streaming_assistant_turn_locked(pending.id)
            .await
            .expect("persist draft");
        let draft = repo
            .get_turns(pending.id, 10)
            .await
            .expect("load draft")
            .pop()
            .expect("draft exists");
        assert!(!draft.is_complete);

        let finalized = service
            .finalize_assistant_stream(pending.id, "complete answer", true)
            .await
            .expect("finalize")
            .expect("turn exists");
        assert!(finalized.is_complete);
        assert_eq!(finalized.content, "complete answer");
    }

    #[tokio::test]
    async fn latest_readable_conversation_prefers_populated_open_conversation() {
        let repo = Arc::new(SqliteConversationRepo::new(in_memory_pool().await));
        let closed = repo
            .save(&Conversation::new_active())
            .await
            .expect("save old conversation");
        repo.update_state(closed.id, ConversationState::Closed)
            .await
            .expect("close old conversation");

        let open = repo
            .save(&Conversation::new_active())
            .await
            .expect("save open conversation");
        let user_turn = ConversationTurn::new(open.id, TurnRole::User, "Current".to_owned());
        repo.add_turn(&user_turn).await.expect("save current turn");
        let service = ConversationService::new(repo);

        let (conversation, turns) = service
            .get_latest_readable_conversation(200, None)
            .await
            .expect("load latest readable conversation")
            .expect("conversation exists");

        assert_eq!(conversation.id, open.id);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "Current");
    }
}
