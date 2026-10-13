use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::AppError;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InitializationState {
    ProviderRequired,
    PersonalizationRequired,
    Ready,
}

impl std::fmt::Display for InitializationState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for InitializationState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).map_err(|error| error.to_string())
    }
}

impl InitializationState {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProviderRequired => "provider_required",
            Self::PersonalizationRequired => "personalization_required",
            Self::Ready => "ready",
        }
    }

    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "provider_required" => Ok(Self::ProviderRequired),
            "personalization_required" => Ok(Self::PersonalizationRequired),
            "ready" => Ok(Self::Ready),
            other => Err(AppError::Internal(format!(
                "invalid runtime initialization state: {other}"
            ))),
        }
    }
}

pub(crate) async fn ensure(pool: &SqlitePool) -> Result<InitializationState, AppError> {
    sqlx::query("INSERT OR IGNORE INTO runtime_initialization (singleton, state) VALUES (1, ?)")
        .bind(InitializationState::ProviderRequired.as_str())
        .execute(pool)
        .await?;
    get(pool).await
}

pub(crate) async fn get(pool: &SqlitePool) -> Result<InitializationState, AppError> {
    let state: String =
        sqlx::query_scalar("SELECT state FROM runtime_initialization WHERE singleton = 1")
            .fetch_one(pool)
            .await?;
    InitializationState::parse(&state)
}

pub(crate) async fn set(pool: &SqlitePool, state: InitializationState) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE runtime_initialization SET state = ?,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE singleton = 1",
    )
    .bind(state.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_runtime_requires_provider_and_a_ready_runtime_stays_ready() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        assert_eq!(
            ensure(&pool).await.expect("fresh state"),
            InitializationState::ProviderRequired
        );
        set(&pool, InitializationState::Ready)
            .await
            .expect("mark ready");
        assert_eq!(
            ensure(&pool).await.expect("stable state"),
            InitializationState::Ready
        );
    }
}
