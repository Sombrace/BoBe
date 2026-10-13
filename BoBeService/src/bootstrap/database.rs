use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use tracing::info;

use crate::error::AppError;

const SCHEMA: &str = include_str!("../../migrations/schema.sql");
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 9;

pub(crate) async fn read_schema_version(database: &std::path::Path) -> Result<i64, AppError> {
    use sqlx::{Connection, SqliteConnection};

    let options = SqliteConnectOptions::new()
        .filename(database)
        .read_only(true)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let has_schema_table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'bobe_schema'",
    )
    .fetch_one(&mut connection)
    .await?;
    let version: Option<i64> = if has_schema_table == 0 {
        None
    } else {
        sqlx::query_scalar("SELECT version FROM bobe_schema WHERE singleton = 1")
            .fetch_optional(&mut connection)
            .await?
    };
    connection.close().await?;
    version.ok_or_else(|| AppError::Conflict("database has no schema version".into()))
}

pub(crate) async fn connect_and_apply_schema(db_url: &str) -> Result<(SqlitePool, i64), AppError> {
    let db_url = normalize_sqlite_url(db_url);

    if let Some(path) = sqlite_file_path(&db_url)
        && let Some(parent) = std::path::Path::new(path).parent()
    {
        tokio::fs::create_dir_all(parent).await?;
    }

    let opts: SqliteConnectOptions = db_url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true)
        .pragma("journal_mode", "WAL")
        .pragma("foreign_keys", "ON")
        .pragma("busy_timeout", "5000");

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    refuse_other_schema(&pool).await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    stamp_schema_version(&pool).await?;

    info!(version = CURRENT_SCHEMA_VERSION, "database.schema_applied");
    Ok((pool, CURRENT_SCHEMA_VERSION))
}

async fn refuse_other_schema(pool: &SqlitePool) -> Result<(), AppError> {
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_one(pool)
    .await?;
    if tables == 0 {
        return Ok(());
    }
    let has_schema_table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'bobe_schema'",
    )
    .fetch_one(pool)
    .await?;
    let version: Option<i64> = if has_schema_table == 0 {
        None
    } else {
        sqlx::query_scalar("SELECT version FROM bobe_schema WHERE singleton = 1")
            .fetch_optional(pool)
            .await?
    };
    if version == Some(CURRENT_SCHEMA_VERSION) {
        return Ok(());
    }
    Err(AppError::Conflict(format!(
        "database schema version {} is not the version this daemon uses ({CURRENT_SCHEMA_VERSION}); start from a new data root",
        version.map_or_else(|| "unknown".to_owned(), |version| version.to_string())
    )))
}

async fn stamp_schema_version(pool: &SqlitePool) -> Result<(), AppError> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO bobe_schema (singleton, version, daemon_version)
         VALUES (1, ?, ?)
         ON CONFLICT(singleton) DO UPDATE SET
           version = excluded.version,
           daemon_version = excluded.daemon_version,
           applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )
    .bind(CURRENT_SCHEMA_VERSION)
    .bind(env!("CARGO_PKG_VERSION"))
    .execute(&mut *transaction)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"
    )))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

fn normalize_sqlite_url(db_url: &str) -> String {
    if let Some(path) = db_url.strip_prefix("sqlite:") {
        let expanded = crate::util::paths::expand_tilde(path);
        return format!("sqlite:{}", expanded.display());
    }
    db_url.to_string()
}

fn sqlite_file_path(db_url: &str) -> Option<&str> {
    let raw = db_url.strip_prefix("sqlite:")?;
    if raw.starts_with(":memory:") {
        return None;
    }
    Some(raw.split('?').next().unwrap_or(raw))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on precondition failures")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_database_does_not_preclaim_sample_goal_seed() {
        let (pool, _) = connect_and_apply_schema("sqlite::memory:").await.unwrap();
        let markers: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runtime_markers WHERE key = 'sample_goal_seeded_v1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(markers, 0);
    }

    #[tokio::test]
    async fn schema_version_can_be_read_without_opening_a_write_connection() {
        let root = std::env::temp_dir().join(format!("bobe-schema-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("bobe.sqlite3");
        let url = format!("sqlite:{}", database.display());
        let (pool, _) = connect_and_apply_schema(&url).await.unwrap();
        pool.close().await;

        assert_eq!(
            read_schema_version(&database).await.unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        let (pool, _) = connect_and_apply_schema(&url).await.unwrap();
        pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_database_at_another_schema_version_is_refused_untouched() {
        for setup in [
            format!(
                "CREATE TABLE bobe_schema (singleton INTEGER PRIMARY KEY, version INTEGER NOT NULL, \
                 applied_at TEXT NOT NULL, daemon_version TEXT NOT NULL); \
                 INSERT INTO bobe_schema VALUES (1, {}, 'then', 'older');",
                CURRENT_SCHEMA_VERSION - 1
            ),
            format!(
                "CREATE TABLE bobe_schema (singleton INTEGER PRIMARY KEY, version INTEGER NOT NULL, \
                 applied_at TEXT NOT NULL, daemon_version TEXT NOT NULL); \
                 INSERT INTO bobe_schema VALUES (1, {}, 'later', 'newer');",
                CURRENT_SCHEMA_VERSION + 1
            ),
            "CREATE TABLE conversations (id BLOB PRIMARY KEY);".to_owned(),
        ] {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("connect");
            sqlx::raw_sql(sqlx::AssertSqlSafe(setup))
                .execute(&pool)
                .await
                .expect("existing database");
            let tables_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master")
                .fetch_one(&pool)
                .await
                .expect("count");

            let error = refuse_other_schema(&pool).await.expect_err("must refuse");

            assert!(matches!(error, AppError::Conflict(_)));
            let tables_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master")
                .fetch_one(&pool)
                .await
                .expect("count");
            assert_eq!(tables_after, tables_before);
        }
    }

    #[tokio::test]
    async fn schema_stamp_sets_table_and_pragma_versions() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::raw_sql(SCHEMA).execute(&pool).await.expect("schema");
        stamp_schema_version(&pool).await.expect("stamp");
        let table_version: i64 =
            sqlx::query_scalar("SELECT version FROM bobe_schema WHERE singleton = 1")
                .fetch_one(&pool)
                .await
                .expect("table version");
        let pragma_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("pragma version");
        assert_eq!(table_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(pragma_version, CURRENT_SCHEMA_VERSION);
    }
}
