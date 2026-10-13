#![allow(unsafe_code, reason = "flock and open flags require libc on Unix")]

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug)]
pub(crate) struct RuntimeLocks {
    _authority: RuntimeLock,
    _database: Option<RuntimeLock>,
}

impl RuntimeLocks {
    #[must_use = "dropping the returned guard releases the runtime locks"]
    pub(crate) fn acquire(paths: &crate::util::paths::DataPaths) -> Result<Self, AppError> {
        let authority_path = paths.runtime_lock();
        let authority = RuntimeLock::acquire(&authority_path, "authority")?;

        let database = match paths.database_file() {
            Some(database_path) => {
                let parent = database_path.parent().ok_or_else(|| {
                    AppError::Config(format!(
                        "database path has no parent: {}",
                        database_path.display()
                    ))
                })?;
                std::fs::create_dir_all(parent)?;
                Some(RuntimeLock::acquire(
                    &database_path.with_extension("bobe-lock"),
                    "database",
                )?)
            }
            None => None,
        };

        Ok(Self {
            _authority: authority,
            _database: database,
        })
    }

    #[must_use = "dropping the returned guard releases the runtime locks"]
    pub(crate) async fn acquire_after_service_stop(
        paths: &crate::util::paths::DataPaths,
        timeout: std::time::Duration,
    ) -> Result<Self, AppError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match Self::acquire(paths) {
                Ok(locks) => return Ok(locks),
                Err(AppError::Conflict(_)) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[derive(Debug)]
struct RuntimeLock {
    _file: File,
    _path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct HolderInfo {
    pid: u32,
    started_at: String,
    daemon_version: String,
    kind: String,
}

impl RuntimeLock {
    fn acquire(path: &Path, kind: &'static str) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;

        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            let code = error.raw_os_error();
            if code == Some(libc::EWOULDBLOCK) || code == Some(libc::EAGAIN) {
                let holder = read_holder(&mut file);
                return Err(AppError::Conflict(format!(
                    "another BoBe daemon owns the {kind} lock {}{}",
                    path.display(),
                    holder.map_or_else(String::new, |h| format!(" (pid {})", h.pid))
                )));
            }
            return Err(AppError::Io(error));
        }

        let holder = HolderInfo {
            pid: std::process::id(),
            started_at: chrono::Utc::now().to_rfc3339(),
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
            kind: kind.to_owned(),
        };
        file.set_len(0)?;
        file.rewind()?;
        serde_json::to_writer(&mut file, &holder)?;
        file.write_all(b"\n")?;
        file.sync_all()?;

        Ok(Self {
            _file: file,
            _path: path.to_path_buf(),
        })
    }
}

fn read_holder(file: &mut File) -> Option<HolderInfo> {
    file.rewind().ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;
    serde_json::from_str(&contents).ok()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn lock_path() -> PathBuf {
        std::env::temp_dir().join(format!("bobe-lock-{}.lock", uuid::Uuid::new_v4()))
    }

    #[test]
    fn shared_database_conflicts_across_distinct_authority_roots() {
        let base = std::env::temp_dir().join(format!("bobe-shared-db-{}", uuid::Uuid::new_v4()));
        let first_root = base.join("first");
        let second_root = base.join("second");
        let database = base.join("shared/bobe.db");
        let first_paths = crate::util::paths::DataPaths::resolve(
            &first_root,
            &format!("sqlite:{}", database.display()),
        )
        .expect("first paths")
        .prepare_root()
        .expect("first root");
        let second_paths = crate::util::paths::DataPaths::resolve(
            &second_root,
            &format!("sqlite:{}", database.display()),
        )
        .expect("second paths")
        .prepare_root()
        .expect("second root");

        let first = RuntimeLocks::acquire(&first_paths).expect("first locks");
        let error = RuntimeLocks::acquire(&second_paths).expect_err("shared database conflicts");
        assert!(matches!(error, AppError::Conflict(_)));
        drop(first);
        drop(std::fs::remove_dir_all(base));
    }

    #[tokio::test]
    async fn bounded_wait_acquires_after_holder_releases() {
        let base = std::env::temp_dir().join(format!("bobe-wait-lock-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &base,
            &format!("sqlite:{}", base.join("bobe.db").display()),
        )
        .expect("paths")
        .prepare_root()
        .expect("root");
        let first = RuntimeLocks::acquire(&paths).expect("first locks");
        let release = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            drop(first);
        });

        let second =
            RuntimeLocks::acquire_after_service_stop(&paths, std::time::Duration::from_secs(1))
                .await
                .expect("locks after delayed release");
        release.await.expect("release task");
        drop(second);
        drop(std::fs::remove_dir_all(base));
    }

    #[tokio::test]
    async fn bounded_wait_preserves_conflict_after_timeout() {
        let base = std::env::temp_dir().join(format!("bobe-timeout-lock-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &base,
            &format!("sqlite:{}", base.join("bobe.db").display()),
        )
        .expect("paths")
        .prepare_root()
        .expect("root");
        let first = RuntimeLocks::acquire(&paths).expect("first locks");

        let error =
            RuntimeLocks::acquire_after_service_stop(&paths, std::time::Duration::from_millis(50))
                .await
                .expect_err("held lock times out");
        assert!(matches!(error, AppError::Conflict(_)));
        drop(first);
        drop(std::fs::remove_dir_all(base));
    }

    #[test]
    fn second_acquire_conflicts_and_drop_releases() {
        let path = lock_path();
        let first = RuntimeLock::acquire(&path, "test").expect("first lock");
        let error = RuntimeLock::acquire(&path, "test").expect_err("second lock conflicts");
        assert!(matches!(error, AppError::Conflict(_)));
        drop(first);
        let second = RuntimeLock::acquire(&path, "test").expect("lock after drop");
        drop(second);
        drop(std::fs::remove_file(path));
    }
}
