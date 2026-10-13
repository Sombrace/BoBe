#[derive(Debug, thiserror::Error)]
pub(crate) enum WorkerError {
    #[error("provider: {0}")]
    Provider(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}
