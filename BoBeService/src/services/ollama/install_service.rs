use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::error::AppError;
use crate::services::ollama::binary_manager::{BinaryManager, DownloadProgress};
use crate::services::ollama::manager::{OllamaManager, PullProgress};

const INSTALL_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

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

    pub(crate) async fn ensure_running(&self) -> Result<(), AppError> {
        match self.binary_manager.installed_binary() {
            Some(binary) => self.ollama_manager.ensure_daemon_running(&binary).await,
            None => Ok(()),
        }
    }

    pub(crate) async fn start(self: &Arc<Self>, req: InstallRequest) -> Result<(), AppError> {
        let mut state = self.state.lock().await;
        if let Some(h) = state.in_flight.as_ref()
            && !h.is_finished()
        {
            return Err(AppError::Conflict("Install already in progress".into()));
        }

        let cancel = CancellationToken::new();
        state.cancel = Some(cancel.clone());

        let svc = Arc::clone(self);
        let snapshot_tx = state.snapshot_tx.clone();
        snapshot_tx
            .send(InstallSnapshot {
                status: InstallStatus::Running,
                ..InstallSnapshot::default()
            })
            .ok();
        state.in_flight = Some(tokio::spawn(async move {
            let result = svc.run(req, snapshot_tx.clone(), cancel).await;
            let final_status = match result {
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
            .ensure_running()
            .await
            .expect("there is nothing to start");
        assert!(!root.join("ollama").exists());
    }
}
