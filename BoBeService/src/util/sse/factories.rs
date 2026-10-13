use serde_json::json;

use super::types::{EventType, IndicatorType, StreamBundle};

pub(crate) fn indicator_event(indicator: IndicatorType) -> StreamBundle {
    StreamBundle::now(
        EventType::Indicator,
        String::new(),
        json!({ "indicator": indicator }),
    )
}

pub(crate) fn text_delta_event(message_id: &str, delta: &str, done: bool) -> StreamBundle {
    StreamBundle::now(
        EventType::TextDelta,
        message_id.to_owned(),
        json!({
            "delta": delta,
            "done": done,
        }),
    )
}

pub(crate) fn error_event(
    message_id: &str,
    code: &str,
    message: &str,
    recoverable: bool,
) -> StreamBundle {
    StreamBundle::now(
        EventType::Error,
        message_id.to_owned(),
        json!({
            "code": code,
            "message": message,
            "recoverable": recoverable,
        }),
    )
}

pub(crate) fn trigger_error_event(trigger: &str, message: &str, recoverable: bool) -> StreamBundle {
    StreamBundle::now(
        EventType::Error,
        uuid::Uuid::new_v4().to_string(),
        json!({
            "trigger": trigger,
            "message": message,
            "recoverable": recoverable,
        }),
    )
}

pub(crate) fn tool_call_start_event(
    message_id: &str,
    tool_name: &str,
    tool_call_id: &str,
) -> StreamBundle {
    StreamBundle::now(
        EventType::ToolCallStart,
        message_id.to_owned(),
        json!({
            "tool_name": tool_name,
            "tool_call_id": tool_call_id,
            "status": crate::constants::tool_call_status::START,
        }),
    )
}

pub(crate) fn tool_call_complete_event(
    message_id: &str,
    tool_name: &str,
    tool_call_id: &str,
    success: bool,
) -> StreamBundle {
    StreamBundle::now(
        EventType::ToolCallComplete,
        message_id.to_owned(),
        json!({
            "tool_name": tool_name,
            "tool_call_id": tool_call_id,
            "success": success,
            "status": crate::constants::tool_call_status::COMPLETE,
        }),
    )
}

pub(crate) fn end_of_turn_event(message_id: &str, full_response: &str) -> StreamBundle {
    text_delta_event(message_id, full_response, true)
}

pub(crate) fn snapshot_required_event(
    missed_from: Option<u64>,
    missed_to: Option<u64>,
    resume_after: Option<u64>,
    reason: &str,
) -> StreamBundle {
    StreamBundle::now(
        EventType::SnapshotRequired,
        String::new(),
        json!({
            "from": missed_from,
            "to": missed_to,
            "resume_after": resume_after,
            "reason": reason,
            "snapshot_required": true,
        }),
    )
}

pub(crate) fn conversation_closed_event(
    conversation_id: &str,
    reason: &str,
    turn_count: u32,
) -> StreamBundle {
    StreamBundle::now(
        EventType::ConversationClosed,
        String::new(),
        json!({
            "conversation_id": conversation_id,
            "reason": reason,
            "turn_count": turn_count,
        }),
    )
}

pub(crate) fn conversation_changed_event(conversation_id: &str) -> StreamBundle {
    StreamBundle::now(
        EventType::ConversationChanged,
        String::new(),
        json!({ "conversation_id": conversation_id }),
    )
}
