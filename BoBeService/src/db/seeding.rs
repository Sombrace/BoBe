use crate::db::SqliteSoulRepo;
use crate::error::AppError;

pub(crate) async fn seed_default_souls(repo: &SqliteSoulRepo) -> Result<(), AppError> {
    repo.refresh_shipped_default().await
}
