use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;

const SCHEMA: &str = include_str!("../../migrations/schema.sql");

pub(crate) async fn in_memory_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("test setup: in-memory sqlite url is well-formed")
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("test setup: sqlite pool connect");

    sqlx::raw_sql(SCHEMA)
        .execute(&pool)
        .await
        .expect("test setup: schema apply");

    pool
}
