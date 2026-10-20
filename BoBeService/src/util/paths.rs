use std::path::{Component, Path, PathBuf};

use crate::error::AppError;

pub(crate) const AUTOMATIC_TLS_CERTIFICATE_FILE: &str = "server-cert.pem";
pub(crate) const AUTOMATIC_TLS_PRIVATE_KEY_FILE: &str = "server-key.pem";
pub(crate) const AUTOMATIC_TLS_METADATA_FILE: &str = "identity.json";

pub(crate) fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

#[derive(Clone, Debug)]
pub(crate) struct DataPaths {
    root: PathBuf,
    database_url: String,
}

impl DataPaths {
    pub(crate) fn resolve(root: impl AsRef<Path>, database_url: &str) -> Result<Self, AppError> {
        let root = absolute_lexical(root.as_ref())?;
        let database_url = normalize_sqlite_url(database_url)?;
        Ok(Self { root, database_url })
    }

    pub(crate) fn prepare_root(mut self) -> Result<Self, AppError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(&self.root)?;
            let metadata = std::fs::symlink_metadata(&self.root)?;
            if metadata.file_type().is_symlink() {
                return Err(AppError::Config(format!(
                    "data root must not be a symbolic link: {}",
                    self.root.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(AppError::Config(format!(
                    "data root is not a directory: {}",
                    self.root.display()
                )));
            }
            if metadata.permissions().mode() & 0o077 != 0 {
                std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
            }
            let metadata = std::fs::metadata(&self.root)?;
            if metadata.uid() != effective_uid() {
                return Err(AppError::Config(format!(
                    "data root is owned by uid {}, expected {}: {}",
                    metadata.uid(),
                    effective_uid(),
                    self.root.display()
                )));
            }
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&self.root)?;

        self.root = std::fs::canonicalize(&self.root)?;
        self.database_url = canonicalize_sqlite_parent(&self.database_url)?;
        Ok(self)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn database_url(&self) -> &str {
        &self.database_url
    }

    pub(crate) fn database_file(&self) -> Option<PathBuf> {
        sqlite_file_path(&self.database_url).map(PathBuf::from)
    }

    pub(crate) fn runtime_lock(&self) -> PathBuf {
        self.root.join("runtime.lock")
    }
    pub(crate) fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }
    pub(crate) fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub(crate) fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }
    pub(crate) fn agent_workspace_dir(&self) -> PathBuf {
        self.root.join("workspace")
    }
    pub(crate) fn secrets_file(&self) -> PathBuf {
        self.root.join("secrets.json")
    }
    pub(crate) fn headless_secrets_file(&self) -> PathBuf {
        self.root.join("headless-secrets.json")
    }
    pub(crate) fn owner_api_token_file(&self) -> PathBuf {
        self.root.join("owner-api-token")
    }
    pub(crate) fn admin_socket(&self) -> PathBuf {
        self.root.join("admin.sock")
    }
    pub(crate) fn tls_dir(&self) -> PathBuf {
        self.root.join("tls")
    }
    pub(crate) fn automatic_tls_certificate(&self) -> PathBuf {
        self.tls_dir().join(AUTOMATIC_TLS_CERTIFICATE_FILE)
    }
    pub(crate) fn automatic_tls_private_key(&self) -> PathBuf {
        self.tls_dir().join(AUTOMATIC_TLS_PRIVATE_KEY_FILE)
    }
    pub(crate) fn automatic_tls_metadata(&self) -> PathBuf {
        self.tls_dir().join(AUTOMATIC_TLS_METADATA_FILE)
    }
    pub(crate) fn agent_dir(&self) -> PathBuf {
        self.root.join("agent")
    }

    pub(crate) fn validate_agent_runtime_tree(&self) -> Result<(), AppError> {
        let path = self.agent_dir();
        validate_private_directory(&path, "agent runtime state root")?;
        validate_private_tree(&path, "agent runtime state")
    }

    pub(crate) fn prepare_skills_dir(&self) -> Result<PathBuf, AppError> {
        let path = self.skills_dir();
        prepare_private_directory(&path, "skill root")?;
        Ok(std::fs::canonicalize(path)?)
    }

    pub(crate) fn prepare_agent_workspace_dir(&self) -> Result<PathBuf, AppError> {
        let path = self.agent_workspace_dir();
        prepare_private_directory(&path, "agent workspace")?;
        Ok(std::fs::canonicalize(path)?)
    }
}

