use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuntimeIdentity {
    pub(crate) runtime_authority_id: Uuid,
    pub(crate) runtime_authority_epoch: i64,
    pub(crate) runtime_incarnation_id: Uuid,
    pub(crate) daemon_version: &'static str,
    pub(crate) schema_version: i64,
    pub(crate) started_at: DateTime<Utc>,
}

pub(crate) async fn claim(
    pool: &SqlitePool,
    paths: &crate::util::paths::DataPaths,
    schema_version: i64,
) -> Result<RuntimeIdentity, AppError> {
    load_identity(pool, paths, schema_version, false).await
}

pub(crate) async fn claim_and_start(
    pool: &SqlitePool,
    paths: &crate::util::paths::DataPaths,
    schema_version: i64,
) -> Result<RuntimeIdentity, AppError> {
    load_identity(pool, paths, schema_version, true).await
}

async fn load_identity(
    pool: &SqlitePool,
    paths: &crate::util::paths::DataPaths,
    schema_version: i64,
    register_start: bool,
) -> Result<RuntimeIdentity, AppError> {
    let mut transaction = pool.begin().await?;
    let proposed_authority = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let started_at = Utc::now();
    let data_root = paths.root().to_string_lossy().into_owned();

    sqlx::query(
        "INSERT OR IGNORE INTO runtime_authority
         (singleton, authority_id, authority_epoch, data_root,
          last_incarnation_id, last_started_at, last_daemon_version)
         VALUES (1, ?, 1, ?, ?, ?, ?)",
    )
    .bind(proposed_authority.as_bytes().as_slice())
    .bind(&data_root)
    .bind(incarnation.as_bytes().as_slice())
    .bind(started_at.to_rfc3339())
    .bind(env!("CARGO_PKG_VERSION"))
    .execute(&mut *transaction)
    .await?;

    let (authority_bytes, authority_epoch, stored_root): (Vec<u8>, i64, String) = sqlx::query_as(
        "SELECT authority_id, authority_epoch, data_root
         FROM runtime_authority WHERE singleton = 1",
    )
    .fetch_one(&mut *transaction)
    .await?;
    let runtime_authority_id = Uuid::from_slice(&authority_bytes)
        .map_err(|error| AppError::Internal(format!("invalid runtime authority UUID: {error}")))?;

    if register_start {
        sqlx::query(
            "UPDATE runtime_authority
             SET last_incarnation_id = ?, last_started_at = ?,
                 last_daemon_version = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE singleton = 1",
        )
        .bind(incarnation.as_bytes().as_slice())
        .bind(started_at.to_rfc3339())
        .bind(env!("CARGO_PKG_VERSION"))
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;

    if stored_root != data_root {
        tracing::warn!(
            stored_root,
            active_root = data_root,
            "runtime.authority_data_root_changed"
        );
    }

    Ok(RuntimeIdentity {
        runtime_authority_id,
        runtime_authority_epoch: authority_epoch,
        runtime_incarnation_id: incarnation,
        daemon_version: env!("CARGO_PKG_VERSION"),
        schema_version,
        started_at,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn paths() -> crate::util::paths::DataPaths {
        let root = std::env::temp_dir().join(format!("bobe-identity-{}", Uuid::new_v4()));
        crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}/data/bobrust.db", root.display()),
        )
        .expect("paths")
    }

    #[tokio::test]
    async fn authority_is_stable_and_incarnation_changes() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let paths = paths();
        let first = claim_and_start(&pool, &paths, 1)
            .await
            .expect("first claim");
        let second = claim_and_start(&pool, &paths, 1)
            .await
            .expect("second load");
        assert_eq!(first.runtime_authority_id, second.runtime_authority_id);
        assert_eq!(first.runtime_authority_epoch, 1);
        assert_eq!(second.runtime_authority_epoch, 1);
        assert_ne!(first.runtime_incarnation_id, second.runtime_incarnation_id);
    }
}
