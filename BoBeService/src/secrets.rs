#[cfg(not(unix))]
compile_error!("BoBe daemon supports only macOS and Unix-family OSes (Linux, *BSD).");

use std::sync::{Arc, OnceLock};

#[derive(Debug, thiserror::Error)]
pub(crate) enum SecretError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("Lock poisoned: {0}")]
    Lock(String),

    #[error("{0}")]
    Backend(String),
}

pub(crate) trait SecretStore: Send + Sync {
    fn read(&self, account: &str) -> Option<String>;
    fn store(&self, account: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

static DEFAULT_STORE: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();
static HEADLESS_STORE: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();

pub(crate) fn default_secret_store(
    paths: &crate::util::paths::DataPaths,
) -> Result<Arc<dyn SecretStore>, SecretError> {
    if let Some(store) = DEFAULT_STORE.get() {
        return Ok(Arc::clone(store));
    }
    let store: Arc<dyn SecretStore> =
        Arc::new(file::FileSecretStore::at_path(paths.secrets_file())?);
    Ok(Arc::clone(DEFAULT_STORE.get_or_init(|| store)))
}

pub(crate) fn headless_secret_store(
    paths: &crate::util::paths::DataPaths,
) -> Result<Arc<dyn SecretStore>, SecretError> {
    if let Some(store) = HEADLESS_STORE.get() {
        return Ok(Arc::clone(store));
    }
    let store: Arc<dyn SecretStore> = Arc::new(file::FileSecretStore::at_path(
        paths.headless_secrets_file(),
    )?);
    Ok(Arc::clone(HEADLESS_STORE.get_or_init(|| store)))
}

pub(crate) fn read_secret(account: &str) -> Option<String> {
    DEFAULT_STORE.get()?.read(account)
}

#[cfg(test)]
pub(crate) mod memory {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{SecretError, SecretStore};

    pub(crate) struct MemorySecretStore {
        values: Mutex<HashMap<String, String>>,
        writes_before_failure: AtomicUsize,
    }

    impl MemorySecretStore {
        pub(crate) fn holding<'a>(values: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
            Self {
                values: Mutex::new(
                    values
                        .into_iter()
                        .map(|(account, value)| (account.to_owned(), value.to_owned()))
                        .collect(),
                ),
                writes_before_failure: AtomicUsize::new(usize::MAX),
            }
        }

        pub(crate) fn failing_after(self, writes: usize) -> Self {
            self.writes_before_failure.store(writes, Ordering::SeqCst);
            self
        }

        fn write(&self) -> Result<(), SecretError> {
            self.writes_before_failure
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .map(drop)
                .map_err(|_| SecretError::Backend("injected secret store failure".into()))
        }
    }

    impl SecretStore for MemorySecretStore {
        fn read(&self, account: &str) -> Option<String> {
            self.values.lock().ok()?.get(account).cloned()
        }

        fn store(&self, account: &str, value: &str) -> Result<(), SecretError> {
            self.write()?;
            self.values
                .lock()
                .map_err(|error| SecretError::Lock(error.to_string()))?
                .insert(account.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), SecretError> {
            self.write()?;
            self.values
                .lock()
                .map_err(|error| SecretError::Lock(error.to_string()))?
                .remove(account);
            Ok(())
        }
    }
}

#[cfg(unix)]
pub(crate) mod file {
    use std::collections::HashMap;
    use std::fs::OpenOptions;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use tracing::{info, warn};

    pub(crate) struct FileSecretStore {
        path: PathBuf,
        cache: Mutex<HashMap<String, String>>,
    }

    impl FileSecretStore {
        pub(crate) fn at_path(path: PathBuf) -> Result<Self, super::SecretError> {
            let cache = load(&path)?;
            Ok(Self {
                path,
                cache: Mutex::new(cache),
            })
        }

        fn flush(&self, map: &HashMap<String, String>) -> Result<(), super::SecretError> {
            write_atomic(&self.path, map)
        }

        fn replace(
            &self,
            cache: &mut HashMap<String, String>,
            updated: HashMap<String, String>,
        ) -> Result<(), super::SecretError> {
            if let Err(error) = self.flush(&updated) {
                if let Ok(current) = load(&self.path) {
                    *cache = current;
                }
                return Err(error);
            }
            *cache = updated;
            Ok(())
        }
    }

    fn write_atomic(path: &Path, map: &HashMap<String, String>) -> Result<(), super::SecretError> {
        let bytes = serde_json::to_vec_pretty(map)?;
        crate::util::durable_fs::atomic_write_sync_mode(path, &bytes, 0o600)
            .map_err(|error| super::SecretError::Backend(error.to_string()))?;
        Ok(())
    }

    fn load(path: &Path) -> Result<HashMap<String, String>, super::SecretError> {
        use std::io::Read;

        let entry = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(HashMap::new());
            }
            Err(error) => return Err(error.into()),
        };
        if entry.file_type().is_symlink() || !entry.is_file() {
            return Err(super::SecretError::Backend(format!(
                "secret store must be a regular file: {}",
                path.display()
            )));
        }

        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(super::SecretError::Backend(format!(
                "secret store must be a regular file: {}",
                path.display()
            )));
        }
        let expected_uid = crate::util::paths::effective_uid();
        if metadata.uid() != expected_uid {
            return Err(super::SecretError::Backend(format!(
                "secret store is owned by uid {}, expected {}: {}",
                metadata.uid(),
                expected_uid,
                path.display()
            )));
        }
        if metadata.permissions().mode() & 0o777 != 0o600 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    impl super::SecretStore for FileSecretStore {
        fn read(&self, account: &str) -> Option<String> {
            self.cache.lock().ok()?.get(account).cloned()
        }

        fn store(&self, account: &str, value: &str) -> Result<(), super::SecretError> {
            let mut guard = self
                .cache
                .lock()
                .map_err(|e| super::SecretError::Lock(e.to_string()))?;
            let mut updated = load(&self.path)?;
            if value.is_empty() {
                updated.remove(account);
            } else {
                updated.insert(account.to_owned(), value.to_owned());
            }
            self.replace(&mut guard, updated)?;
            info!(account, "secrets.stored");
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), super::SecretError> {
            let mut guard = self
                .cache
                .lock()
                .map_err(|e| super::SecretError::Lock(e.to_string()))?;
            let mut updated = load(&self.path)?;
            let removed = updated.remove(account).is_some();
            self.replace(&mut guard, updated)?;
            if !removed {
                warn!(account, "secrets.delete_missing");
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::secrets::SecretStore;
        use std::assert_matches;

        fn tmpfile() -> PathBuf {
            std::env::temp_dir().join(format!("bobe-secrets-{}.json", uuid::Uuid::new_v4()))
        }

        #[test]
        fn store_then_read_back() {
            let path = tmpfile();
            let store = FileSecretStore::at_path(path.clone()).unwrap();
            store.store("acct", "hunter2").unwrap();
            let map = load(&path).unwrap();
            assert_eq!(map.get("acct").map(String::as_str), Some("hunter2"));
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn empty_value_deletes() {
            let path = tmpfile();
            let store = FileSecretStore::at_path(path.clone()).unwrap();
            store.store("acct", "x").unwrap();
            store.store("acct", "").unwrap();
            let map = load(&path).unwrap_or_default();
            assert!(!map.contains_key("acct"), "empty store should drop the key");
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn delete_clears_entry() {
            let path = tmpfile();
            let store = FileSecretStore::at_path(path.clone()).unwrap();
            store.store("acct", "x").unwrap();
            store.delete("acct").unwrap();
            let map = load(&path).unwrap_or_default();
            assert!(!map.contains_key("acct"));
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn file_mode_is_0600() {
            use std::os::unix::fs::PermissionsExt;
            let path = tmpfile();
            let store = FileSecretStore::at_path(path.clone()).unwrap();
            store.store("acct", "x").unwrap();
            let perms = std::fs::metadata(&path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn existing_file_mode_is_tightened_before_read() {
            use std::os::unix::fs::PermissionsExt;

            let path = tmpfile();
            std::fs::write(&path, b"{\"acct\":\"x\"}").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

            let store = FileSecretStore::at_path(path.clone()).unwrap();

            assert_eq!(store.read("acct").as_deref(), Some("x"));
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn restart_reads_persisted_value() {
            let path = tmpfile();
            FileSecretStore::at_path(path.clone())
                .unwrap()
                .store("acct", "persisted")
                .unwrap();

            let reopened = FileSecretStore::at_path(path.clone()).unwrap();

            assert_eq!(reopened.read("acct").as_deref(), Some("persisted"));
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn symlink_is_rejected_without_reading_target() {
            use std::os::unix::fs::symlink;

            let target = tmpfile();
            let link = tmpfile();
            std::fs::write(&target, b"{\"acct\":\"secret\"}").unwrap();
            symlink(&target, &link).unwrap();

            let result = FileSecretStore::at_path(link.clone());

            assert!(matches!(result, Err(super::super::SecretError::Backend(_))));
            assert_eq!(std::fs::read(&target).unwrap(), b"{\"acct\":\"secret\"}");
            drop(std::fs::remove_file(&link));
            drop(std::fs::remove_file(&target));
        }

        #[test]
        fn non_regular_path_is_rejected() {
            let path = tmpfile();
            std::fs::create_dir(&path).unwrap();

            let result = FileSecretStore::at_path(path.clone());

            assert!(matches!(result, Err(super::super::SecretError::Backend(_))));
            drop(std::fs::remove_dir(&path));
        }

        #[test]
        fn missing_file_loads_empty() {
            let path = tmpfile();
            let store = FileSecretStore::at_path(path).unwrap();
            assert!(store.read("anything").is_none());
        }

        #[test]
        fn corrupt_file_fails_without_overwriting_bytes() {
            let path = tmpfile();
            let corrupt = b"{\"provider\":\"still recoverable\"";
            std::fs::write(&path, corrupt).unwrap();

            let result = FileSecretStore::at_path(path.clone());

            assert!(matches!(
                result,
                Err(super::super::SecretError::Serialize(_))
            ));
            assert_eq!(std::fs::read(&path).unwrap(), corrupt);
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn corruption_after_open_blocks_mutation_without_overwriting_bytes() {
            let path = tmpfile();
            let store = FileSecretStore::at_path(path.clone()).unwrap();
            store.store("acct", "recoverable").unwrap();
            let corrupt = b"{\"acct\":\"recoverable\"";
            std::fs::write(&path, corrupt).unwrap();

            let result = store.store("other", "new");

            assert_matches!(result, Err(super::super::SecretError::Serialize(_)));
            assert_eq!(std::fs::read(&path).unwrap(), corrupt);
            assert_eq!(store.read("acct").as_deref(), Some("recoverable"));
            drop(std::fs::remove_file(&path));
        }

        #[test]
        fn read_through_trait_returns_stored_value() {
            let path = tmpfile();
            let store: Box<dyn SecretStore> =
                Box::new(FileSecretStore::at_path(path.clone()).unwrap());
            store.store("acct", "hunter2").unwrap();
            assert_eq!(store.read("acct").as_deref(), Some("hunter2"));
            assert!(store.read("nope").is_none());
            drop(std::fs::remove_file(&path));
        }
    }
}
