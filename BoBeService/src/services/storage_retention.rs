use std::path::PathBuf;

use chrono::{Duration, Utc};
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::config::Config;
use crate::error::AppError;

pub(crate) const RETENTION_INTERVAL: std::time::Duration = std::time::Duration::from_hours(6);
const CONVERSATION_PRUNE_BATCH_SIZE: i64 = 100;

pub(crate) async fn run(pool: &SqlitePool, config: &Config) {
    let conversations_deleted = match prune_closed_conversations(pool, config).await {
        Ok(count) => count,
        Err(error) => {
            warn!(error = %error, "retention.conversation_prune_failed");
            0
        }
    };

    let logging = config.logging.clone();
    let log_result = tokio::task::spawn_blocking(move || prune_daily_logs(&logging)).await;
    let (logs_deleted, log_bytes_deleted) = match log_result {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            warn!(error = %error, "retention.log_prune_failed");
            (0, 0)
        }
        Err(error) => {
            warn!(error = %error, "retention.log_prune_task_failed");
            (0, 0)
        }
    };

    if conversations_deleted > 0 || logs_deleted > 0 || log_bytes_deleted > 0 {
        info!(
            conversations = conversations_deleted,
            logs = logs_deleted,
            log_bytes = log_bytes_deleted,
            "retention.completed"
        );
    }
}

async fn prune_closed_conversations(pool: &SqlitePool, config: &Config) -> Result<u64, AppError> {
    let cutoff = Utc::now() - Duration::days(i64::from(config.conversation.closed_retention_days));
    let keep_count = i64::from(config.conversation.closed_retention_count);
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let candidate_ids: Vec<(Vec<u8>,)> = sqlx::query_as(
        "SELECT id FROM conversations WHERE state = 'closed' \
         AND (closed_at < ?1 OR id NOT IN (SELECT id FROM conversations WHERE state = 'closed' \
         ORDER BY closed_at DESC, id DESC LIMIT ?2)) \
         ORDER BY closed_at ASC, id ASC LIMIT ?3",
    )
    .bind(cutoff)
    .bind(keep_count)
    .bind(CONVERSATION_PRUNE_BATCH_SIZE)
    .fetch_all(&mut *transaction)
    .await?;

    for (id,) in &candidate_ids {
        sqlx::query("DELETE FROM conversation_turns WHERE conversation_id = ?1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM conversations WHERE id = ?1 AND state = 'closed'")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query(
        "INSERT OR IGNORE INTO message_request_tombstones (request_id) \
         SELECT request_id FROM message_requests \
         WHERE created_at < ?1 AND status IN ('completed', 'failed')",
    )
    .bind(cutoff)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM message_requests WHERE created_at < ?1 \
         AND status IN ('completed', 'failed')",
    )
    .bind(cutoff)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(candidate_ids.len() as u64)
}

#[derive(Debug)]
struct LogFile {
    path: PathBuf,
    date: chrono::NaiveDate,
    bytes: u64,
    active: bool,
}

