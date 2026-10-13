use std::sync::Arc;

use chrono::Utc;
use tracing::info;

use crate::db::SqliteSoulRepo;
use crate::error::AppError;
use crate::models::ids::SoulId;
use crate::models::soul::Soul;
use crate::services::{DeleteOutcome, validate_markdown_doc_input};

const MIN_CONTENT_LEN: usize = 10;

pub(crate) struct SoulsService {
    repo: Arc<SqliteSoulRepo>,
}

pub(crate) struct SoulSummary {
    pub(crate) souls: Vec<Soul>,
    pub(crate) enabled_count: usize,
}

impl SoulsService {
    pub(crate) fn new(repo: Arc<SqliteSoulRepo>) -> Self {
        Self { repo }
    }

    pub(crate) async fn list(&self, enabled_only: bool) -> Result<SoulSummary, AppError> {
        let souls = if enabled_only {
            self.repo.find_enabled().await?
        } else {
            self.repo.get_all().await?
        };
        let enabled_count = souls.iter().filter(|s| s.enabled).count();
        Ok(SoulSummary {
            souls,
            enabled_count,
        })
    }

    pub(crate) async fn get(&self, id: SoulId) -> Result<Option<Soul>, AppError> {
        self.repo.get_by_id(id).await
    }

    pub(crate) async fn create(
        &self,
        name: String,
        content: String,
        enabled: bool,
    ) -> Result<Soul, AppError> {
        validate_markdown_doc_input(&name, &content, MIN_CONTENT_LEN)?;
        if self.repo.get_by_name(&name).await?.is_some() {
            return Err(AppError::Validation(format!(
                "Soul with name '{name}' already exists"
            )));
        }

        let mut soul = Soul::new(name, content, false);
        soul.enabled = enabled;
        let saved = self.repo.save(&soul).await?;

        info!(soul_id = %saved.id, name = %saved.name, "souls_service.created");
        Ok(saved)
    }

    pub(crate) async fn update(
        &self,
        id: SoulId,
        content: Option<String>,
        enabled: Option<bool>,
    ) -> Result<Option<Soul>, AppError> {
        let Some(soul) = self.repo.get_by_id(id).await? else {
            return Ok(None);
        };

        let is_content_edit_of_default = content.is_some() && soul.is_default;

        let updated = if is_content_edit_of_default {
            let original_name = soul.name.clone();
            let original_content = soul.content.clone();
            let edited_name = format!("{original_name} (edited)");

            let updated = self
                .repo
                .update(
                    id,
                    content.as_deref(),
                    enabled,
                    Some(false),
                    Some(&edited_name),
                )
                .await?;

            let default_copy = Soul {
                id: SoulId::new(),
                name: original_name,
                content: original_content,
                enabled: false,
                is_default: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            self.repo.save(&default_copy).await?;

            info!(
                soul_id = %id,
                edited_name = %edited_name,
                "souls_service.copy_on_write",
            );
            updated
        } else {
            self.repo
                .update(id, content.as_deref(), enabled, None, None)
                .await?
        };

        if updated.is_some() {
            info!(soul_id = %id, "souls_service.updated");
        }
        Ok(updated)
    }

    pub(crate) async fn set_enabled(
        &self,
        id: SoulId,
        enabled: bool,
    ) -> Result<Option<Soul>, AppError> {
        let updated = self
            .repo
            .update(id, None, Some(enabled), None, None)
            .await?;
        if updated.is_some() {
            if enabled {
                info!(soul_id = %id, "souls_service.enabled");
            } else {
                info!(soul_id = %id, "souls_service.disabled");
            }
        }
        Ok(updated)
    }

    pub(crate) async fn delete(&self, id: SoulId) -> Result<DeleteOutcome, AppError> {
        let Some(soul) = self.repo.get_by_id(id).await? else {
            return Ok(DeleteOutcome::NotFound);
        };
        if soul.is_default {
            return Err(AppError::Validation(
                "Cannot delete default soul. Disable it instead.".into(),
            ));
        }
        let removed = self.repo.delete(id).await?;
        if removed {
            info!(soul_id = %id, name = %soul.name, "souls_service.deleted");
            Ok(DeleteOutcome::Deleted)
        } else {
            Ok(DeleteOutcome::NotFound)
        }
    }
}
