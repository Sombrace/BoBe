use super::binary_download as download;
use super::binary_extract as extract;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::error::AppError;

const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const VERSION_STDERR_LIMIT: u64 = 64 * 1024;

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

    fn find_installation(&self, artifact: &download::OllamaArtifact) -> Option<PathBuf> {
        let install_dir = self.install_dir(artifact);
        if extract::verify_runtime(&install_dir, artifact).is_err() {
            return None;
        }
        let managed = self.managed_binary_path(artifact);
        info!(path = %managed.display(), "binary_manager.found_managed");
        Some(managed)
    }

    pub(crate) fn installed_binary(&self) -> Option<PathBuf> {
        let artifact = download::current_artifact().ok()?;
        self.find_installation(&artifact)
    }

    pub(crate) fn previous_installation(
        &self,
        current: &download::OllamaArtifact,
    ) -> Option<PathBuf> {
        let current_dir = self.install_dir(current);
        std::fs::read_dir(self.runtime_root())
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path == current_dir {
                    return None;
                }
                let release = release_number(entry.file_name().to_str()?.strip_prefix('v')?)?;
                extract::verify_runtime(&path, current).ok()?;
                Some((release, path.join(current.binary)))
            })
            .max_by(|(left, _), (right, _)| left.cmp(right))
            .map(|(_, binary)| binary)
    }

    pub(crate) async fn ensure_managed_ollama(
        &self,
        progress_tx: &watch::Sender<DownloadProgress>,
        cancel: CancellationToken,
    ) -> Result<PathBuf, AppError> {
        let artifact =
            download::current_artifact().map_err(|message| AppError::Config(message.into()))?;
        let binary = self.install_runtime(&artifact, progress_tx, cancel).await?;
        self.prune_superseded_runtimes(&artifact);
        Ok(binary)
    }

    pub(crate) async fn install_runtime(
        &self,
        artifact: &download::OllamaArtifact,
        progress_tx: &watch::Sender<DownloadProgress>,
        cancel: CancellationToken,
    ) -> Result<PathBuf, AppError> {
        let install_dir = self.install_dir(artifact);
        if let Some(path) = self.find_installation(artifact) {
            match self.validate_ollama_binary(&path, cancel.clone()).await {
                Ok(()) => {
                    progress_tx
                        .send(DownloadProgress {
                            current_bytes: 0,
                            total_bytes: None,
                            percent: Some(100),
                        })
                        .ok();
                    return Ok(path);
                }
                Err(AppError::Canceled(message)) => return Err(AppError::Canceled(message)),
                Err(error) => {
                    warn!(path = %install_dir.display(), %error, "binary_manager.invalid_install_removed");
                }
            }
        } else if install_dir.exists() {
            warn!(path = %install_dir.display(), "binary_manager.incomplete_install_removed");
        }
        crate::util::durable_fs::durable_remove_entry(&install_dir).await?;

        let target_path = self.managed_binary_path(artifact);
        let runtime_root = self.runtime_root();
        std::fs::create_dir_all(&runtime_root)?;
        let archive_path = runtime_root.join(artifact.archive_name);

        progress_tx
            .send(DownloadProgress {
                current_bytes: 0,
                total_bytes: None,
                percent: Some(0),
            })
            .ok();

        download::download_ollama(
            &self.http_client,
            artifact,
            &archive_path,
            cancel.clone(),
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

        let installed = extract::install_ollama_runtime(&archive_path, &install_dir, artifact);
        let _ignored = std::fs::remove_file(&archive_path);
        installed?;
        if let Err(error) = self.validate_ollama_binary(&target_path, cancel).await {
            crate::util::durable_fs::durable_remove_entry(&install_dir).await?;
            return Err(error);
        }

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

    pub(crate) async fn validate_ollama_binary(
        &self,
        path: &Path,
        cancel: CancellationToken,
    ) -> Result<(), AppError> {
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

        run_version_check(path, VERSION_TIMEOUT, cancel).await
    }

    pub(crate) fn prune_superseded_runtimes(&self, current: &download::OllamaArtifact) {
        self.prune_stale_runtimes(&self.install_dir(current));
    }

    pub(crate) async fn remove_runtime(
        &self,
        artifact: &download::OllamaArtifact,
    ) -> Result<(), AppError> {
        crate::util::durable_fs::durable_remove_entry(&self.install_dir(artifact)).await?;
        Ok(())
    }

    fn prune_stale_runtimes(&self, keep: &Path) {
        let Ok(entries) = std::fs::read_dir(self.runtime_root()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == keep {
                continue;
            }
            let removed = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match removed {
                Ok(()) => info!(path = %path.display(), "binary_manager.pruned_stale_runtime"),
                Err(error) => {
                    warn!(path = %path.display(), %error, "binary_manager.prune_failed");
                }
            }
        }
    }

    fn runtime_root(&self) -> PathBuf {
        self.data_dir.join("ollama")
    }

    fn install_dir(&self, artifact: &download::OllamaArtifact) -> PathBuf {
        self.runtime_root().join(format!("v{}", artifact.version))
    }

    pub(crate) fn managed_binary_path(&self, artifact: &download::OllamaArtifact) -> PathBuf {
        self.install_dir(artifact).join(artifact.binary)
    }
}

struct GroupKill(Option<u32>);

fn release_number(version: &str) -> Option<Vec<u64>> {
    version.split('.').map(|part| part.parse().ok()).collect()
}

impl GroupKill {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupKill {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            crate::util::process_group::signal_group(pid, libc::SIGKILL);
        }
    }
}

