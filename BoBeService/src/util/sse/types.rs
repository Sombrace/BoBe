use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventAudience {
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventType {
    Indicator,
    TextDelta,
    ToolCallStart,
    ToolCallComplete,
    Error,
    ConversationClosed,
    ConversationChanged,
    SnapshotRequired,
    BodyRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[derive(Default)]
pub(crate) enum IndicatorType {
    #[default]
    Idle,
    Thinking,
    Streaming,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StreamBundle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) channel_id: Option<crate::models::ids::ConversationChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) event_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) runtime_incarnation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) audience: Option<EventAudience>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) turn_id: Option<Uuid>,
    #[serde(rename = "type")]
    pub(crate) event_type: EventType,
    pub(crate) message_id: String,
    pub(crate) payload: serde_json::Value,
}

impl StreamBundle {
    pub(crate) fn now(
        event_type: EventType,
        message_id: String,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            channel_id: None,
            event_id: None,
            runtime_incarnation_id: None,
            audience: None,
            run_id: None,
            turn_id: None,
            event_type,
            message_id,
            payload,
        }
    }
}
