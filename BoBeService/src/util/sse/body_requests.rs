use std::time::Duration;

use serde::Deserialize;

use crate::util::sse::types::{EventType, StreamBundle};

pub(crate) const SCREEN_TEXT_MAX_CHARS: usize = 6_000;
const SCREEN_LABEL_MAX_CHARS: usize = 512;
const REASON_MAX_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyAction {
    ScreenRead,
}

impl BodyAction {
    const ALL: [Self; 1] = [Self::ScreenRead];

    pub(crate) const fn wire_name(self) -> &'static str {
        match self {
            Self::ScreenRead => "screen.read",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::ScreenRead => 1,
        }
    }

    fn from_wire(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.wire_name() == name)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BodyActions(u8);

impl BodyActions {
    pub(crate) fn from_header(value: Option<&str>) -> Self {
        let mut actions = Self::default();
        for name in value
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if let Some(action) = BodyAction::from_wire(name) {
                actions.0 |= action.bit();
            } else {
                tracing::debug!(action = name, "body_requests.unknown_action_ignored");
            }
        }
        actions
    }

    pub(crate) const fn contains(self, action: BodyAction) -> bool {
        self.0 & action.bit() != 0
    }

    #[cfg(test)]
    pub(crate) const fn screen() -> Self {
        Self(BodyAction::ScreenRead.bit())
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum BodyReply {
    Completed { result: serde_json::Value },
    Declined { reason: String },
    Failed { reason: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum BodyRequestError {
    #[error("no connected body can do this")]
    NoBody,
    #[error("the body is still busy with earlier requests")]
    Busy,
    #[error("the body disconnected before answering")]
    Gone,
    #[error("the body did not answer in time")]
    TimedOut,
    #[error("the body declined: {0}")]
    Declined(String),
    #[error("the body failed: {0}")]
    Failed(String),
    #[error("the body's answer was not understood: {0}")]
    Invalid(String),
}

impl BodyRequestError {
    pub(crate) fn from_reply(reply: BodyReply) -> Result<serde_json::Value, Self> {
        match reply {
            BodyReply::Completed { result } => Ok(result),
            BodyReply::Declined { reason } => {
                Err(Self::Declined(bounded(&reason, REASON_MAX_CHARS)))
            }
            BodyReply::Failed { reason } => Err(Self::Failed(bounded(&reason, REASON_MAX_CHARS))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScreenReading {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) application: Option<String>,
    #[serde(default)]
    pub(crate) window_title: Option<String>,
}

impl ScreenReading {
    pub(crate) fn from_result(result: serde_json::Value) -> Result<Self, BodyRequestError> {
        let reading = serde_json::from_value::<Self>(result)
            .map_err(|error| BodyRequestError::Invalid(error.to_string()))?;
        Ok(Self {
            text: bounded(&reading.text, SCREEN_TEXT_MAX_CHARS),
            application: label(reading.application),
            window_title: label(reading.window_title),
        })
    }
}

pub(crate) fn body_request_event(
    request_id: &str,
    action: BodyAction,
    deadline: Duration,
) -> StreamBundle {
    StreamBundle::now(
        EventType::BodyRequest,
        request_id.to_owned(),
        serde_json::json!({
            "request_id": request_id,
            "action": action.wire_name(),
            "deadline_ms": u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
        }),
    )
}

fn label(value: Option<String>) -> Option<String> {
    value
        .map(|value| bounded(value.trim(), SCREEN_LABEL_MAX_CHARS))
        .filter(|value| !value.is_empty())
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_lists_known_actions_and_ignores_the_rest() {
        assert_eq!(
            BodyActions::from_header(Some(" screen.read , camera.snap")),
            BodyActions::screen()
        );
        assert!(!BodyActions::from_header(None).contains(BodyAction::ScreenRead));
        assert!(!BodyActions::from_header(Some("")).contains(BodyAction::ScreenRead));
    }

    #[test]
    fn replies_carry_a_status_and_bounded_reasons() {
        let declined: BodyReply =
            serde_json::from_str(r#"{"status":"declined","reason":"screen_locked"}"#).unwrap();
        assert_eq!(
            BodyRequestError::from_reply(declined),
            Err(BodyRequestError::Declined("screen_locked".into()))
        );
        let failed: BodyReply = serde_json::from_value(serde_json::json!({
            "status": "failed",
            "reason": "x".repeat(1_000),
        }))
        .unwrap();
        assert_eq!(
            BodyRequestError::from_reply(failed),
            Err(BodyRequestError::Failed("x".repeat(REASON_MAX_CHARS)))
        );
        assert!(serde_json::from_str::<BodyReply>(r#"{"status":"maybe"}"#).is_err());
    }

    #[test]
    fn screen_readings_are_bounded_and_trimmed() {
        let reading = ScreenReading::from_result(serde_json::json!({
            "text": "y".repeat(SCREEN_TEXT_MAX_CHARS + 50),
            "application": "  Safari  ",
            "window_title": "   ",
        }))
        .unwrap();
        assert_eq!(reading.text.chars().count(), SCREEN_TEXT_MAX_CHARS);
        assert_eq!(reading.application.as_deref(), Some("Safari"));
        assert_eq!(reading.window_title, None);
        assert!(matches!(
            ScreenReading::from_result(serde_json::json!({"text": "a", "image": "..."})),
            Err(BodyRequestError::Invalid(_))
        ));
    }

    #[test]
    fn requests_name_their_action_and_deadline_without_a_cursor() {
        let event = body_request_event("breq_1", BodyAction::ScreenRead, Duration::from_secs(20));
        assert_eq!(event.event_type, EventType::BodyRequest);
        assert_eq!(event.event_id, None);
        assert_eq!(event.message_id, "breq_1");
        assert_eq!(event.payload["action"], "screen.read");
        assert_eq!(event.payload["deadline_ms"], 20_000);
    }
}
