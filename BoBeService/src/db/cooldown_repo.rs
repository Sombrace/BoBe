use crate::error::AppError;
use crate::models::cooldown::{Cooldown, CooldownInfo};
use crate::models::ids::CooldownId;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tracing::{debug, info};

pub(crate) struct SqliteCooldownRepo {
    pool: SqlitePool,
    state: Mutex<Option<Cooldown>>,
}

impl SqliteCooldownRepo {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            state: Mutex::new(None),
        }
    }

    pub(crate) async fn ensure_loaded(&self) -> Result<CooldownId, AppError> {
        let guard = self.state.lock().await;
        if let Some(s) = guard.as_ref() {
            return Ok(s.id);
        }
        drop(guard);
        self.load_or_create().await?;
        let guard = self.state.lock().await;
        Ok(guard.as_ref().map_or_else(CooldownId::new, |s| s.id))
    }
}

impl SqliteCooldownRepo {
    pub(crate) fn check_cooldown(
        &self,
        base_minutes: i64,
        extended_minutes: i64,
    ) -> Option<CooldownInfo> {
        self.state
            .try_lock()
            .ok()
            .and_then(|s| s.as_ref()?.check_cooldown(base_minutes, extended_minutes))
    }

    pub(crate) async fn load_or_create(&self) -> Result<(), AppError> {
        let row = sqlx::query_as::<_, Cooldown>("SELECT * FROM cooldown_state LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;

        let state = if let Some(existing) = row {
            info!(
                state_id = %existing.id,
                last_engagement = ?existing.last_engagement,
                last_user_response = ?existing.last_user_response,
                "cooldown_repository.state_loaded"
            );
            existing
        } else {
            let new_state = Cooldown::new();
            sqlx::query(
                "INSERT INTO cooldown_state (id, created_at, updated_at) VALUES (?1, ?2, ?3)",
            )
            .bind(new_state.id)
            .bind(new_state.created_at)
            .bind(new_state.updated_at)
            .execute(&self.pool)
            .await?;
            info!(state_id = %new_state.id, "cooldown_repository.state_created");
            new_state
        };

        let mut guard = self.state.lock().await;
        *guard = Some(state);
        Ok(())
    }

    pub(crate) async fn update_last_engagement(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let id = self.ensure_loaded().await?;

        sqlx::query(
            "UPDATE cooldown_state SET last_engagement = ?1, updated_at = ?2 WHERE id = ?3",
        )
        .bind(timestamp)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;

        let mut guard = self.state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.last_engagement = Some(timestamp);
        }

        debug!(last_engagement = %timestamp, "cooldown_repository.state_saved");
        Ok(())
    }

    pub(crate) async fn update_last_user_response(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let id = self.ensure_loaded().await?;

        sqlx::query(
            "UPDATE cooldown_state SET last_user_response = ?1, updated_at = ?2 WHERE id = ?3",
        )
        .bind(timestamp)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;

        let mut guard = self.state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.last_user_response = Some(timestamp);
        }

        debug!(last_user_response = %timestamp, "cooldown_repository.state_saved");
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on precondition failures")]
mod tests {
    use super::*;
    use crate::db::test_helpers::in_memory_pool;
    use chrono::Duration as ChronoDuration;

    #[tokio::test]
    async fn load_or_create_is_idempotent() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool);

        repo.load_or_create().await.expect("first load_or_create");
        let id_first = repo.ensure_loaded().await.expect("ensure_loaded #1");

        repo.load_or_create().await.expect("second load_or_create");
        let id_second = repo.ensure_loaded().await.expect("ensure_loaded #2");

        assert_eq!(id_first, id_second);
    }

    #[tokio::test]
    async fn check_cooldown_returns_none_before_engagement() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool);
        repo.load_or_create().await.expect("seed cooldown state");
        assert!(repo.check_cooldown(5, 30).is_none());
    }

    #[tokio::test]
    async fn check_cooldown_fires_within_window() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool);
        repo.load_or_create().await.expect("seed cooldown state");
        repo.update_last_engagement(Utc::now())
            .await
            .expect("record engagement");

        let info = repo.check_cooldown(5, 30);
        assert!(
            info.is_some(),
            "expected cooldown to fire within base window"
        );
    }

    #[tokio::test]
    async fn check_cooldown_clears_after_base_window() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool);
        repo.load_or_create().await.expect("seed cooldown state");
        let past = Utc::now() - ChronoDuration::minutes(10);
        repo.update_last_engagement(past)
            .await
            .expect("record old engagement");
        assert!(repo.check_cooldown(5, 30).is_none());
    }

    #[tokio::test]
    async fn update_user_response_persists() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool);
        let ts = Utc::now();
        repo.update_last_user_response(ts)
            .await
            .expect("update user response");
        let guard = repo.state.lock().await;
        let cached = guard.as_ref().expect("state cached after update");
        assert_eq!(cached.last_user_response, Some(ts));
    }
}
