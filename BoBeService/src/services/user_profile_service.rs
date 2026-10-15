use std::sync::Arc;

use tracing::info;

use crate::db::SqliteUserProfileRepo;
use crate::error::AppError;
use crate::models::ids::UserProfileId;
use crate::models::user_profile::UserProfile;
use crate::services::{DeleteOutcome, validate_markdown_doc_input};

const MIN_CONTENT_LEN: usize = 10;

pub(crate) struct UserProfileService {
    repo: Arc<SqliteUserProfileRepo>,
}

pub(crate) struct UserProfileSummary {
    pub(crate) profiles: Vec<UserProfile>,
    pub(crate) enabled_count: usize,
}

impl UserProfileService {
    pub(crate) fn new(repo: Arc<SqliteUserProfileRepo>) -> Self {
        Self { repo }
    }

    pub(crate) async fn list(&self, enabled_only: bool) -> Result<UserProfileSummary, AppError> {
        let profiles = if enabled_only {
            self.repo.find_enabled().await?
        } else {
            self.repo.get_all().await?
        };
        let enabled_count = profiles.iter().filter(|p| p.enabled).count();
        Ok(UserProfileSummary {
            profiles,
            enabled_count,
        })
    }

    pub(crate) async fn create(
        &self,
        name: String,
        content: String,
        enabled: bool,
    ) -> Result<UserProfile, AppError> {
        validate_markdown_doc_input(&name, &content, MIN_CONTENT_LEN)?;
        if self.repo.get_by_name(&name).await?.is_some() {
            return Err(AppError::Validation(format!(
                "User profile with name '{name}' already exists"
            )));
        }

        let mut profile = UserProfile::new(name, content, false);
        profile.enabled = enabled;
        let saved = self.repo.save(&profile).await?;

        info!(profile_id = %saved.id, name = %saved.name, "user_profile_service.created");
        Ok(saved)
    }

    pub(crate) async fn update(
        &self,
        id: UserProfileId,
        content: Option<String>,
        enabled: Option<bool>,
    ) -> Result<Option<UserProfile>, AppError> {
        if self.repo.get_by_id(id).await?.is_none() {
            return Ok(None);
        }
        let updated = self.repo.update(id, content.as_deref(), enabled).await?;
        if updated.is_some() {
            info!(profile_id = %id, "user_profile_service.updated");
        }
        Ok(updated)
    }

    pub(crate) async fn set_enabled(
        &self,
        id: UserProfileId,
        enabled: bool,
    ) -> Result<Option<UserProfile>, AppError> {
        let updated = self.repo.update(id, None, Some(enabled)).await?;
        if updated.is_some() {
            if enabled {
                info!(profile_id = %id, "user_profile_service.enabled");
            } else {
                info!(profile_id = %id, "user_profile_service.disabled");
            }
        }
        Ok(updated)
    }

    pub(crate) async fn delete(&self, id: UserProfileId) -> Result<DeleteOutcome, AppError> {
        let Some(profile) = self.repo.get_by_id(id).await? else {
            return Ok(DeleteOutcome::NotFound);
        };
        if profile.is_default {
            return Err(AppError::Validation(
                "Cannot delete default user profile. Disable it instead.".into(),
            ));
        }
        let removed = self.repo.delete(id).await?;
        if removed {
            info!(profile_id = %id, name = %profile.name, "user_profile_service.deleted");
            Ok(DeleteOutcome::Deleted)
        } else {
            Ok(DeleteOutcome::NotFound)
        }
    }
}
