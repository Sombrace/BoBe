use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::error::AppError;
use crate::models::ids::GoalId;

use super::goal_md::{GoalDoc, parse, to_md};

pub(crate) struct GoalFileStore {
    dir: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl GoalFileStore {
    pub(crate) fn new(dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            dir,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    fn path_for(&self, id: GoalId) -> PathBuf {
        self.dir.join(format!("{id}.md"))
    }

    pub(crate) async fn list(&self) -> Result<Vec<GoalDoc>, AppError> {
        let mut goals = Vec::new();
        let mut read_dir = match tokio::fs::read_dir(&self.dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(AppError::Io(e)),
        };

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            let Some(ext) = path.extension() else {
                continue;
            };
            if ext != "md" {
                continue;
            }

            match tokio::fs::read_to_string(&path).await {
                Ok(body) => match parse_for_path(&path, &body) {
                    Ok(doc) => goals.push(doc),
                    Err(e) => {
                        warn!(
                            path = %path.display(),
                            err = %e,
                            "goal_file_store.parse_failed"
                        );
                    }
                },
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        err = %e,
                        "goal_file_store.read_failed"
                    );
                }
            }
        }

        debug!(count = goals.len(), "goal_file_store.list");
        Ok(goals)
    }

    pub(crate) async fn get(&self, id: GoalId) -> Result<Option<GoalDoc>, AppError> {
        let path = self.path_for(id);
        let body = match tokio::fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AppError::Io(e)),
        };
        parse_checked(&path, &body, id).map(Some)
    }

    pub(crate) async fn update<F>(&self, id: GoalId, mutate: F) -> Result<Option<GoalDoc>, AppError>
    where
        F: FnOnce(GoalDoc) -> GoalDoc,
    {
        let _guard = self.write_lock.lock().await;
        let path = self.path_for(id);
        let body = match tokio::fs::read_to_string(&path).await {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(AppError::Io(error)),
        };
        let doc = parse_checked(&path, &body, id)?;
        let updated = mutate(doc);
        write_doc(&self.dir, &path, &updated).await?;
        Ok(Some(updated))
    }

    pub(crate) async fn save(&self, doc: &GoalDoc) -> Result<(), AppError> {
        let _guard = self.write_lock.lock().await;
        tokio::fs::create_dir_all(&self.dir).await?;

        let path = self.path_for(doc.id);
        write_doc(&self.dir, &path, doc).await?;
        debug!(goal_id = %doc.id, "goal_file_store.saved");
        Ok(())
    }

    pub(crate) async fn delete_all(&self) -> Result<usize, AppError> {
        let _guard = self.write_lock.lock().await;
        let mut removed = 0;
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(AppError::Io(error)),
        };
        while let Some(entry) = entries.next_entry().await? {
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "md")
                && crate::util::durable_fs::durable_remove_file(&entry.path()).await?
            {
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub(crate) async fn delete(&self, id: GoalId) -> Result<bool, AppError> {
        let _guard = self.write_lock.lock().await;
        let path = self.path_for(id);
        crate::util::durable_fs::durable_remove_file(&path).await
    }
}

fn parse_for_path(path: &std::path::Path, body: &str) -> Result<GoalDoc, AppError> {
    let doc = parse(body).map_err(|error| {
        AppError::Internal(format!(
            "could not parse goal file {}: {error}",
            path.display()
        ))
    })?;
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            AppError::Internal(format!(
                "goal file has invalid filename: {}",
                path.display()
            ))
        })?;
    let filename_id = stem.parse::<GoalId>().map_err(|error| {
        AppError::Internal(format!("goal filename is not an ID ({stem}): {error}"))
    })?;
    if doc.id != filename_id {
        return Err(AppError::Internal(format!(
            "goal filename ID {filename_id} does not match frontmatter ID {}",
            doc.id
        )));
    }
    Ok(doc)
}

fn parse_checked(
    path: &std::path::Path,
    body: &str,
    expected: GoalId,
) -> Result<GoalDoc, AppError> {
    let doc = parse_for_path(path, body)?;
    if doc.id != expected {
        return Err(AppError::Internal(format!(
            "goal request ID {expected} does not match frontmatter ID {}",
            doc.id
        )));
    }
    Ok(doc)
}

async fn write_doc(
    _dir: &std::path::Path,
    path: &std::path::Path,
    doc: &GoalDoc,
) -> Result<(), AppError> {
    crate::util::durable_fs::atomic_write(path, to_md(doc).as_bytes()).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests panic on precondition failures")]
mod tests {
    use super::*;
    use crate::models::types::GoalStatus;
    use uuid::Uuid;

    fn tempdir() -> PathBuf {
        let suffix: String = Uuid::new_v4()
            .as_simple()
            .to_string()
            .chars()
            .take(8)
            .collect();
        let d = PathBuf::from(format!("/tmp/bbg-{suffix}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn list_empty_dir_returns_empty() {
        let store = GoalFileStore::new(tempdir());
        let goals = store.list().await.unwrap();
        assert!(goals.is_empty());
    }

    #[tokio::test]
    async fn save_then_get_round_trips() {
        let store = GoalFileStore::new(tempdir());
        let doc = GoalDoc::new("Test goal", "A test summary.");
        let id = doc.id;
        store.save(&doc).await.unwrap();

        let loaded = store.get(id).await.unwrap().unwrap();
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.title, "Test goal");
        assert_eq!(loaded.summary, "A test summary.");
    }

    #[tokio::test]
    async fn list_returns_saved_goals() {
        let store = GoalFileStore::new(tempdir());
        store
            .save(&GoalDoc::new("Goal A", "summary a"))
            .await
            .unwrap();
        store
            .save(&GoalDoc::new("Goal B", "summary b"))
            .await
            .unwrap();
        let goals = store.list().await.unwrap();
        assert_eq!(goals.len(), 2);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let store = GoalFileStore::new(tempdir());
        let id = GoalId::new();
        assert!(store.get(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let store = GoalFileStore::new(tempdir());
        let id = GoalId::new();
        assert!(!store.delete(id).await.unwrap());

        let mut doc = GoalDoc::new("X", "Y");
        doc.id = id;
        store.save(&doc).await.unwrap();
        assert!(store.delete(id).await.unwrap());
        assert!(store.get(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_filters_non_md_files() {
        let dir = tempdir();
        tokio::fs::write(dir.join("notes.txt"), b"unrelated")
            .await
            .unwrap();

        let store = GoalFileStore::new(dir);
        store.save(&GoalDoc::new("Real goal", "A")).await.unwrap();
        let goals = store.list().await.unwrap();
        assert_eq!(goals.len(), 1);
    }

    #[tokio::test]
    async fn save_preserves_status_changes() {
        let store = GoalFileStore::new(tempdir());
        let mut doc = GoalDoc::new("Status test", "summary");
        let id = doc.id;
        store.save(&doc).await.unwrap();

        doc.status = GoalStatus::Paused;
        store.save(&doc).await.unwrap();

        let loaded = store.get(id).await.unwrap().unwrap();
        assert_eq!(loaded.status, GoalStatus::Paused);
    }
}
