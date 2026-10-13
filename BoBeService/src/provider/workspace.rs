use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

use crate::error::AppError;

const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 500;

pub(crate) struct AgentWorkspace {
    root: PathBuf,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct WorkspaceListParams {
    #[serde(default)]
    path: String,
    #[serde(default = "default_list_entries")]
    max_entries: usize,
}

impl WorkspaceListParams {
    pub(crate) fn new(path: String, max_entries: usize) -> Self {
        Self { path, max_entries }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct WorkspaceReadParams {
    path: String,
    #[serde(default = "default_read_bytes")]
    max_bytes: usize,
}

impl WorkspaceReadParams {
    pub(crate) fn new(path: String, max_bytes: usize) -> Self {
        Self { path, max_bytes }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct WorkspaceWriteParams {
    pub(crate) path: String,
    pub(crate) content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceEntry {
    path: String,
    kind: &'static str,
    bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceRead {
    path: String,
    content: String,
    truncated: bool,
}

impl AgentWorkspace {
    pub(crate) fn prepare(paths: &crate::util::paths::DataPaths) -> Result<Arc<Self>, AppError> {
        Ok(Arc::new(Self {
            root: paths.prepare_agent_workspace_dir()?,
        }))
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) async fn purge_files(&self) -> Result<usize, AppError> {
        let file_count = count_workspace_files(&self.root).await?;
        let mut entries = tokio::fs::read_dir(&self.root).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            if file_type.is_dir() && !file_type.is_symlink() {
                crate::util::durable_fs::durable_remove_dir_all(&entry.path()).await?;
            } else {
                crate::util::durable_fs::durable_remove_file(&entry.path()).await?;
            }
        }
        Ok(file_count)
    }

    pub(crate) async fn list(
        &self,
        params: WorkspaceListParams,
    ) -> Result<Vec<WorkspaceEntry>, AppError> {
        let maximum = params.max_entries.clamp(1, MAX_LIST_ENTRIES);
        let directory = self.resolve_existing(&params.path).await?;
        let metadata = tokio::fs::symlink_metadata(&directory).await?;
        if !metadata.is_dir() {
            return Err(AppError::Validation(format!(
                "workspace path is not a directory: {}",
                display_relative(&self.root, &directory)
            )));
        }
        let mut entries = tokio::fs::read_dir(&directory).await?;
        let mut result = Vec::new();
        while result.len() < maximum {
            let Some(entry) = entries.next_entry().await? else {
                break;
            };
            let file_type = entry.file_type().await?;
            let (kind, bytes) = if file_type.is_symlink() {
                ("symlink", None)
            } else if file_type.is_dir() {
                ("directory", None)
            } else if file_type.is_file() {
                ("file", Some(entry.metadata().await?.len()))
            } else {
                ("other", None)
            };
            result.push(WorkspaceEntry {
                path: display_relative(&self.root, &entry.path()),
                kind,
                bytes,
            });
        }
        result.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(result)
    }

    pub(crate) async fn read(
        &self,
        params: WorkspaceReadParams,
    ) -> Result<WorkspaceRead, AppError> {
        let maximum = params.max_bytes.clamp(1, MAX_FILE_BYTES);
        let path = self.resolve_existing(&params.path).await?;
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(AppError::Validation(
                "workspace reads require a regular file".into(),
            ));
        }
        let file = tokio::fs::File::open(&path).await?;
        let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
        file.take((maximum + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        let truncated = bytes.len() > maximum;
        bytes.truncate(maximum);
        let content = match std::str::from_utf8(&bytes) {
            Ok(content) => content.to_owned(),
            Err(error) if truncated && error.error_len().is_none() => {
                String::from_utf8(bytes[..error.valid_up_to()].to_vec())
                    .map_err(|_| AppError::Validation("workspace file is not UTF-8 text".into()))?
            }
            Err(_) => {
                return Err(AppError::Validation(
                    "workspace file is not UTF-8 text".into(),
                ));
            }
        };
        Ok(WorkspaceRead {
            path: display_relative(&self.root, &path),
            content,
            truncated,
        })
    }

    pub(crate) async fn write(
        &self,
        params: WorkspaceWriteParams,
    ) -> Result<WorkspaceEntry, AppError> {
        if params.content.len() > MAX_FILE_BYTES || params.content.contains('\0') {
            return Err(AppError::Validation(format!(
                "workspace text files must be at most {MAX_FILE_BYTES} bytes without NUL characters"
            )));
        }
        let path = self.resolve_for_write(&params.path).await?;
        if let Ok(metadata) = tokio::fs::symlink_metadata(&path).await
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(AppError::Validation(
                "workspace write target must be a regular file".into(),
            ));
        }
        crate::util::durable_fs::atomic_write(&path, params.content.as_bytes()).await?;
        Ok(WorkspaceEntry {
            path: display_relative(&self.root, &path),
            kind: "file",
            bytes: Some(params.content.len() as u64),
        })
    }

    async fn resolve_existing(&self, relative: &str) -> Result<PathBuf, AppError> {
        let path = join_relative(&self.root, relative)?;
        reject_symlink_components(&self.root, &path).await?;
        let canonical = tokio::fs::canonicalize(&path).await?;
        if !canonical.starts_with(&self.root) {
            return Err(AppError::Validation(
                "workspace path resolves outside the workspace".into(),
            ));
        }
        Ok(canonical)
    }

    async fn resolve_for_write(&self, relative: &str) -> Result<PathBuf, AppError> {
        let path = join_relative(&self.root, relative)?;
        let parent = path
            .parent()
            .ok_or_else(|| AppError::Validation("workspace path has no parent".into()))?;
        create_directories_no_symlinks(&self.root, parent).await?;
        reject_symlink_components(&self.root, parent).await?;
        Ok(path)
    }
}

fn join_relative(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(relative);
    if relative.is_absolute() {
        return Err(AppError::Validation(
            "workspace paths must be relative".into(),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(component) => normalized.push(component),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Validation(
                    "workspace paths must not contain parent traversal".into(),
                ));
            }
        }
    }
    Ok(root.join(normalized))
}

async fn reject_symlink_components(root: &Path, target: &Path) -> Result<(), AppError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| AppError::Validation("workspace path escapes the root".into()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = tokio::fs::symlink_metadata(&current).await?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Validation(format!(
                "workspace paths must not traverse symbolic links: {}",
                display_relative(root, &current)
            )));
        }
    }
    Ok(())
}

async fn create_directories_no_symlinks(root: &Path, target: &Path) -> Result<(), AppError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| AppError::Validation("workspace path escapes the root".into()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(AppError::Validation(format!(
                    "workspace parent is not a regular directory: {}",
                    display_relative(root, &current)
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match tokio::fs::create_dir(&current).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let metadata = tokio::fs::symlink_metadata(&current).await?;
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(AppError::Validation(format!(
                                "workspace parent is not a regular directory: {}",
                                display_relative(root, &current)
                            )));
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

async fn count_workspace_files(root: &Path) -> Result<usize, AppError> {
    let mut pending = vec![root.to_path_buf()];
    let mut count = 0;
    while let Some(directory) = pending.pop() {
        let mut entries = tokio::fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            if file_type.is_dir() && !file_type.is_symlink() {
                pending.push(entry.path());
            } else {
                count += 1;
            }
        }
    }
    Ok(count)
}

const fn default_list_entries() -> usize {
    100
}

const fn default_read_bytes() -> usize {
    256 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (PathBuf, Arc<AgentWorkspace>) {
        let root = std::env::temp_dir().join(format!("bobe-workspace-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let workspace = AgentWorkspace::prepare(&paths).unwrap();
        (root, workspace)
    }

    #[tokio::test]
    async fn owner_api_reads_and_writes_relative_workspace_files() {
        let (root, workspace) = workspace();
        workspace
            .write(WorkspaceWriteParams {
                path: "notes/plan.md".into(),
                content: "bounded".into(),
            })
            .await
            .unwrap();
        let read = workspace
            .read(WorkspaceReadParams {
                path: "notes/plan.md".into(),
                max_bytes: 128,
            })
            .await
            .unwrap();
        assert_eq!(read.content, "bounded");
        assert!(
            workspace
                .read(WorkspaceReadParams {
                    path: "../memory.md".into(),
                    max_bytes: 128,
                })
                .await
                .is_err()
        );
        drop(tokio::fs::remove_dir_all(root).await);
    }

    #[tokio::test]
    async fn purge_keeps_workspace_root() {
        let (root, workspace) = workspace();
        workspace
            .write(WorkspaceWriteParams {
                path: "one.md".into(),
                content: "one".into(),
            })
            .await
            .unwrap();
        assert_eq!(workspace.purge_files().await.unwrap(), 1);
        assert!(workspace.root().is_dir());
        drop(tokio::fs::remove_dir_all(root).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owner_api_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let (root, workspace) = workspace();
        symlink("/tmp", workspace.root().join("outside")).unwrap();
        assert!(
            workspace
                .write(WorkspaceWriteParams {
                    path: "outside/escaped.txt".into(),
                    content: "no".into(),
                })
                .await
                .is_err()
        );
        drop(tokio::fs::remove_dir_all(root).await);
    }
}
