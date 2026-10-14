use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};

use crate::error::AppError;

const TABLES: [&str; 12] = [
    "agent_jobs",
    "conversation_turns",
    "conversations",
    "cooldown_state",
    "goal_plan_steps",
    "goal_plans",
    "goals",
    "learning_state",
    "memories",
    "observations",
    "souls",
    "user_profiles",
];

pub(crate) async fn purge(root: &Path) -> Result<bool, AppError> {
    let database = root.join("data").join("bobrust.db");
    if !database.is_file() || !is_release_0_2_database(&database).await? {
        return Ok(false);
    }
    for suffix in ["", "-wal", "-shm", "-journal"] {
        remove_if_present(&with_suffix(&database, suffix))?;
    }
    remove_if_present(&root.join("config.toml"))?;
    Ok(true)
}

async fn is_release_0_2_database(database: &Path) -> Result<bool, AppError> {
    let options = SqliteConnectOptions::new().filename(database);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&mut connection)
    .await?;
    connection.close().await?;
    let tables: BTreeSet<&str> = tables.iter().map(String::as_str).collect();
    Ok(tables == TABLES.into_iter().collect())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn remove_if_present(path: &Path) -> Result<(), AppError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_0_2_schema() -> String {
        TABLES
            .iter()
            .map(|table| format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY);"))
            .collect()
    }

    async fn root_with(statements: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bobe-release-0-2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("data")).unwrap();
        let options = SqliteConnectOptions::new()
            .filename(root.join("data").join("bobrust.db"))
            .create_if_missing(true);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(statements.to_owned()))
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        std::fs::write(root.join("config.toml"), "config_version = 1\n").unwrap();
        root
    }

    #[tokio::test]
    async fn a_release_0_2_database_and_its_config_are_purged() {
        let root = root_with(&release_0_2_schema()).await;

        assert!(purge(&root).await.unwrap());

        assert!(!root.join("data").join("bobrust.db").exists());
        assert!(!root.join("config.toml").exists());
        assert!(root.join("data").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn any_other_database_and_its_config_are_kept() {
        for statements in [
            format!(
                "{} CREATE TABLE bobe_schema (singleton INTEGER PRIMARY KEY);",
                release_0_2_schema()
            ),
            format!(
                "{} CREATE TABLE endpoints (id INTEGER PRIMARY KEY);",
                release_0_2_schema()
            ),
            "CREATE TABLE conversations (id BLOB PRIMARY KEY);".to_owned(),
        ] {
            let root = root_with(&statements).await;

            assert!(!purge(&root).await.unwrap());

            assert!(root.join("data").join("bobrust.db").is_file());
            assert!(root.join("config.toml").is_file());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn a_root_without_a_database_is_untouched() {
        let root = std::env::temp_dir().join(format!("bobe-release-0-2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.toml"), "").unwrap();

        assert!(!purge(&root).await.unwrap());

        assert!(root.join("config.toml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
