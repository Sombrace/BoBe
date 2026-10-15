use std::time::Duration;

use super::WorkerClass;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub(crate) struct WorkloadBudget {
    pub(crate) wall_clock: Duration,
    pub(crate) max_prompt_tokens: usize,
    pub(crate) max_output_tokens: usize,
}

impl WorkloadBudget {
    pub(crate) const fn context_tokens(self) -> usize {
        self.max_prompt_tokens + self.max_output_tokens
    }
}

impl WorkerClass {
    pub(crate) fn largest_context_tokens() -> usize {
        [Self::Goals, Self::Small, Self::Chat, Self::Consolidate]
            .into_iter()
            .map(|class| class.budget().context_tokens())
            .fold(0, usize::max)
    }

    pub(crate) const fn budget(self) -> WorkloadBudget {
        match self {
            Self::Chat => WorkloadBudget {
                wall_clock: Duration::from_mins(15),
                max_prompt_tokens: 32_000,
                max_output_tokens: 4_096,
            },
            Self::Goals | Self::Small => WorkloadBudget {
                wall_clock: Duration::from_mins(3),
                max_prompt_tokens: 16_000,
                max_output_tokens: 4_096,
            },
            Self::Consolidate => WorkloadBudget {
                wall_clock: Duration::from_mins(15),
                max_prompt_tokens: 32_000,
                max_output_tokens: 8_192,
            },
        }
    }
}
