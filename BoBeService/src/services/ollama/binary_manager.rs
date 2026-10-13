use super::binary_download as download;
use super::binary_extract as extract;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use tokio::sync::watch;
use tracing::info;

use crate::error::AppError;

#[derive(Debug, Clone, Default)]
pub(crate) struct DownloadProgress {
    pub(crate) current_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) percent: Option<u8>,
}

pub(crate) struct BinaryManager {
    data_dir: PathBuf,
    http_client: Arc<reqwest::Client>,
}

impl BinaryManager {
    pub(crate) fn new(data_dir: &Path, http_client: Arc<reqwest::Client>) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            http_client,
        }
    }

    pub(crate) fn find_managed_ollama(&self) -> Option<PathBuf> {
        let managed = self.managed_binary_path();
        if managed.exists() {
            info!(path = %managed.display(), "binary_manager.found_managed");
            return Some(managed);
        }
        None
    }

    pub(crate) async fn ensure_managed_ollama(
        &self,
        progress_tx: &watch::Sender<DownloadProgress>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<PathBuf, AppError> {
        if let Some(path) = self.find_managed_ollama() {
            progress_tx
                .send(DownloadProgress {
                    current_bytes: 0,
                    total_bytes: None,
                    percent: Some(100),
                })
                .ok();
            return Ok(path);
        }

        let target_path = self.managed_binary_path();
        let parent = target_path
            .parent()
            .ok_or_else(|| AppError::Config("Invalid binary path".into()))?;
        std::fs::create_dir_all(parent)?;

        let artifact =
            download::current_artifact().map_err(|message| AppError::Config(message.into()))?;
        let archive_path = self.data_dir.join("ollama").join(artifact.archive_name);

        progress_tx
            .send(DownloadProgress {
                current_bytes: 0,
                total_bytes: None,
                percent: Some(0),
            })
            .ok();

        download::download_ollama(
            &self.http_client,
            &archive_path,
            cancel_rx,
            |current, total| {
                let percent = total.map(|t| {
                    if t > 0 {
                        ((current as f64 / t as f64) * 90.0).min(100.0) as u8
                    } else {
                        0
                    }
                });
                progress_tx
                    .send(DownloadProgress {
                        current_bytes: current,
                        total_bytes: total,
                        percent,
                    })
                    .ok();
            },
        )
        .await?;

        progress_tx
            .send(DownloadProgress {
                current_bytes: 0,
                total_bytes: None,
                percent: Some(92),
            })
            .ok();

        if let Err(error) =
            extract::extract_ollama_archive(&archive_path, &target_path, artifact.format)
        {
            let _ignored = std::fs::remove_file(&archive_path);
            let _ignored = std::fs::remove_file(&target_path);
            return Err(error);
        }
        let _ignored = std::fs::remove_file(&archive_path);

        progress_tx
            .send(DownloadProgress {
                current_bytes: 0,
                total_bytes: None,
                percent: Some(100),
            })
            .ok();

        info!(path = %target_path.display(), "binary_manager.installed");
        Ok(target_path)
    }

    pub(crate) async fn validate_ollama_binary(&self, path: &Path) -> Result<(), AppError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| AppError::Config(format!("Managed Ollama binary missing: {e}")))?;

        if !metadata.is_file() {
            return Err(AppError::Config(format!(
                "Managed Ollama path is not a file: {}",
                path.display()
            )));
        }

        #[cfg(unix)]
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(AppError::Config(format!(
                "Managed Ollama binary is not executable: {}",
                path.display()
            )));
        }

        let output = tokio::process::Command::new(path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Config(format!("Failed to invoke managed Ollama: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Config(format!(
                "Managed Ollama failed validation (status {}): {}",
                output.status,
                stderr.trim()
            )));
        }

        Ok(())
    }

    pub(crate) fn managed_binary_path(&self) -> PathBuf {
        self.data_dir.join("ollama").join("bin").join("ollama")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
    use super::BinaryManager;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bobe-binary-manager-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn test_client() -> Arc<reqwest::Client> {
        crate::util::tls::install_crypto_provider().expect("TLS provider should install");
        Arc::new(reqwest::Client::new())
    }

    #[tokio::test]
    async fn validate_ollama_binary_accepts_executable() {
        let dir = temp_dir("valid");
        let binary_path = dir.join("ollama");
        std::fs::write(
            &binary_path,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'ollama version test'\n  exit 0\nfi\nexit 1\n",
        )
        .expect("test binary should be written");
        #[cfg(unix)]
        {
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&binary_path, perms).expect("permissions should be set");
        }

        let manager = BinaryManager::new(&dir, test_client());
        manager
            .validate_ollama_binary(&binary_path)
            .await
            .expect("validation should pass");
    }

    #[tokio::test]
    async fn validate_ollama_binary_rejects_non_executable_file() {
        let dir = temp_dir("non-executable");
        let binary_path = dir.join("ollama");
        std::fs::write(&binary_path, "not executable").expect("test file should be written");
        #[cfg(unix)]
        {
            let perms = std::fs::Permissions::from_mode(0o644);
            std::fs::set_permissions(&binary_path, perms).expect("permissions should be set");
        }

        let manager = BinaryManager::new(&dir, test_client());
        let error = manager
            .validate_ollama_binary(&binary_path)
            .await
            .expect_err("validation should fail");

        assert!(error.to_string().contains("not executable"));
    }

    #[test]
    fn find_managed_ollama_returns_none_for_fresh_data_dir() {
        let dir = temp_dir("find-absent");
        let manager = BinaryManager::new(&dir, test_client());
        assert_eq!(manager.find_managed_ollama(), None);
    }

    #[test]
    fn find_managed_ollama_returns_path_when_binary_exists() {
        let dir = temp_dir("find-present");
        let bin_dir = dir.join("ollama").join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir should be created");
        std::fs::write(bin_dir.join("ollama"), "fake").expect("fake binary should be written");

        let manager = BinaryManager::new(&dir, test_client());
        let result = manager.find_managed_ollama();
        assert!(result.is_some());
        assert_eq!(
            result.expect("file should exist after write"),
            dir.join("ollama").join("bin").join("ollama")
        );
    }
}
