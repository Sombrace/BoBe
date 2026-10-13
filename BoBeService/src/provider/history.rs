use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const HISTORY_RECALL: &str = "bobe_history_recall";

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryRecallParams {
    pub(crate) query: Option<String>,
    pub(crate) channel_id: Option<String>,
    pub(crate) limit: Option<u32>,
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct HistoryMatch {
    pub(crate) channel_id: crate::models::ids::ConversationChannelId,
    #[sqlx(skip)]
    pub(crate) channel_label: Option<String>,
    pub(crate) conversation_id: crate::models::ids::ConversationId,
    pub(crate) turn_id: crate::models::ids::ConversationTurnId,
    pub(crate) role: crate::models::types::TurnRole,
    pub(crate) content: String,
    pub(crate) is_complete: bool,
    pub(crate) created_at: chrono::DateTime<chrono::Utc>,
}
