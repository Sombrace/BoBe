#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InterruptedReply {
    pub(crate) heard: String,
    pub(crate) cut_off: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ChatPrompt {
    pub(crate) text: String,
    pub(crate) channel: super::TurnChannel,
    pub(crate) session_key: crate::models::ids::ConversationChannelId,
    pub(crate) run_id: Option<uuid::Uuid>,
    pub(crate) background: bool,
    pub(crate) goal_id: Option<crate::models::ids::GoalId>,
    pub(crate) interrupted_reply: Option<InterruptedReply>,
}

impl ChatPrompt {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            channel: super::TurnChannel::Text,
            session_key: crate::models::ids::ConversationChannelId::OWNER,
            run_id: None,
            background: false,
            goal_id: None,
            interrupted_reply: None,
        }
    }

    pub(crate) fn voice(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            channel: super::TurnChannel::Voice,
            session_key: crate::models::ids::ConversationChannelId::OWNER,
            run_id: None,
            background: false,
            goal_id: None,
            interrupted_reply: None,
        }
    }

    pub(crate) fn in_channel(mut self, channel: crate::models::ids::ConversationChannelId) -> Self {
        self.session_key = channel;
        self
    }

    pub(crate) fn in_run(mut self, run_id: uuid::Uuid) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub(crate) fn for_goal(mut self, goal: Option<crate::models::ids::GoalId>) -> Self {
        self.goal_id = goal;
        self
    }

    pub(crate) fn after_interruption(mut self, reply: Option<InterruptedReply>) -> Self {
        self.interrupted_reply = reply;
        self
    }

    pub(crate) fn background(text: impl Into<String>) -> Self {
        Self {
            background: true,
            ..Self::text(text)
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ChatDelta {
    MessageDelta {
        message_id: String,
        text: String,
    },
    MessageComplete {
        message_id: String,
        content: String,
        output_tokens: u64,
    },
    ToolStart {
        id: String,
        name: String,
    },
    ToolComplete {
        id: String,
        name: String,
        success: bool,
    },
    Error {
        message: String,
    },
    Done,
}
