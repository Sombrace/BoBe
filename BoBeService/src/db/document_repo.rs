use std::marker::PhantomData;

use chrono::Utc;
use sqlx::AssertSqlSafe;
use sqlx::sqlite::{SqliteConnection, SqliteExecutor, SqlitePool};
use tracing::info;

use crate::error::AppError;
use crate::models::document::{Document, DocumentKind};

pub(crate) struct SqliteDocumentRepo<K> {
    pool: SqlitePool,
    kind: PhantomData<K>,
}

impl<K: DocumentKind> SqliteDocumentRepo<K> {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            kind: PhantomData,
        }
    }

    pub(crate) async fn list(&self, enabled_only: bool) -> Result<Vec<Document<K>>, AppError> {
        let statement = if enabled_only {
            "SELECT * FROM {table} WHERE enabled = 1"
        } else {
            "SELECT * FROM {table}"
        };
        Ok(sqlx::query_as(sql::<K>(statement))
            .fetch_all(&self.pool)
            .await?)
    }

    pub(crate) async fn get_by_id(&self, id: K::Id) -> Result<Option<Document<K>>, AppError> {
        Ok(select_by_id::<K>(&self.pool, id).await?)
    }

    pub(crate) async fn get_by_name(&self, name: &str) -> Result<Option<Document<K>>, AppError> {
        Ok(select_by_name::<K>(&self.pool, name).await?)
    }

    pub(crate) async fn insert(&self, document: Document<K>) -> Result<Document<K>, AppError> {
        insert(&self.pool, &document).await?;
        Ok(document)
    }

    pub(crate) async fn edit(
        &self,
        id: K::Id,
        content: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<Option<Document<K>>, AppError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some(current) = select_by_id::<K>(&mut *transaction, id).await? else {
            return Ok(None);
        };
        let edited = match content {
            Some(content) if current.is_shipped_default() => {
                fork_shipped_default(&mut transaction, current, content, enabled).await?
            }
            _ => update::<K>(&mut *transaction, id, content, enabled).await?,
        };
        transaction.commit().await?;
        Ok(Some(edited))
    }

    pub(crate) async fn delete(&self, id: K::Id) -> Result<bool, AppError> {
        let result = sqlx::query(sql::<K>("DELETE FROM {table} WHERE id = ?1"))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub(crate) async fn refresh_shipped_default(&self) -> Result<(), AppError> {
        let Some(shipped) = K::SHIPPED_DEFAULT else {
            return Ok(());
        };
        match self.get_by_name(shipped.name).await? {
            None => {
                let document = Document::<K>::new(
                    shipped.name.to_owned(),
                    shipped.content.to_owned(),
                    true,
                    true,
                );
                insert(&self.pool, &document).await?;
                info!(
                    kind = K::TABLE,
                    name = shipped.name,
                    "document_repo.shipped_default_created"
                );
            }
            Some(current) if current.is_default && current.content != shipped.content => {
                update::<K>(&self.pool, current.id, Some(shipped.content), None).await?;
                info!(
                    kind = K::TABLE,
                    name = shipped.name,
                    "document_repo.shipped_default_refreshed"
                );
            }
            Some(_) => {}
        }
        Ok(())
    }

    pub(crate) async fn reset_to_shipped(
        connection: &mut SqliteConnection,
    ) -> Result<u64, AppError> {
        let Some(shipped) = K::SHIPPED_DEFAULT else {
            return Ok(sqlx::query(sql::<K>("DELETE FROM {table}"))
                .execute(connection)
                .await?
                .rows_affected());
        };
        let removed = sqlx::query(sql::<K>(
            "DELETE FROM {table} WHERE NOT (is_default = 1 AND name = ?1)",
        ))
        .bind(shipped.name)
        .execute(&mut *connection)
        .await?
        .rows_affected();
        if let Some(current) = select_by_name::<K>(&mut *connection, shipped.name).await? {
            update::<K>(
                &mut *connection,
                current.id,
                Some(shipped.content),
                Some(true),
            )
            .await?;
        } else {
            let document = Document::<K>::new(
                shipped.name.to_owned(),
                shipped.content.to_owned(),
                true,
                true,
            );
            insert(&mut *connection, &document).await?;
        }
        Ok(removed)
    }
}

