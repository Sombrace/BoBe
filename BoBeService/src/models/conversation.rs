use chrono::{DateTime, Utc};

use super::ids::{ConversationChannelId, ConversationId, ConversationTurnId};
use super::types::{ConversationState, TurnRole};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(crate) struct Conversation {
    pub(crate) id: ConversationId,
    pub(crate) channel_id: ConversationChannelId,
    pub(crate) state: ConversationState,
    pub(crate) closed_at: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl Conversation {
    pub(crate) fn new_pending() -> Self {
        let now = Utc::now();
        Self {
            id: ConversationId::new(),
            channel_id: ConversationChannelId::OWNER,
            state: ConversationState::Pending,
            closed_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub(crate) fn new_active() -> Self {
        let now = Utc::now();
        Self {
            id: ConversationId::new(),
            channel_id: ConversationChannelId::OWNER,
            state: ConversationState::Active,
            closed_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.state == ConversationState::Pending
    }

    pub(crate) fn in_channel(mut self, channel_id: ConversationChannelId) -> Self {
        self.channel_id = channel_id;
        self
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.state == ConversationState::Closed
    }

    pub(crate) fn is_stale_since(
        &self,
        auto_close_minutes: i64,
        last_user_at: Option<DateTime<Utc>>,
    ) -> bool {
        let reference = last_user_at.unwrap_or(self.created_at);
        let elapsed = Utc::now() - reference;
        elapsed
            >= chrono::TimeDelta::try_minutes(auto_close_minutes).unwrap_or(chrono::TimeDelta::MAX)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(crate) struct ConversationTurn {
    pub(crate) id: ConversationTurnId,
    pub(crate) role: TurnRole,
    pub(crate) content: String,
    pub(crate) is_complete: bool,
    pub(crate) conversation_id: ConversationId,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl ConversationTurn {
    pub(crate) fn new(conversation_id: ConversationId, role: TurnRole, content: String) -> Self {
        Self::new_with_id(ConversationTurnId::new(), conversation_id, role, content)
    }

    pub(crate) fn new_with_id(
        id: ConversationTurnId,
        conversation_id: ConversationId,
        role: TurnRole,
        content: String,
    ) -> Self {
        Self::new_with_completion(id, conversation_id, role, content, true)
    }

    pub(crate) fn new_with_completion(
        id: ConversationTurnId,
        conversation_id: ConversationId,
        role: TurnRole,
        content: String,
        is_complete: bool,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            role,
            content,
            is_complete,
            conversation_id,
            created_at: now,
            updated_at: now,
        }
    }

    pub(crate) fn append_content(&mut self, delta: &str) {
        self.content.push_str(delta);
        self.updated_at = Utc::now();
    }

    pub(crate) fn replace_content(&mut self, content: String) {
        self.content = content;
        self.updated_at = Utc::now();
    }

    pub(crate) fn set_complete(&mut self, is_complete: bool) {
        self.is_complete = is_complete;
        self.updated_at = Utc::now();
    }
}