async fn run_version_check(
    path: &Path,
    timeout: Duration,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| AppError::Config(format!("Failed to invoke managed Ollama: {e}")))?;
    let mut group = GroupKill(child.id());
    let mut stderr = child.stderr.take();
    let finished = tokio::select! {
        result = async {
            let mut captured = Vec::new();
            if let Some(stderr) = stderr.as_mut() {
                drop((&mut *stderr).take(VERSION_STDERR_LIMIT).read_to_end(&mut captured).await);
            }
            (child.wait().await, captured)
        } => Ok(result),
        () = tokio::time::sleep(timeout) => Err(AppError::Config(format!(
            "Managed Ollama did not answer --version within {}s",
            timeout.as_secs()
        ))),
        () = cancel.cancelled() => {
            Err(AppError::Canceled("Ollama validation canceled".into()))
        }
    };
    let (status, captured) = match finished {
        Ok((status, captured)) => (status?, captured),
        Err(error) => {
            drop(group);
            drop(child.start_kill());
            drop(child.wait().await);
            return Err(error);
        }
    };
    if let Some(pid) = group.0
        && crate::util::process_group::group_exists(pid)
    {
        crate::util::process_group::signal_group(pid, libc::SIGKILL);
    }
    group.disarm();
    if !status.success() {
        let stderr = String::from_utf8_lossy(&captured);
        return Err(AppError::Config(format!(
            "Managed Ollama failed validation (status {status}): {}",
            stderr.trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::BinaryManager;
    use super::download::current_artifact;
    use std::assert_matches;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

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
        crate::util::tls::install_crypto_provider();
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
            .validate_ollama_binary(&binary_path, CancellationToken::new())
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
            .validate_ollama_binary(&binary_path, CancellationToken::new())
            .await
            .expect_err("validation should fail");

        assert!(error.to_string().contains("not executable"));
    }

    #[test]
    fn find_installation_returns_none_for_fresh_data_dir() {
        let dir = temp_dir("find-absent");
        let manager = BinaryManager::new(&dir, test_client());
        assert_eq!(
            manager.find_installation(&current_artifact().expect("artifact")),
            None
        );
    }

    fn write_executable(path: &std::path::Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("file should have a parent"))
            .expect("parent should be created");
        std::fs::write(path, content).expect("executable should be written");
        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("permissions should be set");
    }

    #[test]
    fn find_installation_returns_path_of_current_version() {
        let dir = temp_dir("find-present");
        let artifact = current_artifact().expect("developer platform should be supported");
        let install = dir.join("ollama").join(format!("v{}", artifact.version));
        let binary = install.join(artifact.binary);
        write_executable(&binary, "fake");
        write_executable(&install.join(artifact.llama_server), "server");

        let manager = BinaryManager::new(&dir, test_client());
        assert_eq!(manager.find_installation(&artifact), Some(binary));
        let _ignored = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_installation_ignores_an_incomplete_install() {
        let artifact = current_artifact().expect("developer platform should be supported");
        for (name, server) in [("missing-runner", None), ("empty-runner", Some(""))] {
            let dir = temp_dir(name);
            let install = dir.join("ollama").join(format!("v{}", artifact.version));
            write_executable(&install.join(artifact.binary), "fake");
            if let Some(server) = server {
                write_executable(&install.join(artifact.llama_server), server);
            }

            let manager = BinaryManager::new(&dir, test_client());
            assert_eq!(manager.find_installation(&artifact), None, "{name}");
            let _ignored = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn previous_installation_is_the_newest_complete_older_runtime() {
        let dir = temp_dir("previous");
        let current = super::download::OllamaArtifact {
            version: "9.9.9",
            ..current_artifact().expect("developer platform should be supported")
        };
        let root = dir.join("ollama");
        for (name, complete) in [
            ("v0.40.9", true),
            ("v0.40.10", true),
            ("v0.41.0", false),
            ("vnext", true),
            ("v9.9.9", true),
        ] {
            let install = root.join(name);
            write_executable(&install.join(current.binary), "fake");
            if complete {
                write_executable(&install.join(current.llama_server), "server");
            }
        }

        let manager = BinaryManager::new(&dir, test_client());
        assert_eq!(
            manager.previous_installation(&current),
            Some(root.join("v0.40.10").join(current.binary))
        );
        let _ignored = std::fs::remove_dir_all(dir);
    }

    fn hanging_binary(dir: &std::path::Path) -> (PathBuf, PathBuf) {
        let pid_file = dir.join("pid");
        let binary = dir.join("ollama");
        write_executable(
            &binary,
            &format!(
                "#!/bin/sh\necho $$ > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\nsleep 600 &\nwait\n",
                pid = pid_file.display()
            ),
        );
        (binary, pid_file)
    }

    async fn started_pid(pid_file: &std::path::Path) -> u32 {
        for _ in 0..500 {
            if let Some(pid) = std::fs::read_to_string(pid_file)
                .ok()
                .and_then(|pid| pid.trim().parse().ok())
            {
                return pid;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the fake binary never started");
    }

    async fn assert_group_gone(pid: u32) {
        for _ in 0..100 {
            if !crate::util::process_group::group_exists(pid) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("process group {pid} survived validation");
    }

    #[tokio::test]
    async fn a_version_check_that_never_exits_is_killed_with_its_process_group() {
        let dir = temp_dir("hanging");
        let (binary, pid_file) = hanging_binary(&dir);
        let check = tokio::spawn(async move {
            super::run_version_check(
                &binary,
                std::time::Duration::from_secs(5),
                CancellationToken::new(),
            )
            .await
        });
        let pid = started_pid(&pid_file).await;

        let error = tokio::time::timeout(std::time::Duration::from_secs(30), check)
            .await
            .expect("the timeout should end validation")
            .expect("check should join")
            .expect_err("a hanging binary must fail validation");

        assert!(error.to_string().contains("did not answer"), "{error}");
        assert_group_gone(pid).await;
        let _ignored = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn canceling_a_version_check_kills_its_process_group() {
        let dir = temp_dir("canceled");
        let (binary, pid_file) = hanging_binary(&dir);
        let cancel = CancellationToken::new();
        let checking = cancel.clone();
        let check = tokio::spawn(async move {
            super::run_version_check(&binary, std::time::Duration::from_mins(10), checking).await
        });
        let pid = started_pid(&pid_file).await;
        cancel.cancel();

        let error = tokio::time::timeout(std::time::Duration::from_secs(5), check)
            .await
            .expect("cancellation should be prompt")
            .expect("check should join")
            .expect_err("validation should be canceled");

        assert_matches!(error, crate::error::AppError::Canceled(_));
        assert_group_gone(pid).await;
        let _ignored = std::fs::remove_dir_all(dir);
    }

    fn runtime_archive() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, content) in [
            (
                "ollama",
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'ollama version 0.0.0-test'\n  exit 0\nfi\nexit 1\n",
            ),
            ("llama-server", "#!/bin/sh\nexit 0\n"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o755);
            header.set_size(content.len() as u64);
            header.set_path(name).expect("path");
            header.set_cksum();
            builder
                .append(&header, content.as_bytes())
                .expect("entry should append");
        }
        let tar = builder.into_inner().expect("archive should finish");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut encoder, &tar).expect("gzip should encode");
        encoder.finish().expect("gzip should finish")
    }

    #[tokio::test]
    async fn a_corrupt_managed_install_is_removed_and_reinstalled() {
        use sha2::Digest;

        let archive = runtime_archive();
        let sha256: &'static str = Box::leak(
            crate::util::text::hex_encode(&sha2::Sha256::digest(&archive)).into_boxed_str(),
        );
        let app =
            axum::Router::new().route("/ollama.tgz", axum::routing::get(async move || archive));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("test server");
        });
        let url: &'static str = Box::leak(format!("http://{address}/ollama.tgz").into_boxed_str());
        let artifact = super::download::OllamaArtifact {
            version: "0.0.0-test",
            url,
            sha256,
            max_bytes: 1_000_000,
            archive_name: "ollama-test.tgz",
            format: super::download::ArchiveFormat::TarGz,
            binary: "ollama",
            llama_server: "llama-server",
        };

        for (name, binary, server) in [
            (
                "failing-binary",
                "#!/bin/sh\nexit 1\n",
                Some("#!/bin/sh\nexit 0\n"),
            ),
            ("missing-runner", "#!/bin/sh\nexit 0\n", None),
        ] {
            let dir = temp_dir(name);
            let install = dir.join("ollama").join("v0.0.0-test");
            write_executable(&install.join("ollama"), binary);
            if let Some(server) = server {
                write_executable(&install.join("llama-server"), server);
            }
            let manager = BinaryManager::new(&dir, test_client());
            let (progress_tx, _progress_rx) =
                tokio::sync::watch::channel(super::DownloadProgress::default());

            let installed = manager
                .install_runtime(&artifact, &progress_tx, CancellationToken::new())
                .await
                .unwrap_or_else(|error| panic!("{name}: {error}"));

            assert_eq!(installed, install.join("ollama"));
            assert!(
                std::fs::read_to_string(&installed)
                    .expect("installed binary")
                    .contains("0.0.0-test")
            );
            assert!(install.join("llama-server").is_file());
            assert!(!dir.join("ollama").join("ollama-test.tgz").exists());
            let again = manager
                .install_runtime(&artifact, &progress_tx, CancellationToken::new())
                .await
                .expect("a valid install is reused");
            assert_eq!(again, installed);
            let _ignored = std::fs::remove_dir_all(dir);
        }
        server.abort();
        drop(server.await);
    }

    #[test]
    fn find_installation_ignores_a_binary_only_legacy_install() {
        let dir = temp_dir("find-legacy");
        let bin_dir = dir.join("ollama").join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir should be created");
        std::fs::write(bin_dir.join("ollama"), "fake").expect("fake binary should be written");

        let manager = BinaryManager::new(&dir, test_client());
        assert_eq!(
            manager.find_installation(&current_artifact().expect("artifact")),
            None
        );
    }

    #[test]
    fn prune_stale_runtimes_keeps_only_the_current_install() {
        let dir = temp_dir("prune");
        let root = dir.join("ollama");
        let keep = root.join("v9.9.9");
        for stale in [root.join("bin"), root.join("v0.31.2"), keep.clone()] {
            std::fs::create_dir_all(&stale).expect("runtime dir should be created");
            std::fs::write(stale.join("ollama"), "fake").expect("fake binary should be written");
        }
        std::fs::write(root.join("ollama-darwin.tgz.part"), "partial")
            .expect("partial archive should be written");

        BinaryManager::new(&dir, test_client()).prune_stale_runtimes(&keep);

        let remaining: Vec<_> = std::fs::read_dir(&root)
            .expect("runtime root should exist")
            .map(|entry| entry.expect("entry should read").path())
            .collect();
        assert_eq!(remaining, vec![keep.clone()]);
        assert!(keep.join("ollama").is_file());
        let _ignored = std::fs::remove_dir_all(dir);
    }
}
