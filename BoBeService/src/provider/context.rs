use std::sync::Arc;

use chrono::{DateTime, Local};

use crate::provider::{DomainTools, InterruptedReply, MemoryFile};

pub(crate) const VOICE_TONE_HINT: &str = concat!(
    "[voice channel] Respond in 1-2 short sentences. ",
    "No markdown lists, code blocks, or headers. ",
    "Use natural speech rhythms — short clauses, conversational tone. ",
    "Spell out abbreviations as words. ",
    "If you can't help, suggest an alternative instead of refusing.",
);

const HEARD_REPLY_MAX_BYTES: usize = 600;
const CUT_OFF_SENTENCE_MAX_BYTES: usize = 240;

pub(crate) fn interruption_hint(reply: &InterruptedReply) -> String {
    let mut hint = String::from(
        "[voice interruption] The owner interrupted your last spoken reply before it finished. ",
    );
    if reply.heard.is_empty() {
        hint.push_str("They heard none of it in full. ");
    } else {
        let heard = crate::util::text::tail_str(&reply.heard, HEARD_REPLY_MAX_BYTES);
        hint.push_str("They heard: \"");
        if heard.len() < reply.heard.len() {
            hint.push('\u{2026}');
        }
        hint.push_str(heard);
        hint.push_str("\" ");
    }
    if let Some(cut_off) = &reply.cut_off {
        let shown = crate::util::text::truncate_str(cut_off, CUT_OFF_SENTENCE_MAX_BYTES);
        hint.push_str("You were cut off while saying: \"");
        hint.push_str(shown);
        if shown.len() < cut_off.len() {
            hint.push('\u{2026}');
        }
        hint.push_str("\" ");
    }
    hint.push_str(
        "Nothing after that was spoken. Do not assume they heard it; say it again only if it still matters.",
    );
    hint
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TurnChannel {
    #[default]
    Text,
    Voice,
}

#[derive(Debug, Clone)]
pub(crate) struct ContextBlock {
    pub(crate) body: String,
    pub(crate) memory_revision: Option<u64>,
}

pub(crate) struct TurnContext {
    memory_file: Arc<MemoryFile>,
    domain_tools: Arc<DomainTools>,
}

impl TurnContext {
    pub(crate) fn new(memory_file: Arc<MemoryFile>, domain_tools: Arc<DomainTools>) -> Arc<Self> {
        Arc::new(Self {
            memory_file,
            domain_tools,
        })
    }

    pub(crate) async fn session_context(
        &self,
        workload: WorkerClass,
    ) -> Result<ContextBlock, crate::error::AppError> {
        if !self.domain_tools.memory_enabled() {
            return Ok(ContextBlock {
                body: String::new(),
                memory_revision: None,
            });
        }
        Ok(ContextBlock {
            body: self
                .memory_file
                .context(memory_context_bytes(workload))
                .await?,
            memory_revision: Some(self.memory_file.revision()),
        })
    }

    pub(crate) async fn turn_context(
        &self,
        workload: WorkerClass,
        channel: TurnChannel,
        previously_injected_memory_revision: u64,
        now: DateTime<Local>,
    ) -> ContextBlock {
        let body = format!(
            "Current local time: {}. Worker class: {}.",
            now.format("%Y-%m-%d %H:%M:%S %z"),
            workload.name(),
        );
        let mut body = body;
        if workload == WorkerClass::Chat && channel == TurnChannel::Voice {
            body.push_str("\n\n");
            body.push_str(VOICE_TONE_HINT);
        }

        let revision = self.memory_file.revision();
        if !self.domain_tools.memory_enabled() {
            return ContextBlock {
                body,
                memory_revision: None,
            };
        }
        let memory_revision = if revision == previously_injected_memory_revision {
            None
        } else {
            match self
                .memory_file
                .context(memory_context_bytes(workload))
                .await
            {
                Ok(memory) => {
                    body.push_str("\n\n[memory refresh]\n");
                    body.push_str(&memory);
                    Some(revision)
                }
                Err(error) => {
                    tracing::warn!(%error, "provider context memory refresh failed");
                    None
                }
            }
        };
        ContextBlock {
            body,
            memory_revision,
        }
    }
}

use super::WorkerClass;

fn memory_context_bytes(workload: WorkerClass) -> usize {
    (workload.budget().max_prompt_tokens / 2).min(16 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::SkillStore;

    #[test]
    fn interruption_hint_separates_heard_from_unheard() {
        let hint = interruption_hint(&InterruptedReply {
            heard: "The train leaves at nine.".into(),
            cut_off: Some("You should book the".into()),
        });
        assert!(hint.contains("They heard: \"The train leaves at nine.\""));
        assert!(hint.contains("cut off while saying: \"You should book the\""));

        let nothing = interruption_hint(&InterruptedReply {
            heard: String::new(),
            cut_off: None,
        });
        assert!(nothing.contains("none of it in full"));
        assert!(!nothing.contains("cut off while saying"));
    }

    #[test]
    fn interruption_hint_keeps_the_end_of_a_long_reply() {
        let heard = format!("{} The last thing they heard.", "é".repeat(600));
        let hint = interruption_hint(&InterruptedReply {
            heard,
            cut_off: None,
        });
        assert!(hint.contains("The last thing they heard.\""));
        assert!(hint.contains("They heard: \"\u{2026}"));
        assert!(hint.len() < HEARD_REPLY_MAX_BYTES + 300);

        let cut = interruption_hint(&InterruptedReply {
            heard: String::new(),
            cut_off: Some("word ".repeat(100)),
        });
        assert!(cut.contains("\u{2026}\" Nothing after that was spoken."));
    }

    #[tokio::test]
    async fn voice_hint_is_chat_only() {
        let root =
            std::env::temp_dir().join(format!("bobe-provider-context-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("skills/chat")).unwrap();
        std::fs::write(
            root.join("skills/chat/SKILL.md"),
            super::super::skills::CHAT_SKILL_MD,
        )
        .unwrap();
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let memory = MemoryFile::new(root.join("memory.md"));
        let skills = SkillStore::prepare(&paths).await.unwrap();
        let context = TurnContext::new(
            Arc::clone(&memory),
            DomainTools::test_default(Arc::clone(&memory), Arc::clone(&skills), true, true),
        );
        let now = Local::now();
        let chat = context
            .turn_context(WorkerClass::Chat, TurnChannel::Voice, 0, now)
            .await;
        let goals = context
            .turn_context(WorkerClass::Goals, TurnChannel::Voice, 0, now)
            .await;
        assert!(chat.body.contains(VOICE_TONE_HINT));
        assert!(!goals.body.contains(VOICE_TONE_HINT));

        let disabled_memory = MemoryFile::new(root.join("disabled-memory.md"));
        let disabled_skills = SkillStore::prepare(&paths).await.unwrap();
        let disabled = TurnContext::new(
            Arc::clone(&disabled_memory),
            DomainTools::test_default(disabled_memory, disabled_skills, false, false),
        );
        let session = disabled.session_context(WorkerClass::Chat).await.unwrap();
        assert!(session.body.is_empty());
        assert!(session.memory_revision.is_none());
        let dynamic_context = disabled
            .turn_context(WorkerClass::Chat, TurnChannel::Text, 0, now)
            .await
            .body;
        assert!(dynamic_context.contains("Current local time:"));
        assert!(!dynamic_context.contains("Active daemon capabilities"));
        drop(tokio::fs::remove_dir_all(root).await);
    }
}
