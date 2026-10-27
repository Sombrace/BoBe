use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::error::AppError;
use crate::services::ollama::binary_download::{OllamaArtifact, current_artifact};
use crate::services::ollama::binary_manager::{BinaryManager, DownloadProgress};
use crate::services::ollama::manager::{OllamaManager, PullProgress};

const INSTALL_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const UPGRADE_IDLE_POLL: Duration = Duration::from_secs(1);

pub(crate) type IdleCheck = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallStage {
    Text,
    Batch,
}

impl InstallStage {
    const fn name(self) -> &'static str {
        match self {
            InstallStage::Text => "chat-model",
            InstallStage::Batch => "batch-model",
        }
    }

    fn progress_mut(self, snapshot: &mut InstallSnapshot) -> &mut PullProgress {
        match self {
            InstallStage::Text => &mut snapshot.chat_model,
            InstallStage::Batch => &mut snapshot.batch_model,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InstallSnapshot {
    pub(crate) runtime: DownloadProgress,
    pub(crate) chat_model: PullProgress,
    pub(crate) batch_model: PullProgress,
    pub(crate) status: InstallStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum InstallStatus {
    #[default]
    Idle,
    Running,
    Complete,
    Canceled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct InstallRequest {
    pub(crate) chat_model: String,
    pub(crate) batch_model: String,
}

pub(crate) fn uses_managed_runtime<'a>(
    profiles: &crate::provider::profile::ProvidersConfig,
    used: impl IntoIterator<Item = &'a crate::provider::ProfileId>,
) -> bool {
    used.into_iter().any(|profile| {
        profiles.profiles.get(profile).is_some_and(|profile| {
            profile.local_runtime == Some(crate::provider::LocalRuntimeKind::Ollama)
        })
    })
}

pub(crate) struct OllamaInstallService {
    binary_manager: Arc<BinaryManager>,
    ollama_manager: Arc<OllamaManager>,
    state: Arc<Mutex<ServiceState>>,
}

struct ServiceState {
    snapshot_tx: watch::Sender<InstallSnapshot>,
    snapshot_rx: watch::Receiver<InstallSnapshot>,
    in_flight: Option<JoinHandle<()>>,
    cancel: Option<CancellationToken>,
}

impl OllamaInstallService {
    pub(crate) fn new(
        binary_manager: Arc<BinaryManager>,
        ollama_manager: Arc<OllamaManager>,
    ) -> Arc<Self> {
        let (snapshot_tx, snapshot_rx) = watch::channel(InstallSnapshot::default());
        Arc::new(Self {
            binary_manager,
            ollama_manager,
            state: Arc::new(Mutex::new(ServiceState {
                snapshot_tx,
                snapshot_rx,
                in_flight: None,
                cancel: None,
            })),
        })
    }

    pub(crate) async fn subscribe(&self) -> watch::Receiver<InstallSnapshot> {
        self.state.lock().await.snapshot_rx.clone()
    }

    pub(crate) async fn ensure_running(self: &Arc<Self>, idle: IdleCheck) -> Result<(), AppError> {
        let Ok(artifact) = current_artifact() else {
            return Ok(());
        };
        if let Some(binary) = self.binary_manager.installed_binary() {
            self.ollama_manager.ensure_daemon_running(&binary).await?;
            if !self.busy().await {
                self.binary_manager.prune_superseded_runtimes(&artifact);
            }
            return Ok(());
        }
        let Some(previous) = self.binary_manager.previous_installation(&artifact) else {
            return Ok(());
        };
        let started = self.ollama_manager.ensure_daemon_running(&previous).await;
        let upgrade = self
            .launch(async move |service, snapshot_tx, cancel| {
                service
                    .upgrade(&artifact, &previous, &*idle, &snapshot_tx, cancel)
                    .await
            })
            .await;
        if let Err(error) = upgrade {
            info!(%error, "ollama_install.upgrade_deferred");
        }
        started
    }

    async fn busy(&self) -> bool {
        self.state
            .lock()
            .await
            .in_flight
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    pub(crate) async fn start(self: &Arc<Self>, req: InstallRequest) -> Result<(), AppError> {
        self.launch(async move |service, snapshot_tx, cancel| {
            service.run(req, snapshot_tx, cancel).await
        })
        .await
    }

    async fn launch<Work, Done>(self: &Arc<Self>, work: Work) -> Result<(), AppError>
    where
        Work: FnOnce(Arc<Self>, watch::Sender<InstallSnapshot>, CancellationToken) -> Done,
        Done: Future<Output = Result<(), AppError>> + Send + 'static,
    {
        let mut state = self.state.lock().await;
        if let Some(h) = state.in_flight.as_ref()
            && !h.is_finished()
        {
            return Err(AppError::Conflict("Install already in progress".into()));
        }

        let cancel = CancellationToken::new();
        state.cancel = Some(cancel.clone());

        let snapshot_tx = state.snapshot_tx.clone();
        snapshot_tx
            .send(InstallSnapshot {
                status: InstallStatus::Running,
                ..InstallSnapshot::default()
            })
            .ok();
        let work = work(Arc::clone(self), snapshot_tx.clone(), cancel);
        state.in_flight = Some(tokio::spawn(async move {
            let final_status = match work.await {
                Ok(()) => InstallStatus::Complete,
                Err(AppError::Canceled(_)) => InstallStatus::Canceled,
                Err(e) => {
                    warn!(err = %e, "ollama_install.failed");
                    InstallStatus::Failed(e.to_string())
                }
            };
            snapshot_tx.send_modify(|snap| snap.status = final_status);
        }));

        Ok(())
    }

    pub(crate) async fn cancel(&self) {
        let state = self.state.lock().await;
        if let Some(cancel) = state.cancel.as_ref() {
            cancel.cancel();
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.cancel().await;
        let in_flight = self.state.lock().await.in_flight.take();
        if let Some(mut handle) = in_flight
            && tokio::time::timeout(INSTALL_SHUTDOWN_GRACE, &mut handle)
                .await
                .is_err()
        {
            warn!("ollama_install.shutdown_aborted");
            handle.abort();
            drop(handle.await);
        }
        self.ollama_manager.stop().await;
    }

    async fn run(
        self: Arc<Self>,
        req: InstallRequest,
        snapshot_tx: watch::Sender<InstallSnapshot>,
        cancel: CancellationToken,
    ) -> Result<(), AppError> {
        let (runtime_tx, mut runtime_rx) = watch::channel(DownloadProgress::default());
        let runtime_snapshot_tx = snapshot_tx.clone();
        let runtime_pump = tokio::spawn(async move {
            while runtime_rx.changed().await.is_ok() {
                let progress = runtime_rx.borrow_and_update().clone();
                runtime_snapshot_tx.send_modify(|snap| snap.runtime = progress);
            }
        });

        let needs_managed_binary = !self.ollama_manager.health_check().await;
        let binary_path = if needs_managed_binary {
            info!("ollama_install.runtime_missing_downloading");
            Some(
                self.binary_manager
                    .ensure_managed_ollama(&runtime_tx, cancel.clone())
                    .await?,
            )
        } else {
            info!("ollama_install.runtime_present_skipping_download");
            runtime_tx
                .send(DownloadProgress {
                    current_bytes: 0,
                    total_bytes: None,
                    percent: Some(100),
                })
                .ok();
            None
        };
        drop(runtime_tx);
        drop(runtime_pump.await);

        if cancel.is_cancelled() {
            return Err(AppError::Canceled("Install canceled".into()));
        }

        if let Some(binary_path) = binary_path.as_deref() {
            self.ollama_manager
                .ensure_daemon_running(binary_path)
                .await?;
        }

        if cancel.is_cancelled() {
            return Err(AppError::Canceled("Install canceled".into()));
        }

        let installed = self
            .ollama_manager
            .list_installed_models()
            .await
            .unwrap_or_default();

        self.maybe_pull_model(
            &req.chat_model,
            &installed,
            InstallStage::Text,
            &snapshot_tx,
            cancel.clone(),
        )
        .await?;

        if req.batch_model == req.chat_model {
            snapshot_tx.send_modify(|snap| {
                snap.batch_model = PullProgress {
                    status: "skipped (same as chat)".into(),
                    completed_bytes: None,
                    total_bytes: None,
                    percent: Some(100),
                };
            });
        } else {
            let installed_now = self
                .ollama_manager
                .list_installed_models()
                .await
                .unwrap_or_default();
            self.maybe_pull_model(
                &req.batch_model,
                &installed_now,
                InstallStage::Batch,
                &snapshot_tx,
                cancel,
            )
            .await?;
        }

        Ok(())
    }

    async fn upgrade(
        &self,
        artifact: &OllamaArtifact,
        previous: &Path,
        idle: &(dyn Fn() -> bool + Send + Sync),
        snapshot_tx: &watch::Sender<InstallSnapshot>,
        cancel: CancellationToken,
    ) -> Result<(), AppError> {
        info!(
            version = artifact.version,
            previous = %previous.display(),
            "ollama_install.upgrade_started"
        );
        let (runtime_tx, mut runtime_rx) = watch::channel(DownloadProgress::default());
        let runtime_snapshot_tx = snapshot_tx.clone();
        let runtime_pump = tokio::spawn(async move {
            while runtime_rx.changed().await.is_ok() {
                let progress = runtime_rx.borrow_and_update().clone();
                runtime_snapshot_tx.send_modify(|snap| snap.runtime = progress);
            }
        });
        let installed = self
            .binary_manager
            .install_runtime(artifact, &runtime_tx, cancel.clone())
            .await;
        drop(runtime_tx);
        drop(runtime_pump.await);
        let binary = installed?;

        while !idle() {
            tokio::select! {
                () = cancel.cancelled() => {
                    return Err(AppError::Canceled("Ollama upgrade canceled".into()));
                }
                () = tokio::time::sleep(UPGRADE_IDLE_POLL) => {}
            }
        }
        if let Err(error) = self.ollama_manager.restart_with(&binary).await {
            warn!(%error, "ollama_install.upgrade_reverted");
            if let Err(restore) = self.ollama_manager.ensure_daemon_running(previous).await {
                warn!(error = %restore, "ollama_install.previous_restart_failed");
            }
            if let Err(cleanup) = self.binary_manager.remove_runtime(artifact).await {
                warn!(error = %cleanup, "ollama_install.upgrade_cleanup_failed");
            }
            return Err(error);
        }
        self.binary_manager.prune_superseded_runtimes(artifact);
        info!(version = artifact.version, "ollama_install.upgraded");
        Ok(())
    }

    async fn maybe_pull_model(
        &self,
        name: &str,
        installed: &[String],
        stage: InstallStage,
        snapshot_tx: &watch::Sender<InstallSnapshot>,
        cancel: CancellationToken,
    ) -> Result<(), AppError> {
        let already_installed = installed.iter().any(|m| m == name);
        if already_installed {
            info!(
                model = name,
                stage = stage.name(),
                "ollama_install.model_already_installed"
            );
            self.write_stage(
                snapshot_tx,
                stage,
                PullProgress {
                    status: "already installed".into(),
                    completed_bytes: None,
                    total_bytes: None,
                    percent: Some(100),
                },
            );
            return Ok(());
        }

        let (pull_tx, mut pull_rx) = watch::channel(PullProgress::default());
        let pump_snapshot_tx = snapshot_tx.clone();
        let pump = tokio::spawn(async move {
            while pull_rx.changed().await.is_ok() {
                let progress = pull_rx.borrow_and_update().clone();
                pump_snapshot_tx.send_modify(|snap| *stage.progress_mut(snap) = progress);
            }
        });

        let result = self.ollama_manager.pull_model(name, &pull_tx, cancel).await;
        pump.abort();
        drop(pump.await);
        result
    }

    fn write_stage(
        &self,
        snapshot_tx: &watch::Sender<InstallSnapshot>,
        stage: InstallStage,
        progress: PullProgress,
    ) {
        snapshot_tx.send_modify(|snap| *stage.progress_mut(snap) = progress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_routes_on_the_managed_runtime_need_it_started() {
        let providers = crate::config::Config::default().providers;
        let local = crate::provider::ProfileId::from_static("local");
        let copilot = crate::provider::ProfileId::from_static("copilot");
        let missing = crate::provider::ProfileId::from_static("missing");
        assert!(uses_managed_runtime(&providers, [&copilot, &local]));
        assert!(!uses_managed_runtime(&providers, [&copilot, &missing]));
    }

    #[tokio::test]
    async fn a_runtime_that_was_never_installed_is_not_started() {
        crate::util::tls::install_crypto_provider();
        let root =
            std::env::temp_dir().join(format!("bobe-ollama-ensure-{}", uuid::Uuid::new_v4()));
        let client = Arc::new(reqwest::Client::new());
        let service = OllamaInstallService::new(
            Arc::new(BinaryManager::new(&root, Arc::clone(&client))),
            Arc::new(OllamaManager::new(client, "http://127.0.0.1:9")),
        );
        service
            .ensure_running(Arc::new(|| true))
            .await
            .expect("there is nothing to start");
        assert!(!root.join("ollama").exists());
    }

    fn write_runtime(install: &Path, artifact: &OllamaArtifact) {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        for (relative, script) in [
            (
                artifact.binary,
                "#!/bin/sh\n[ \"$1\" = --version ] && exit 0\nexit 1\n",
            ),
            (artifact.llama_server, "#!/bin/sh\nexit 0\n"),
        ] {
            let path = install.join(relative);
            std::fs::create_dir_all(path.parent().expect("runtime file should have a parent"))
                .expect("runtime dir should be created");
            std::fs::write(&path, script).expect("runtime file should be written");
            #[cfg(unix)]
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("permissions should be set");
        }
    }

    async fn serving_ollama() -> (std::net::SocketAddr, JoinHandle<()>) {
        let app =
            axum::Router::new().route("/api/tags", axum::routing::get(async || "{\"models\":[]}"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("test server");
        });
        (address, server)
    }

    fn service_for(root: &Path, address: std::net::SocketAddr) -> Arc<OllamaInstallService> {
        crate::util::tls::install_crypto_provider();
        let client = Arc::new(reqwest::Client::new());
        OllamaInstallService::new(
            Arc::new(BinaryManager::new(root, Arc::clone(&client))),
            Arc::new(OllamaManager::new(client, &format!("http://{address}"))),
        )
    }

    #[tokio::test]
    async fn an_installed_runtime_is_replaced_once_no_run_is_active() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use crate::services::ollama::binary_download::ArchiveFormat;

        let root =
            std::env::temp_dir().join(format!("bobe-ollama-upgrade-{}", uuid::Uuid::new_v4()));
        let artifact = OllamaArtifact {
            version: "0.2.0",
            url: "http://127.0.0.1:9/ollama.tgz",
            sha256: "",
            max_bytes: 0,
            archive_name: "ollama.tgz",
            format: ArchiveFormat::TarGz,
            binary: "ollama",
            llama_server: "llama-server",
        };
        let previous = root.join("ollama").join("v0.1.0");
        let current = root.join("ollama").join("v0.2.0");
        write_runtime(&previous, &artifact);
        write_runtime(&current, &artifact);
        let (address, server) = serving_ollama().await;
        let service = service_for(&root, address);
        let polls = AtomicUsize::new(0);
        let idle = || polls.fetch_add(1, Ordering::SeqCst) >= 2;
        let (snapshot_tx, _snapshot_rx) = watch::channel(InstallSnapshot::default());

        service
            .upgrade(
                &artifact,
                &previous.join("ollama"),
                &idle,
                &snapshot_tx,
                CancellationToken::new(),
            )
            .await
            .expect("the upgrade should switch runtimes");

        assert_eq!(polls.load(Ordering::SeqCst), 3);
        assert!(current.join("ollama").is_file());
        assert!(!previous.exists());
        assert_eq!(snapshot_tx.borrow().runtime.percent, Some(100));
        server.abort();
        drop(server.await);
        let _ignored = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn starting_the_pinned_runtime_removes_superseded_ones() {
        let artifact = current_artifact().expect("developer platform should be supported");
        let root = std::env::temp_dir().join(format!("bobe-ollama-prune-{}", uuid::Uuid::new_v4()));
        let current = root.join("ollama").join(format!("v{}", artifact.version));
        let superseded = root.join("ollama").join("v0.1.0");
        write_runtime(&current, &artifact);
        write_runtime(&superseded, &artifact);
        let (address, server) = serving_ollama().await;

        service_for(&root, address)
            .ensure_running(Arc::new(|| true))
            .await
            .expect("the pinned runtime is already serving");

        assert!(current.join(artifact.binary).is_file());
        assert!(!superseded.exists());
        server.abort();
        drop(server.await);
        let _ignored = std::fs::remove_dir_all(root);
    }
}