fn prune_daily_logs(config: &crate::config::LoggingConfig) -> std::io::Result<(usize, u64)> {
    let Some((directory, prefix)) = crate::util::logging::daily_log_target(config) else {
        return Ok((0, 0));
    };
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(error) => return Err(error),
    };
    let today = Utc::now().date_naive();
    let cutoff = today - Duration::days(i64::from(config.retention_days));
    let owned_prefix = format!("{prefix}.");
    let owned_suffix = format!(".{}", crate::util::logging::DAILY_LOG_SUFFIX);
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(date_text) = name
            .strip_prefix(&owned_prefix)
            .and_then(|rest| rest.strip_suffix(&owned_suffix))
        else {
            continue;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(date_text, "%Y-%m-%d") else {
            continue;
        };
        if entry.file_type()?.is_file() {
            let metadata = entry.metadata()?;
            files.push(LogFile {
                path: entry.path(),
                date,
                bytes: metadata.len(),
                active: date == today,
            });
        }
    }
    files.sort_by_key(|file| file.date);
    let mut total_bytes = files.iter().map(|file| file.bytes).sum::<u64>();
    let mut remaining_count = files.len();
    let max_count = config.retention_count as usize;
    let mut deleted = 0;
    let mut bytes_deleted = 0;
    for file in files {
        if file.active {
            continue;
        }
        if file.date < cutoff
            || remaining_count > max_count
            || total_bytes > config.retention_total_bytes
        {
            std::fs::remove_file(&file.path)?;
            deleted += 1;
            bytes_deleted += file.bytes;
            remaining_count -= 1;
            total_bytes = total_bytes.saturating_sub(file.bytes);
        }
    }
    Ok((deleted, bytes_deleted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/schema.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn insert_conversation(
        pool: &SqlitePool,
        state: &str,
        closed_at: chrono::DateTime<Utc>,
    ) -> Vec<u8> {
        let id = uuid::Uuid::new_v4().as_bytes().to_vec();
        sqlx::query("INSERT INTO conversations (id, state, closed_at, created_at, updated_at) VALUES (?1, ?2, ?3, ?3, ?3)")
            .bind(&id).bind(state).bind(closed_at).execute(pool).await.unwrap();
        id
    }

    #[tokio::test]
    async fn pruning_is_bounded_and_preserves_active() {
        let pool = test_pool().await;
        let old = Utc::now() - Duration::days(365);
        let active = insert_conversation(&pool, "active", old).await;
        for _ in 0..105 {
            insert_conversation(&pool, "closed", old).await;
        }
        let mut config = Config::default();
        config.conversation.closed_retention_days = 30;
        config.conversation.closed_retention_count = 1_000;
        assert_eq!(
            prune_closed_conversations(&pool, &config).await.unwrap(),
            100
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id = ?1")
            .bind(active)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn pruning_waits_for_concurrent_job_delivery_writes() {
        let root =
            std::env::temp_dir().join(format!("bobe-retention-writer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(root.join("data.sqlite3"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .busy_timeout(std::time::Duration::from_secs(2)),
            )
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/schema.sql"))
            .execute(&pool)
            .await
            .unwrap();
        insert_conversation(&pool, "closed", Utc::now() - Duration::days(365)).await;
        let mut writer = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO conversations (id) VALUES (?1)")
            .bind(uuid::Uuid::new_v4())
            .execute(&mut *writer)
            .await
            .unwrap();
        let reader_pool = pool.clone();
        let pruning = tokio::spawn(async move {
            prune_closed_conversations(&reader_pool, &Config::default()).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let waited = !pruning.is_finished();
        writer.commit().await.unwrap();
        let result = pruning.await.unwrap();
        pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            waited,
            "retention must wait for the existing writer instead of failing a snapshot upgrade"
        );
        assert_eq!(result.unwrap(), 1);
    }

    #[tokio::test]
    async fn pruning_preserves_executable_jobs_and_tombstones_terminal_requests() {
        let pool = test_pool().await;
        let old = Utc::now() - Duration::days(365);
        for status in ["queued", "running", "completed", "failed"] {
            sqlx::query(
                "INSERT INTO message_requests \
                 (request_id, content_sha256, content, status, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(vec![0_u8; 32])
            .bind(status)
            .bind(status)
            .bind(old)
            .execute(&pool)
            .await
            .unwrap();
        }
        let mut config = Config::default();
        config.conversation.closed_retention_days = 30;

        prune_closed_conversations(&pool, &config).await.unwrap();

        let remaining: Vec<(String,)> =
            sqlx::query_as("SELECT status FROM message_requests ORDER BY status")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            remaining,
            vec![("queued".to_owned(),), ("running".to_owned(),)]
        );
        let tombstones: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_request_tombstones")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tombstones, 2);
    }

    #[test]
    fn log_pruning_deletes_old_daily_files_and_keeps_the_rest() {
        let directory =
            std::env::temp_dir().join(format!("bobe-retention-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        drop(crate::util::logging::daily_appender(&directory, "bobe").unwrap());
        let today = Utc::now().date_naive().to_string();
        let current = std::fs::read_dir(&directory)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_type().unwrap().is_file())
            .unwrap()
            .file_name()
            .into_string()
            .unwrap();
        assert_eq!(current, format!("bobe.{today}.log"));
        let old = (Utc::now().date_naive() - Duration::days(30)).to_string();
        std::fs::write(
            directory.join(current.replace(&today, &old)),
            vec![0_u8; 10],
        )
        .unwrap();
        std::fs::write(directory.join("other.2000-01-01.log"), b"keep").unwrap();
        let config = crate::config::LoggingConfig {
            file: Some(directory.join("bobe").to_string_lossy().into_owned()),
            retention_days: 15,
            ..crate::config::LoggingConfig::default()
        };
        assert_eq!(prune_daily_logs(&config).unwrap(), (1, 10));
        assert!(directory.join(&current).exists());
        assert!(directory.join("bobe").is_symlink());
        assert!(directory.join("other.2000-01-01.log").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
