use crate::error::AppError;
use crate::models::cooldown::{Cooldown, CooldownInfo};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tracing::{debug, info};

const SELECT_STATE: &str = "SELECT id, last_engagement, last_user_response, created_at, updated_at FROM cooldown_state ORDER BY created_at, id LIMIT 1";

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

    pub(crate) async fn load_or_create(&self) -> Result<(), AppError> {
        let mut state = self.state.lock().await;
        if state.is_none() {
            *state = Some(self.fetch_or_create().await?);
        }
        Ok(())
    }

    pub(crate) async fn check_cooldown(
        &self,
        base_minutes: i64,
        extended_minutes: i64,
    ) -> Result<Option<CooldownInfo>, AppError> {
        let mut state = self.state.lock().await;
        let cooldown = match state.take() {
            Some(cooldown) => cooldown,
            None => self.fetch_or_create().await?,
        };
        Ok(state
            .insert(cooldown)
            .check_cooldown(base_minutes, extended_minutes))
    }

    pub(crate) async fn update_last_engagement(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.record(
            "UPDATE cooldown_state SET last_engagement = ?1, updated_at = ?2 WHERE id = ?3",
            timestamp,
            |cooldown| cooldown.last_engagement = Some(timestamp),
        )
        .await?;
        debug!(last_engagement = %timestamp, "cooldown_repository.state_saved");
        Ok(())
    }

    pub(crate) async fn update_last_user_response(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.record(
            "UPDATE cooldown_state SET last_user_response = ?1, updated_at = ?2 WHERE id = ?3",
            timestamp,
            |cooldown| cooldown.last_user_response = Some(timestamp),
        )
        .await?;
        debug!(last_user_response = %timestamp, "cooldown_repository.state_saved");
        Ok(())
    }

    async fn record(
        &self,
        statement: &'static str,
        timestamp: DateTime<Utc>,
        apply: impl FnOnce(&mut Cooldown),
    ) -> Result<(), AppError> {
        let mut state = self.state.lock().await;
        let mut cooldown = match state.take() {
            Some(cooldown) => cooldown,
            None => self.fetch_or_create().await?,
        };
        let updated_at = Utc::now();
        sqlx::query(statement)
            .bind(timestamp)
            .bind(updated_at)
            .bind(cooldown.id)
            .execute(&self.pool)
            .await?;
        apply(&mut cooldown);
        cooldown.updated_at = updated_at;
        *state = Some(cooldown);
        Ok(())
    }

    async fn fetch_or_create(&self) -> Result<Cooldown, AppError> {
        let candidate = Cooldown::new();
        let created = sqlx::query(
            "INSERT INTO cooldown_state (id, created_at, updated_at) SELECT ?1, ?2, ?3 WHERE NOT EXISTS (SELECT 1 FROM cooldown_state)",
        )
        .bind(candidate.id)
        .bind(candidate.created_at)
        .bind(candidate.updated_at)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1;
        let state = sqlx::query_as::<_, Cooldown>(SELECT_STATE)
            .fetch_one(&self.pool)
            .await?;
        if created {
            info!(state_id = %state.id, "cooldown_repository.state_created");
        } else {
            info!(
                state_id = %state.id,
                last_engagement = ?state.last_engagement,
                last_user_response = ?state.last_user_response,
                "cooldown_repository.state_loaded"
            );
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::in_memory_pool;
    use chrono::Duration as ChronoDuration;
    use std::sync::Arc;
    use std::time::Duration;

    async fn stored(pool: &SqlitePool) -> Cooldown {
        sqlx::query_as::<_, Cooldown>(SELECT_STATE)
            .fetch_one(pool)
            .await
            .expect("stored cooldown")
    }

    async fn cached(repo: &SqliteCooldownRepo) -> Cooldown {
        repo.state.lock().await.clone().expect("cooldown is cached")
    }

    async fn rows(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM cooldown_state")
            .fetch_one(pool)
            .await
            .expect("cooldown rows")
    }

    #[tokio::test]
    async fn load_or_create_is_idempotent() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool.clone());

        repo.load_or_create().await.expect("first load_or_create");
        let id_first = cached(&repo).await.id;

        repo.load_or_create().await.expect("second load_or_create");
        assert_eq!(cached(&repo).await.id, id_first);
        assert_eq!(rows(&pool).await, 1);
    }

    #[tokio::test]
    async fn concurrent_first_use_creates_one_row() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool.clone());
        let other = SqliteCooldownRepo::new(pool.clone());

        let (loaded, checked, responded, other_loaded) = tokio::join!(
            repo.load_or_create(),
            repo.check_cooldown(5, 30),
            repo.update_last_user_response(Utc::now()),
            other.load_or_create(),
        );
        loaded.expect("load");
        checked.expect("check");
        responded.expect("response");
        other_loaded.expect("second repository load");

        assert_eq!(rows(&pool).await, 1);
        assert_eq!(cached(&repo).await.id, cached(&other).await.id);
    }

    #[tokio::test]
    async fn the_oldest_row_is_always_the_cooldown() {
        let pool = in_memory_pool().await;
        sqlx::raw_sql(
            "INSERT INTO cooldown_state (id, last_engagement, created_at) VALUES
                (X'00000000000000000000000000000002', '2026-10-15T07:00:00.000Z', '2026-10-02T00:00:00.000Z'),
                (X'00000000000000000000000000000003', NULL, '2026-10-01T00:00:00.000Z'),
                (X'00000000000000000000000000000001', NULL, '2026-10-01T00:00:00.000Z')",
        )
        .execute(&pool)
        .await
        .expect("duplicate rows from an earlier race");

        for _ in 0..3 {
            let repo = SqliteCooldownRepo::new(pool.clone());
            repo.load_or_create().await.expect("load");
            let loaded = cached(&repo).await;
            assert_eq!(
                loaded.id,
                crate::models::ids::CooldownId::from(uuid::Uuid::from_u128(1))
            );
            assert_eq!(loaded.last_engagement, None);
        }
        assert_eq!(rows(&pool).await, 3);
    }

    #[tokio::test]
    async fn admission_waits_for_an_engagement_being_written() {
        let pool = in_memory_pool().await;
        let repo = Arc::new(SqliteCooldownRepo::new(pool.clone()));
        repo.load_or_create().await.expect("seed cooldown state");
        let connection = pool.acquire().await.expect("hold the database");

        let update = tokio::spawn({
            let repo = Arc::clone(&repo);
            async move { repo.update_last_engagement(Utc::now()).await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while repo.state.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the update owns the cooldown while it writes");
        let check = tokio::spawn({
            let repo = Arc::clone(&repo);
            async move { repo.check_cooldown(5, 30).await }
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!check.is_finished());

        drop(connection);
        update.await.expect("update task").expect("update");
        let cooldown = check.await.expect("check task").expect("check");
        assert_eq!(
            cooldown
                .expect("engagement starts a cooldown")
                .cooldown_type,
            "ai_engagement"
        );
    }

    #[tokio::test]
    async fn concurrent_updates_leave_the_cache_equal_to_sqlite() {
        let pool = in_memory_pool().await;
        let repo = Arc::new(SqliteCooldownRepo::new(pool.clone()));
        let start = Utc::now() - ChronoDuration::minutes(60);
        let updates = (0..24)
            .map(|index| {
                let repo = Arc::clone(&repo);
                let timestamp = start + ChronoDuration::seconds(index);
                tokio::spawn(async move {
                    if index % 2 == 0 {
                        repo.update_last_engagement(timestamp).await
                    } else {
                        repo.update_last_user_response(timestamp).await
                    }
                })
            })
            .collect::<Vec<_>>();
        for update in updates {
            update.await.expect("update task").expect("update");
        }

        let cached = cached(&repo).await;
        let stored = stored(&pool).await;
        assert_eq!(rows(&pool).await, 1);
        assert_eq!(cached.id, stored.id);
        assert_eq!(cached.last_engagement, stored.last_engagement);
        assert_eq!(cached.last_user_response, stored.last_user_response);
        assert_eq!(cached.updated_at, stored.updated_at);
    }

    #[tokio::test]
    async fn check_cooldown_returns_none_before_engagement() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool);
        assert!(repo.check_cooldown(5, 30).await.expect("check").is_none());
    }

    #[tokio::test]
    async fn check_cooldown_fires_within_window() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool);
        repo.load_or_create().await.expect("seed cooldown state");
        repo.update_last_engagement(Utc::now())
            .await
            .expect("record engagement");

        let info = repo.check_cooldown(5, 30).await.expect("check");
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
        assert!(repo.check_cooldown(5, 30).await.expect("check").is_none());
    }

    #[tokio::test]
    async fn update_user_response_persists() {
        let pool = in_memory_pool().await;
        let repo = SqliteCooldownRepo::new(pool.clone());
        let ts = Utc::now();
        repo.update_last_user_response(ts)
            .await
            .expect("update user response");
        assert_eq!(cached(&repo).await.last_user_response, Some(ts));
        assert_eq!(stored(&pool).await.last_user_response, Some(ts));
    }
}
