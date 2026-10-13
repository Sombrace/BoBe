use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WorkerClass {
    Goals,
    Small,
    Chat,
    Consolidate,
}

impl WorkerClass {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Goals => "goals",
            Self::Small => "small",
            Self::Chat => "chat",
            Self::Consolidate => "consolidate",
        }
    }

    pub(crate) const fn turn_timeout(self) -> Duration {
        self.budget().wall_clock
    }
}