#[cfg(unix)]
pub(crate) fn prepare_private_directory(path: &Path, purpose: &str) -> Result<(), AppError> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(false).mode(0o700);
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    validate_private_directory(path, purpose)
}

#[cfg(unix)]
pub(crate) fn validate_private_directory(path: &Path, purpose: &str) -> Result<(), AppError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Config(format!(
            "{purpose} must be a regular directory: {}",
            path.display()
        )));
    }
    if metadata.uid() != effective_uid() {
        return Err(AppError::Config(format!(
            "{purpose} is owned by uid {}, expected {}: {}",
            metadata.uid(),
            effective_uid(),
            path.display()
        )));
    }
    if metadata.permissions().mode() & 0o777 != 0o700 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn prepare_private_directory(path: &Path, purpose: &str) -> Result<(), AppError> {
    std::fs::create_dir_all(path)?;
    validate_private_directory(path, purpose)
}

#[cfg(not(unix))]
pub(crate) fn validate_private_directory(path: &Path, purpose: &str) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Config(format!(
            "{purpose} must be a regular directory: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn validate_private_tree(root: &Path, purpose: &str) -> Result<(), AppError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Config(format!(
                "{purpose} must not contain symbolic links: {}",
                path.display()
            )));
        }
        if metadata.uid() != effective_uid() {
            return Err(AppError::Config(format!(
                "{purpose} entry is owned by uid {}, expected {}: {}",
                metadata.uid(),
                effective_uid(),
                path.display()
            )));
        }
        if metadata.is_dir() {
            if metadata.permissions().mode() & 0o777 != 0o700 {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            }
            validate_private_tree(&path, purpose)?;
        } else if metadata.is_file() {
            if metadata.permissions().mode() & 0o777 != 0o600 {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
        } else {
            return Err(AppError::Config(format!(
                "{purpose} contains a non-regular entry: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn validate_private_tree(root: &Path, purpose: &str) -> Result<(), AppError> {
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Config(format!(
                "{purpose} must not contain symbolic links: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            validate_private_tree(&path, purpose)?;
        } else if !metadata.is_file() {
            return Err(AppError::Config(format!(
                "{purpose} contains a non-regular entry: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(crate) fn bobe_data_dir() -> Result<PathBuf, AppError> {
    if let Ok(dir) = std::env::var("BOBE_DATA_DIR") {
        let path = PathBuf::from(dir);
        if !path.is_absolute() {
            return Err(AppError::Config(
                "BOBE_DATA_DIR must be an absolute path".into(),
            ));
        }
        return Ok(path);
    }
    std::env::home_dir()
        .map(|home| home.join(crate::constants::BOBE_DATA_DIR_NAME))
        .ok_or_else(|| AppError::Config("HOME is unavailable; set BOBE_DATA_DIR explicitly".into()))
}

pub(crate) fn home_dir() -> Result<PathBuf, AppError> {
    std::env::home_dir()
        .filter(|home| !home.as_os_str().is_empty())
        .ok_or_else(|| AppError::Config("HOME is unavailable".into()))
}

fn absolute_lexical(path: &Path) -> Result<PathBuf, AppError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::absolute(path)?
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn normalize_sqlite_url(database_url: &str) -> Result<String, AppError> {
    let Some(raw) = database_url.strip_prefix("sqlite:") else {
        return Ok(database_url.to_owned());
    };
    if raw.starts_with(":memory:") {
        return Ok(database_url.to_owned());
    }
    let (path, query) = raw
        .split_once('?')
        .map_or((raw, None), |(path, query)| (path, Some(query)));
    let absolute = absolute_lexical(&expand_tilde(path))?;
    Ok(match query {
        Some(query) => format!("sqlite:{}?{query}", absolute.display()),
        None => format!("sqlite:{}", absolute.display()),
    })
}

fn canonicalize_sqlite_parent(database_url: &str) -> Result<String, AppError> {
    let Some(raw) = database_url.strip_prefix("sqlite:") else {
        return Ok(database_url.to_owned());
    };
    if raw.starts_with(":memory:") {
        return Ok(database_url.to_owned());
    }
    let (path, query) = raw
        .split_once('?')
        .map_or((raw, None), |(path, query)| (path, Some(query)));
    let path = PathBuf::from(path);
    let parent = path.parent().ok_or_else(|| {
        AppError::Config(format!("database path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let file_name = path.file_name().ok_or_else(|| {
        AppError::Config(format!(
            "database path has no file name: {}",
            path.display()
        ))
    })?;
    let canonical_path = canonical_parent.join(file_name);
    Ok(match query {
        Some(query) => format!("sqlite:{}?{query}", canonical_path.display()),
        None => format!("sqlite:{}", canonical_path.display()),
    })
}

fn sqlite_file_path(database_url: &str) -> Option<&str> {
    let raw = database_url.strip_prefix("sqlite:")?;
    if raw.starts_with(":memory:") {
        return None;
    }
    Some(raw.split('?').next().unwrap_or(raw))
}

#[expect(unsafe_code, reason = "geteuid has no std wrapper")]
pub(crate) fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("bobe-paths-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn derives_every_runtime_path_from_one_root() {
        let root = root();
        let paths =
            DataPaths::resolve(&root, &format!("sqlite:{}/data/bobrust.db", root.display()))
                .expect("paths");
        assert_eq!(paths.runtime_lock(), root.join("runtime.lock"));
        assert_eq!(paths.models_dir(), root.join("models"));
        assert_eq!(paths.secrets_file(), root.join("secrets.json"));
        assert_eq!(
            paths.headless_secrets_file(),
            root.join("headless-secrets.json")
        );
        assert_eq!(paths.agent_dir(), root.join("agent"));
    }

    #[test]
    fn construction_is_side_effect_free() {
        let root = root();
        DataPaths::resolve(&root, &format!("sqlite:{}/data/bobrust.db", root.display()))
            .expect("paths");
        assert!(!root.exists());
    }

    #[test]
    fn database_url_in_the_home_directory_is_expanded() {
        let Some(home) = std::env::home_dir() else {
            return;
        };
        let paths = DataPaths::resolve(root(), "sqlite:~/bobe/data.db?mode=rwc").expect("paths");
        assert_eq!(
            paths.database_url(),
            format!("sqlite:{}?mode=rwc", home.join("bobe/data.db").display())
        );
        assert_eq!(paths.database_file(), Some(home.join("bobe/data.db")));
    }

    #[test]
    fn prepare_agent_root_creates_private_authority_directory() {
        use std::os::unix::fs::PermissionsExt;

        let root = root();
        let paths =
            DataPaths::resolve(&root, &format!("sqlite:{}/data/bobrust.db", root.display()))
                .expect("paths")
                .prepare_root()
                .expect("prepare root");

        crate::agent::state::prepare(&paths).expect("prepare agent root");
        let agent = paths.agent_dir();

        assert_eq!(agent, paths.root().join("agent"));
        assert_eq!(
            std::fs::metadata(&agent)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        drop(std::fs::remove_dir_all(paths.root()));
    }

    #[test]
    fn prepare_agent_root_tightens_nested_credentials() {
        use std::os::unix::fs::PermissionsExt;

        let root = root();
        let paths =
            DataPaths::resolve(&root, &format!("sqlite:{}/data/bobrust.db", root.display()))
                .expect("paths")
                .prepare_root()
                .expect("prepare root");
        let agent = paths.agent_dir();
        std::fs::create_dir(&agent).expect("agent root");
        let nested = agent.join("credentials.json");
        std::fs::write(&nested, b"{}").expect("credential file");
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o644))
            .expect("loose mode");

        crate::agent::state::prepare(&paths).expect("prepare agent root");

        assert_eq!(
            std::fs::metadata(&nested)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        drop(std::fs::remove_dir_all(paths.root()));
    }

    #[test]
    fn prepare_agent_root_rejects_nested_symlink() {
        use std::os::unix::fs::symlink;

        let root = root();
        let paths =
            DataPaths::resolve(&root, &format!("sqlite:{}/data/bobrust.db", root.display()))
                .expect("paths")
                .prepare_root()
                .expect("prepare root");
        let agent = paths.agent_dir();
        std::fs::create_dir(&agent).expect("agent root");
        symlink(paths.root(), agent.join("escape")).expect("symlink");

        let result = crate::agent::state::prepare(&paths);

        assert!(result.is_err());
        drop(std::fs::remove_dir_all(paths.root()));
    }

    #[test]
    fn prepare_creates_private_root() {
        let root = root();
        let paths =
            DataPaths::resolve(&root, &format!("sqlite:{}/data/bobrust.db", root.display()))
                .expect("paths")
                .prepare_root()
                .expect("prepare root");
        assert!(paths.root().is_dir());
        drop(std::fs::remove_dir_all(paths.root()));
    }
}
