use tracing::{debug, info};

use crate::error::AppError;
use crate::models::ids::GoalId;

use super::goal_md::GoalDoc;
use super::goals_service::GoalsService;

const SAMPLE_GOAL_SEEDED_MARKER: &str = "sample_goal_seeded_v1";
const STARTER_GOAL_ID: uuid::Uuid =
    uuid::Uuid::from_u128(0xb0be_5eed_60a1_4f00_8000_0000_0000_0001);

pub(crate) async fn seed_sample_goal(
    pool: &sqlx::SqlitePool,
    goals_service: &GoalsService,
) -> Result<(), AppError> {
    if marker_exists(pool).await? {
        debug!("goals_seeding.skipped_completed");
        return Ok(());
    }
    if publish_starter_goal(goals_service).await? {
        info!(goal_id = %starter_goal_id(), "goals_seeding.created");
    } else {
        info!(goal_id = %starter_goal_id(), "goals_seeding.reconciled");
    }
    sqlx::query("INSERT OR IGNORE INTO runtime_markers (key) VALUES (?1)")
        .bind(SAMPLE_GOAL_SEEDED_MARKER)
        .execute(pool)
        .await?;
    Ok(())
}

fn starter_goal_id() -> GoalId {
    GoalId::from(STARTER_GOAL_ID)
}

async fn publish_starter_goal(goals_service: &GoalsService) -> Result<bool, AppError> {
    let mut doc = GoalDoc::new(
        "Getting started with BoBe",
        "A starter goal so you can see how the goals system works. Edit any section, talk to BoBe about it, or delete this and write your own.",
    );
    doc.id = starter_goal_id();
    doc.priority = 1;
    doc.status = crate::models::types::GoalStatus::Paused;
    doc.why_it_matters = "Goals are how BoBe tracks what you actually care about across conversations. \
They're plain Markdown files under BoBe's configurable data root, so you can edit them in any editor and BoBe will pick up the changes."
        .to_string();
    doc.how_working_on_it =
        "Try editing this goal's sections in the Settings panel, or just chat with BoBe about something you want to work on."
            .to_string();
    goals_service.create_if_absent(doc).await
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
        let goals = goals_at(&root);
        (root, pool, goals)
    }

    fn goals_at(root: &std::path::Path) -> GoalsService {
        GoalsService::new(crate::services::goals::file_store::GoalFileStore::new(
            root.join("goals"),
        ))
    }

    #[tokio::test]
    async fn sample_goal_is_seeded_only_once_even_after_goals_are_deleted() {
        let (root, pool, service) = fixture().await;
        seed_sample_goal(&pool, &service).await.unwrap();
        assert!(marker_exists(&pool).await.unwrap());
        assert!(service.list_active().await.unwrap().is_empty());
        let seeded = service.list_all().await.unwrap();
        assert_eq!(seeded.len(), 1);
        assert_eq!(seeded[0].id, starter_goal_id());
        assert_eq!(seeded[0].status, crate::models::types::GoalStatus::Paused);
        assert_eq!(service.delete_all().await.unwrap(), 1);
        seed_sample_goal(&pool, &service).await.unwrap();
        assert!(service.list_all().await.unwrap().is_empty());
        pool.close().await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn a_restart_after_the_goal_was_published_records_the_marker_without_a_duplicate() {
        let (root, pool, service) = fixture().await;
        assert!(publish_starter_goal(&service).await.unwrap());
        assert!(!marker_exists(&pool).await.unwrap());
        service
            .update(
                starter_goal_id(),
                crate::services::goals::goals_service::GoalPatch {
                    title: Some("Edited before the marker".into()),
                    status: None,
                    priority: None,
                    summary: None,
                    why_it_matters: None,
                    how_working_on_it: None,
                    patterns_observed: None,
                    attitude_feelings: None,
                    open_questions: None,
                    notes: None,
                },
            )
            .await
            .unwrap();

        let restarted = goals_at(&root);
        seed_sample_goal(&pool, &restarted).await.unwrap();
        seed_sample_goal(&pool, &restarted).await.unwrap();

        assert!(marker_exists(&pool).await.unwrap());
        let goals = restarted.list_all().await.unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].id, starter_goal_id());
        assert_eq!(goals[0].title, "Edited before the marker");
        pool.close().await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn a_restart_before_the_goal_was_published_seeds_it() {
        let (root, pool, service) = fixture().await;
        assert!(!marker_exists(&pool).await.unwrap());
        assert!(service.list_all().await.unwrap().is_empty());

        seed_sample_goal(&pool, &service).await.unwrap();

        assert!(marker_exists(&pool).await.unwrap());
        let goals = service.list_all().await.unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].id, starter_goal_id());
        pool.close().await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn a_seeded_runtime_keeps_its_goal_and_does_not_restore_a_deleted_one() {
        let (root, pool, service) = fixture().await;
        seed_sample_goal(&pool, &service).await.unwrap();
        let seeded = service.list_all().await.unwrap();

        seed_sample_goal(&pool, &service).await.unwrap();
        assert_eq!(service.list_all().await.unwrap(), seeded);

        assert!(service.delete(starter_goal_id()).await.unwrap());
        seed_sample_goal(&pool, &service).await.unwrap();
        assert!(service.list_all().await.unwrap().is_empty());
        pool.close().await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