fn sql<K: DocumentKind>(statement: &str) -> AssertSqlSafe<String> {
    AssertSqlSafe(statement.replace("{table}", K::TABLE))
}

async fn select_by_id<'c, K: DocumentKind>(
    executor: impl SqliteExecutor<'c>,
    id: K::Id,
) -> Result<Option<Document<K>>, sqlx::Error> {
    sqlx::query_as(sql::<K>("SELECT * FROM {table} WHERE id = ?1"))
        .bind(id)
        .fetch_optional(executor)
        .await
}

async fn select_by_name<'c, K: DocumentKind>(
    executor: impl SqliteExecutor<'c>,
    name: &str,
) -> Result<Option<Document<K>>, sqlx::Error> {
    sqlx::query_as(sql::<K>("SELECT * FROM {table} WHERE name = ?1"))
        .bind(name)
        .fetch_optional(executor)
        .await
}

async fn insert<'c, K: DocumentKind>(
    executor: impl SqliteExecutor<'c>,
    document: &Document<K>,
) -> Result<(), sqlx::Error> {
    sqlx::query(sql::<K>(
        "INSERT INTO {table} (id, name, content, enabled, is_default, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    ))
    .bind(document.id)
    .bind(&document.name)
    .bind(&document.content)
    .bind(document.enabled)
    .bind(document.is_default)
    .bind(document.created_at)
    .bind(document.updated_at)
    .execute(executor)
    .await?;
    Ok(())
}

async fn update<'c, K: DocumentKind>(
    executor: impl SqliteExecutor<'c>,
    id: K::Id,
    content: Option<&str>,
    enabled: Option<bool>,
) -> Result<Document<K>, sqlx::Error> {
    sqlx::query_as(sql::<K>(
        "UPDATE {table} SET content = COALESCE(?2, content), enabled = COALESCE(?3, enabled), \
         updated_at = ?4 WHERE id = ?1 RETURNING *",
    ))
    .bind(id)
    .bind(content)
    .bind(enabled)
    .bind(Utc::now())
    .fetch_one(executor)
    .await
}

async fn fork_shipped_default<K: DocumentKind>(
    connection: &mut SqliteConnection,
    shipped: Document<K>,
    content: &str,
    enabled: Option<bool>,
) -> Result<Document<K>, sqlx::Error> {
    let owner_name = unused_copy_name::<K>(connection, &shipped.name).await?;
    let owned: Document<K> = sqlx::query_as(sql::<K>(
        "UPDATE {table} SET name = ?2, content = ?3, enabled = COALESCE(?4, enabled), \
         is_default = 0, updated_at = ?5 WHERE id = ?1 RETURNING *",
    ))
    .bind(shipped.id)
    .bind(&owner_name)
    .bind(content)
    .bind(enabled)
    .bind(Utc::now())
    .fetch_one(&mut *connection)
    .await?;
    let restored = Document::<K>::new(shipped.name, shipped.content, false, true);
    insert(&mut *connection, &restored).await?;
    info!(
        kind = K::TABLE,
        id = %owned.id,
        name = %owned.name,
        "document_repo.shipped_default_forked"
    );
    Ok(owned)
}

async fn unused_copy_name<K: DocumentKind>(
    connection: &mut SqliteConnection,
    shipped_name: &str,
) -> Result<String, sqlx::Error> {
    let mut name = format!("{shipped_name} (edited)");
    let mut copy = 1;
    while sqlx::query_scalar::<_, bool>(sql::<K>(
        "SELECT EXISTS (SELECT 1 FROM {table} WHERE name = ?1)",
    ))
    .bind(&name)
    .fetch_one(&mut *connection)
    .await?
    {
        copy += 1;
        name = format!("{shipped_name} (edited {copy})");
    }
    Ok(name)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on precondition failures")]
mod tests {
    use super::*;
    use crate::db::test_helpers::in_memory_pool;
    use crate::models::soul::Souls;
    use crate::models::user_profile::UserProfiles;

    async fn empty_repo<K: DocumentKind>() -> SqliteDocumentRepo<K> {
        SqliteDocumentRepo::new(in_memory_pool().await)
    }

    fn document<K: DocumentKind>(name: &str, enabled: bool) -> Document<K> {
        Document::new(name.to_owned(), format!("# {name}"), enabled, false)
    }

    fn shipped_soul() -> &'static str {
        Souls::SHIPPED_DEFAULT
            .expect("souls ship a default")
            .content
    }

    async fn shipped_soul_row(repo: &SqliteDocumentRepo<Souls>) -> Document<Souls> {
        repo.get_by_name("default_soul")
            .await
            .expect("get_by_name")
            .expect("shipped default exists")
    }

    async fn stores_and_edits_in_its_table<K: DocumentKind>() {
        let repo = empty_repo::<K>().await;
        let on = repo.insert(document("on", true)).await.expect("insert on");
        repo.insert(document("off", false))
            .await
            .expect("insert off");

        let fetched = repo
            .get_by_id(on.id)
            .await
            .expect("get_by_id")
            .expect("document exists");
        assert_eq!(
            (fetched.name.as_str(), fetched.content.as_str()),
            ("on", "# on")
        );
        assert!(
            repo.get_by_name("missing")
                .await
                .expect("get_by_name")
                .is_none()
        );
        let enabled = repo.list(true).await.expect("list enabled");
        assert_eq!(
            enabled.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["on"]
        );
        assert_eq!(repo.list(false).await.expect("list all").len(), 2);

        let edited = repo
            .edit(on.id, Some("# new"), Some(false))
            .await
            .expect("edit")
            .expect("edited document");
        assert_eq!(edited.id, on.id);
        assert_eq!(edited.name, "on");
        assert_eq!(edited.content, "# new");
        assert!(!edited.enabled);
        assert!(
            repo.edit(K::new_id(), Some("# new"), None)
                .await
                .expect("edit missing")
                .is_none()
        );

        assert!(repo.delete(on.id).await.expect("delete existing"));
        assert!(!repo.delete(on.id).await.expect("delete missing"));
        assert!(repo.get_by_id(on.id).await.expect("get_by_id").is_none());
    }

    #[tokio::test]
    async fn souls_are_stored_and_edited_in_their_table() {
        stores_and_edits_in_its_table::<Souls>().await;
    }

    #[tokio::test]
    async fn user_profiles_are_stored_and_edited_in_their_table() {
        stores_and_edits_in_its_table::<UserProfiles>().await;
    }

    #[tokio::test]
    async fn editing_the_shipped_default_forks_an_owner_copy() {
        let repo = empty_repo::<Souls>().await;
        repo.refresh_shipped_default().await.expect("seed");
        let shipped = shipped_soul_row(&repo).await;

        let owned = repo
            .edit(shipped.id, Some("# mine"), None)
            .await
            .expect("edit")
            .expect("owner copy");
        assert_eq!(owned.id, shipped.id);
        assert_eq!(owned.name, "default_soul (edited)");
        assert_eq!(owned.content, "# mine");
        assert!(owned.enabled);
        assert!(!owned.is_default);

        let restored = shipped_soul_row(&repo).await;
        assert_ne!(restored.id, shipped.id);
        assert_eq!(restored.content, shipped_soul());
        assert!(restored.is_default);
        assert!(!restored.enabled);

        let second = repo
            .edit(restored.id, Some("# again"), Some(true))
            .await
            .expect("second edit")
            .expect("second owner copy");
        assert_eq!(second.name, "default_soul (edited 2)");
        assert!(second.enabled);
        assert!(shipped_soul_row(&repo).await.is_default);
        assert_eq!(repo.list(false).await.expect("list").len(), 3);
    }

    #[tokio::test]
    async fn toggling_the_shipped_default_keeps_it_in_place() {
        let repo = empty_repo::<Souls>().await;
        repo.refresh_shipped_default().await.expect("seed");
        let shipped = shipped_soul_row(&repo).await;

        let toggled = repo
            .edit(shipped.id, None, Some(false))
            .await
            .expect("toggle")
            .expect("toggled default");
        assert_eq!(toggled.name, "default_soul");
        assert!(toggled.is_default);
        assert!(!toggled.enabled);
        assert_eq!(repo.list(false).await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn start_refreshes_the_shipped_default_but_never_an_owner_document() {
        let repo = empty_repo::<Souls>().await;
        repo.refresh_shipped_default().await.expect("seed");
        let shipped = shipped_soul_row(&repo).await;
        sqlx::query("UPDATE souls SET content = 'older shipped text' WHERE id = ?1")
            .bind(shipped.id)
            .execute(&repo.pool)
            .await
            .expect("age the shipped text");
        repo.refresh_shipped_default().await.expect("refresh");
        let refreshed = shipped_soul_row(&repo).await;
        assert_eq!(refreshed.id, shipped.id);
        assert_eq!(refreshed.content, shipped_soul());

        let repo = empty_repo::<Souls>().await;
        repo.insert(Document::new(
            "default_soul".to_owned(),
            "# owner text".to_owned(),
            true,
            false,
        ))
        .await
        .expect("owner document");
        repo.refresh_shipped_default().await.expect("refresh");
        let kept = shipped_soul_row(&repo).await;
        assert_eq!(kept.content, "# owner text");
        assert!(!kept.is_default);
        assert_eq!(repo.list(false).await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn user_profiles_ship_no_default_and_edit_in_place() {
        let repo = empty_repo::<UserProfiles>().await;
        repo.refresh_shipped_default().await.expect("refresh");
        assert!(repo.list(false).await.expect("list").is_empty());

        let flagged = repo
            .insert(Document::new(
                "default".to_owned(),
                "# About me".to_owned(),
                true,
                true,
            ))
            .await
            .expect("insert");
        let edited = repo
            .edit(flagged.id, Some("# About me, edited"), None)
            .await
            .expect("edit")
            .expect("edited profile");
        assert_eq!(edited.id, flagged.id);
        assert_eq!(edited.name, "default");
        assert_eq!(edited.content, "# About me, edited");
        assert_eq!(repo.list(false).await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn purge_returns_an_edited_default_soul_to_its_shipped_state() {
        let repo = empty_repo::<Souls>().await;
        repo.refresh_shipped_default().await.expect("seed");
        let shipped = shipped_soul_row(&repo).await;
        repo.edit(shipped.id, Some("# My own soul"), None)
            .await
            .expect("edit")
            .expect("owner copy");
        repo.insert(document("custom", true))
            .await
            .expect("insert custom");
        sqlx::query("UPDATE souls SET content = 'older shipped text' WHERE is_default = 1")
            .execute(&repo.pool)
            .await
            .expect("age the shipped text");

        let mut transaction = repo.pool.begin().await.expect("begin");
        let removed = SqliteDocumentRepo::<Souls>::reset_to_shipped(&mut transaction)
            .await
            .expect("reset souls");
        transaction.commit().await.expect("commit");

        assert_eq!(removed, 2);
        let souls = repo.list(false).await.expect("list");
        assert_eq!(souls.len(), 1);
        assert_eq!(souls[0].name, "default_soul");
        assert!(souls[0].is_default);
        assert!(souls[0].enabled);
        assert_eq!(souls[0].content, shipped_soul());
    }

    #[tokio::test]
    async fn purge_leaves_no_user_profiles() {
        let repo = empty_repo::<UserProfiles>().await;
        repo.insert(document("ada", true)).await.expect("insert");

        let mut transaction = repo.pool.begin().await.expect("begin");
        let removed = SqliteDocumentRepo::<UserProfiles>::reset_to_shipped(&mut transaction)
            .await
            .expect("reset profiles");
        transaction.commit().await.expect("commit");

        assert_eq!(removed, 1);
        assert!(repo.list(false).await.expect("list").is_empty());
    }
}
