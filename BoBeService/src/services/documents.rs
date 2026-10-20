use std::sync::Arc;

use tracing::info;

use crate::db::SqliteDocumentRepo;
use crate::error::AppError;
use crate::models::document::{Document, DocumentKind};
use crate::models::soul::Souls;
use crate::models::user_profile::UserProfiles;

const MIN_CONTENT_CHARS: usize = 10;

#[derive(Debug)]
pub(crate) enum DeleteOutcome {
    Deleted,
    NotFound,
}

pub(crate) type SoulsService = DocumentService<Souls>;
pub(crate) type UserProfileService = DocumentService<UserProfiles>;

pub(crate) struct DocumentService<K: DocumentKind> {
    repo: Arc<SqliteDocumentRepo<K>>,
}

impl<K: DocumentKind> DocumentService<K> {
    pub(crate) fn new(repo: Arc<SqliteDocumentRepo<K>>) -> Self {
        Self { repo }
    }

    pub(crate) async fn list(&self, enabled_only: bool) -> Result<K::Listing, AppError> {
        Ok(self.repo.list(enabled_only).await?.into())
    }

    pub(crate) async fn create(
        &self,
        name: String,
        content: String,
        enabled: bool,
    ) -> Result<Document<K>, AppError> {
        if name.is_empty() {
            return Err(AppError::Validation("name must not be empty".into()));
        }
        validate_content(&content)?;
        if self.repo.get_by_name(&name).await?.is_some() {
            return Err(AppError::Validation(format!(
                "{} with name '{name}' already exists",
                K::NOUN
            )));
        }
        let document = self
            .repo
            .insert(Document::new(name, content, enabled, false))
            .await?;
        info!(kind = K::TABLE, id = %document.id, name = %document.name, "documents.created");
        Ok(document)
    }

    pub(crate) async fn update(
        &self,
        id: K::Id,
        content: Option<String>,
        enabled: Option<bool>,
    ) -> Result<Option<Document<K>>, AppError> {
        if let Some(content) = content.as_deref() {
            validate_content(content)?;
        }
        let updated = self.repo.edit(id, content.as_deref(), enabled).await?;
        if updated.is_some() {
            info!(kind = K::TABLE, %id, "documents.updated");
        }
        Ok(updated)
    }

    pub(crate) async fn set_enabled(
        &self,
        id: K::Id,
        enabled: bool,
    ) -> Result<Option<Document<K>>, AppError> {
        let updated = self.repo.edit(id, None, Some(enabled)).await?;
        if updated.is_some() {
            info!(kind = K::TABLE, %id, enabled, "documents.enabled_changed");
        }
        Ok(updated)
    }

    pub(crate) async fn delete(&self, id: K::Id) -> Result<DeleteOutcome, AppError> {
        let Some(document) = self.repo.get_by_id(id).await? else {
            return Ok(DeleteOutcome::NotFound);
        };
        if document.is_shipped_default() {
            return Err(AppError::Validation(format!(
                "Cannot delete default {}. Disable it instead.",
                K::NOUN.to_lowercase()
            )));
        }
        if !self.repo.delete(id).await? {
            return Ok(DeleteOutcome::NotFound);
        }
        info!(kind = K::TABLE, %id, name = %document.name, "documents.deleted");
        Ok(DeleteOutcome::Deleted)
    }
}

fn validate_content(content: &str) -> Result<(), AppError> {
    if content.trim().chars().count() < MIN_CONTENT_CHARS {
        return Err(AppError::Validation(format!(
            "content must be at least {MIN_CONTENT_CHARS} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::in_memory_pool;

    async fn service<K: DocumentKind>() -> DocumentService<K> {
        DocumentService::new(Arc::new(SqliteDocumentRepo::new(in_memory_pool().await)))
    }

    fn validation_message(error: AppError) -> String {
        match error {
            AppError::Validation(message) => message,
            other => panic!("expected a validation error, got {other}"),
        }
    }

    #[tokio::test]
    async fn create_validates_input_and_names_the_kind_on_duplicates() {
        let souls = service::<Souls>().await;
        let empty_name = souls
            .create(String::new(), "# Long enough".into(), true)
            .await
            .expect_err("empty name refused");
        assert_eq!(validation_message(empty_name), "name must not be empty");
        let short = souls
            .create("brief".into(), "# Short".into(), true)
            .await
            .expect_err("short content refused");
        assert_eq!(
            validation_message(short),
            "content must be at least 10 characters"
        );

        let profiles = service::<UserProfiles>().await;
        let created = profiles
            .create("john".into(), "# About John".into(), false)
            .await
            .expect("create");
        assert!(!created.enabled);
        assert!(!created.is_default);
        let duplicate = profiles
            .create("john".into(), "# About John".into(), true)
            .await
            .expect_err("duplicate refused");
        assert_eq!(
            validation_message(duplicate),
            "User profile with name 'john' already exists"
        );
    }

    #[tokio::test]
    async fn content_length_counts_trimmed_characters_on_create_and_update() {
        let souls = service::<Souls>().await;
        for short in ["   # Short   \n", "日本語のテキストで", "\n\n\t         "] {
            let refused = souls
                .create(format!("short-{}", short.len()), short.into(), true)
                .await
                .expect_err("short content refused");
            assert_eq!(
                validation_message(refused),
                "content must be at least 10 characters"
            );
        }
        let created = souls
            .create("japanese".into(), "  日本語のテキストです  ".into(), true)
            .await
            .expect("ten characters are enough");

        for short in ["   # Short   ", "日本語のテキストで"] {
            let refused = souls
                .update(created.id, Some(short.into()), None)
                .await
                .expect_err("short update refused");
            assert_eq!(
                validation_message(refused),
                "content must be at least 10 characters"
            );
        }
        let disabled = souls
            .update(created.id, None, Some(false))
            .await
            .expect("toggle without content")
            .expect("stored soul");
        assert!(!disabled.enabled);
        assert_eq!(disabled.content, "  日本語のテキストです  ");
        let updated = souls
            .update(created.id, Some("ελληνικό κείμενο".into()), None)
            .await
            .expect("multibyte update")
            .expect("stored soul");
        assert_eq!(updated.content, "ελληνικό κείμενο");
    }

    #[tokio::test]
    async fn only_a_shipped_default_is_protected_from_deletion() {
        let souls = service::<Souls>().await;
        souls.repo.refresh_shipped_default().await.expect("seed");
        let shipped = souls
            .repo
            .get_by_name("default_soul")
            .await
            .expect("lookup")
            .expect("shipped default");
        let refused = souls
            .delete(shipped.id)
            .await
            .expect_err("shipped default kept");
        assert_eq!(
            validation_message(refused),
            "Cannot delete default soul. Disable it instead."
        );

        let owned = souls
            .update(shipped.id, Some("# My own soul".into()), None)
            .await
            .expect("fork")
            .expect("owner copy");
        assert!(matches!(
            souls.delete(owned.id).await.expect("delete owner copy"),
            DeleteOutcome::Deleted
        ));
        assert!(matches!(
            souls.delete(owned.id).await.expect("delete again"),
            DeleteOutcome::NotFound
        ));

        let profiles = service::<UserProfiles>().await;
        let flagged = profiles
            .repo
            .insert(Document::new(
                "default".into(),
                "# About me".into(),
                true,
                true,
            ))
            .await
            .expect("insert");
        assert!(matches!(
            profiles.delete(flagged.id).await.expect("delete"),
            DeleteOutcome::Deleted
        ));
    }
}
