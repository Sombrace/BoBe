use tracing::{debug, info, warn};

use crate::error::AppError;

use super::goal_md::GoalDoc;
use super::goals_service::GoalsService;

const SAMPLE_GOAL_SEEDED_MARKER: &str = "sample_goal_seeded_v1";

pub(crate) async fn seed_sample_goal(
    pool: &sqlx::SqlitePool,
    goals_service: &GoalsService,
) -> Result<(), AppError> {
    if marker_exists(pool).await? {
        debug!("goals_seeding.skipped_completed");
        return Ok(());
    }

    let claimed = sqlx::query("INSERT OR IGNORE INTO runtime_markers (key) VALUES (?1)")
        .bind(SAMPLE_GOAL_SEEDED_MARKER)
        .execute(pool)
        .await?
        .rows_affected()
        == 1;
    if !claimed {
        return Ok(());
    }

    let mut doc = GoalDoc::new(
        "Getting started with BoBe",
        "A starter goal so you can see how the goals system works. Edit any section, talk to BoBe about it, or delete this and write your own.",
    );
    doc.priority = 1;
    doc.status = crate::models::types::GoalStatus::Paused;
    doc.why_it_matters = "Goals are how BoBe tracks what you actually care about across conversations. \
They're plain Markdown files under BoBe's configurable data root, so you can edit them in any editor and BoBe will pick up the changes."
        .to_string();
    doc.how_working_on_it =
        "Try editing this goal's sections in the Settings panel, or just chat with BoBe about something you want to work on."
            .to_string();

    match goals_service.create(doc).await {
        Ok(saved) => info!(goal_id = %saved.id, "goals_seeding.created"),
        Err(e) => {
            sqlx::query("DELETE FROM runtime_markers WHERE key = ?1")
                .bind(SAMPLE_GOAL_SEEDED_MARKER)
                .execute(pool)
                .await?;
            warn!(error = %e, "goals_seeding.create_failed");
        }
    }
    Ok(())
}

async fn marker_exists(pool: &sqlx::SqlitePool) -> Result<bool, AppError> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM runtime_markers WHERE key = ?1")
            .bind(SAMPLE_GOAL_SEEDED_MARKER)
            .fetch_one(pool)
            .await?
            != 0,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests panic on fixture failures")]
mod tests {
    use super::*;

    async fn fixture() -> (std::path::PathBuf, sqlx::SqlitePool, GoalsService) {
        let root = std::env::temp_dir().join(format!("bobe-goal-seed-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap();
        let (pool, _) = crate::bootstrap::database::open(&paths).await.unwrap();
        let service = GoalsService::new(crate::services::goals::file_store::GoalFileStore::new(
            root.join("goals"),
        ));
        (root, pool, service)
    }

    #[tokio::test]
    async fn sample_goal_is_seeded_only_once_even_after_goals_are_deleted() {
        let (root, pool, service) = fixture().await;
        seed_sample_goal(&pool, &service).await.unwrap();
        assert!(marker_exists(&pool).await.unwrap());
        assert!(service.list_active().await.unwrap().is_empty());
        let seeded = service.list_all().await.unwrap();
        assert_eq!(seeded.len(), 1);
        assert_eq!(seeded[0].status, crate::models::types::GoalStatus::Paused);
        assert_eq!(service.delete_all().await.unwrap(), 1);
        seed_sample_goal(&pool, &service).await.unwrap();
        assert!(service.list_all().await.unwrap().is_empty());
        pool.close().await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
