use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::error::AppError;
use crate::voice::install_artifacts::{ARTIFACTS, ModelArtifact, ModelFile};
use crate::voice::install_extract::{extract_and_install, install_directory, tempfile_dir};

pub(crate) use crate::voice::install_artifacts::{
    InstallStatus, ModelProgress, VoiceInstallSnapshot, VoiceModelKind,
};

pub(crate) struct VoiceEngineAvailability {
    pub(crate) stt: bool,
    pub(crate) tts: bool,
}

type OnCompleteCallback =
    Arc<dyn Fn() -> futures::future::BoxFuture<'static, VoiceEngineAvailability> + Send + Sync>;

pub(crate) struct VoiceInstallService {
    http: Arc<reqwest::Client>,
    models_root: PathBuf,
    state: Arc<Mutex<ServiceState>>,
    on_complete: OnCompleteCallback,
}

struct ServiceState {
    snapshot_tx: watch::Sender<VoiceInstallSnapshot>,
    snapshot_rx: watch::Receiver<VoiceInstallSnapshot>,
    in_flight: Option<JoinHandle<()>>,
    cancel_tx: Option<watch::Sender<bool>>,
}

fn safe_model_target(models_root: &Path, target_subpath: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(target_subpath);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(AppError::Config(format!(
            "Invalid voice model target path: {target_subpath}"
        )));
    }
    Ok(models_root.join(relative))
}

fn artifact_installed_at(artifact: &ModelArtifact, target: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(target) else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    metadata.is_dir()
        && artifact.required_files.iter().all(|relative| {
            std::fs::symlink_metadata(target.join(relative))
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        })
}

async fn remove_install_entry(path: &Path) -> Result<(), AppError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        crate::util::durable_fs::durable_remove_dir_all(path).await?;
    } else {
        crate::util::durable_fs::durable_remove_file(path).await?;
    }
    Ok(())
}

async fn cleanup_stale_staging(models_root: &Path) -> Result<(), AppError> {
    let mut entries = tokio::fs::read_dir(models_root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".voice-install-"))
        {
            remove_install_entry(&entry.path()).await?;
        }
    }
    Ok(())
}

async fn reconcile_replaced_model(
    models_root: &Path,
    artifact: &ModelArtifact,
) -> Result<(), AppError> {
    let target = safe_model_target(models_root, artifact.target_subpath)?;
    let backup = target.with_extension("replaced");
    if !tokio::fs::try_exists(&backup).await? {
        return Ok(());
    }
    if artifact_installed_at(artifact, &target) {
        remove_install_entry(&backup).await?;
        return Ok(());
    }
    if artifact_installed_at(artifact, &backup) {
        remove_install_entry(&target).await?;
        tokio::fs::rename(&backup, &target).await?;
        crate::util::durable_fs::sync_directory(models_root).await?;
        tracing::warn!(
            kind = %artifact.kind.label(),
            path = %target.display(),
            "voice_install.recovered_previous_model"
        );
    } else {
        remove_install_entry(&backup).await?;
    }
    Ok(())
}

pub(crate) async fn reconcile_install_state(models_root: &Path) -> Result<(), AppError> {
    tokio::fs::create_dir_all(models_root).await?;
    cleanup_stale_staging(models_root).await?;
    for artifact in ARTIFACTS {
        reconcile_replaced_model(models_root, artifact).await?;
    }
    Ok(())
}

#[cfg(test)]
#[expect(
    clippy::items_after_test_module,
    reason = "focused path-validation tests sit next to their private helper"
)]
mod tests {
    use super::{
        ARTIFACTS, artifact_installed_at, cleanup_stale_staging, reconcile_install_state,
        reconcile_replaced_model, safe_model_target,
    };
    use std::path::{Path, PathBuf};

    #[tokio::test]
    async fn a_failed_tts_reload_does_not_quarantine_a_healthy_recognizer() {
        use super::{
            VoiceEngineAvailability, VoiceInstallService, VoiceModelKind, unavailable_kinds,
        };
        use std::sync::Arc;
        crate::util::tls::install_crypto_provider();
        let root =
            std::env::temp_dir().join(format!("bobe-voice-isolation-{}", uuid::Uuid::new_v4()));
        for artifact in ARTIFACTS {
            create_artifact_files(&root.join(artifact.target_subpath), artifact.required_files);
        }
        let service = VoiceInstallService::new(
            Arc::new(reqwest::Client::new()),
            root.clone(),
            Arc::new(|| {
                Box::pin(async {
                    VoiceEngineAvailability {
                        tts: false,
                        stt: true,
                    }
                })
            }),
        );
        let failures = unavailable_kinds(
            &[VoiceModelKind::Tts, VoiceModelKind::Stt],
            &VoiceEngineAvailability {
                tts: false,
                stt: true,
            },
        );
        service.quarantine_invalid_targets(&failures).await;
        assert!(!service.is_installed(VoiceModelKind::Tts));
        assert!(service.is_installed(VoiceModelKind::Stt));
        assert!(
            service
                .target_path(VoiceModelKind::Tts)
                .with_extension("invalid")
                .is_dir()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_target_stays_beneath_models_root() {
        assert_eq!(
            safe_model_target(Path::new("/models"), "pocket/model.q8.gguf")
                .expect("relative target is valid"),
            PathBuf::from("/models/pocket/model.q8.gguf")
        );
    }

    #[test]
    fn model_target_rejects_absolute_and_traversal_paths() {
        for target in [
            "/tmp/model.onnx",
            "../model.onnx",
            "pocket/../model.q8.gguf",
            "",
        ] {
            assert!(safe_model_target(Path::new("/models"), target).is_err());
        }
    }

    fn create_artifact_files(root: &Path, required: &[&str]) {
        for relative in required {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().expect("artifact parent"))
                .expect("create artifact parent");
            std::fs::write(path, b"valid").expect("write artifact");
        }
    }

    #[tokio::test]
    async fn recovers_valid_previous_model_before_presence_check() {
        let models_root =
            std::env::temp_dir().join(format!("bobe-voice-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&models_root).expect("models root");
        let artifact = &ARTIFACTS[0];
        let target = safe_model_target(&models_root, artifact.target_subpath).unwrap();
        let backup = target.with_extension("replaced");
        create_artifact_files(&backup, artifact.required_files);
        std::fs::create_dir_all(&target).expect("partial target");
        std::fs::write(target.join("partial"), b"incomplete").expect("partial file");

        reconcile_install_state(&models_root).await.unwrap();

        assert!(artifact_installed_at(artifact, &target));
        assert!(!backup.exists());
        assert!(!target.join("partial").exists());
        std::fs::remove_dir_all(models_root).unwrap();
    }

    #[tokio::test]
    async fn valid_target_wins_and_stale_install_directories_are_removed() {
        let models_root =
            std::env::temp_dir().join(format!("bobe-voice-stale-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&models_root).expect("models root");
        let artifact = &ARTIFACTS[0];
        let target = safe_model_target(&models_root, artifact.target_subpath).unwrap();
        let backup = target.with_extension("replaced");
        create_artifact_files(&target, artifact.required_files);
        create_artifact_files(&backup, artifact.required_files);
        let staging = models_root.join(".voice-install-interrupted");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(staging.join("archive"), b"partial").expect("staging file");
        let unrelated = models_root.join("keep");
        std::fs::create_dir_all(&unrelated).expect("unrelated");

        cleanup_stale_staging(&models_root).await.unwrap();
        reconcile_replaced_model(&models_root, artifact)
            .await
            .unwrap();

        assert!(artifact_installed_at(artifact, &target));
        assert!(!backup.exists());
        assert!(!staging.exists());
        assert!(unrelated.exists());
        std::fs::remove_dir_all(models_root).unwrap();
    }
}

impl VoiceInstallService {
    pub(crate) fn new(
        http: Arc<reqwest::Client>,
        models_root: PathBuf,
        on_complete: OnCompleteCallback,
    ) -> Arc<Self> {
        let initial = VoiceInstallSnapshot {
            models: VoiceModelKind::all().map(ModelProgress::pending).collect(),
            status: InstallStatus::Idle,
            error: None,
        };
        let (snapshot_tx, snapshot_rx) = watch::channel(initial);
        Arc::new(Self {
            http,
            models_root,
            state: Arc::new(Mutex::new(ServiceState {
                snapshot_tx,
                snapshot_rx,
                in_flight: None,
                cancel_tx: None,
            })),
            on_complete,
        })
    }

    pub(crate) async fn subscribe(&self) -> watch::Receiver<VoiceInstallSnapshot> {
        self.state.lock().await.snapshot_rx.clone()
    }

    pub(crate) fn is_installed(&self, kind: VoiceModelKind) -> bool {
        let target = self.target_path(kind);
        if let Some(art) = ARTIFACTS.iter().find(|artifact| artifact.kind == kind) {
            artifact_installed_at(art, &target)
        } else {
            false
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "compile-time artifact catalog is exhaustive and validated by tests"
    )]
    pub(crate) fn target_path(&self, kind: VoiceModelKind) -> PathBuf {
        let art = ARTIFACTS
            .iter()
            .find(|a| a.kind == kind)
            .expect("every VoiceModelKind has a manifest entry");
        safe_model_target(&self.models_root, art.target_subpath)
            .expect("compile-time voice artifact target must remain relative")
    }

    pub(crate) async fn start(self: &Arc<Self>) -> Result<(), AppError> {
        self.start_kinds(VoiceModelKind::all().collect()).await
    }

    pub(crate) async fn start_kind(self: &Arc<Self>, kind: VoiceModelKind) -> Result<(), AppError> {
        self.start_kinds(vec![kind]).await
    }

    async fn start_kinds(self: &Arc<Self>, kinds: Vec<VoiceModelKind>) -> Result<(), AppError> {
        let mut state = self.state.lock().await;
        if let Some(h) = state.in_flight.as_ref()
            && !h.is_finished()
        {
            return Err(AppError::Conflict(
                "Voice install already in progress".into(),
            ));
        }
        let (cancel_tx, cancel_rx) = watch::channel(false);
        state.cancel_tx = Some(cancel_tx);
        let snapshot_tx = state.snapshot_tx.clone();
        drop(snapshot_tx.send_replace(VoiceInstallSnapshot {
            models: VoiceModelKind::all().map(ModelProgress::pending).collect(),
            status: InstallStatus::Running,
            error: None,
        }));
        let svc = Arc::clone(self);
        let on_complete = Arc::clone(&self.on_complete);
        state.in_flight = Some(tokio::spawn(async move {
            let requested_kinds = kinds.clone();
            if requested_kinds.iter().any(|kind| svc.is_installed(*kind)) {
                let availability = (on_complete)().await;
                let invalid = unavailable_kinds(&requested_kinds, &availability);
                svc.quarantine_invalid_targets(&invalid).await;
            }
            let result = Arc::clone(&svc)
                .run(kinds, snapshot_tx.clone(), cancel_rx)
                .await;
            let (mut final_status, mut final_error) = match result {
                Ok(()) => (InstallStatus::Complete, None),
                Err(AppError::Canceled(_)) => (InstallStatus::Canceled, None),
                Err(e) => {
                    warn!(err = %e, "voice_install.failed");
                    (InstallStatus::Failed, Some(e.to_string()))
                }
            };
            let availability = (on_complete)().await;
            let invalid = unavailable_kinds(&requested_kinds, &availability);
            svc.quarantine_invalid_targets(&invalid).await;
            if matches!(final_status, InstallStatus::Complete) && !invalid.is_empty() {
                final_status = InstallStatus::Failed;
                final_error =
                    Some("Installed voice model failed to load or pass its readiness probe".into());
            }
            let snap = snapshot_tx.borrow().clone();
            drop(snapshot_tx.send_replace(VoiceInstallSnapshot {
                status: final_status,
                error: final_error,
                ..snap
            }));
        }));
        Ok(())
    }

    async fn quarantine_invalid_targets(&self, kinds: &[VoiceModelKind]) {
        for kind in kinds {
            let target = self.target_path(*kind);
            if !tokio::fs::try_exists(&target).await.unwrap_or(false) {
                continue;
            }
            let invalid = target.with_extension("invalid");
            if tokio::fs::try_exists(&invalid).await.unwrap_or(false) {
                drop(tokio::fs::remove_dir_all(&invalid).await);
            }
            if let Err(error) = tokio::fs::rename(&target, &invalid).await {
                warn!(
                    kind = %kind.label(),
                    path = %target.display(),
                    %error,
                    "voice_install.invalid_target_quarantine_failed"
                );
            }
        }
    }

    pub(crate) async fn cancel(&self) {
        let state = self.state.lock().await;
        if let Some(tx) = state.cancel_tx.as_ref() {
            tx.send_replace(true);
        }
    }

    async fn run(
        self: Arc<Self>,
        kinds: Vec<VoiceModelKind>,
        snapshot_tx: watch::Sender<VoiceInstallSnapshot>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<(), AppError> {
        reconcile_install_state(&self.models_root).await?;

        for kind in kinds {
            let Some(art) = ARTIFACTS.iter().find(|artifact| artifact.kind == kind) else {
                return Err(AppError::Internal(format!(
                    "No artifact is configured for {}",
                    kind.label()
                )));
            };
            if *cancel_rx.borrow() {
                return Err(AppError::Canceled("Voice install canceled".into()));
            }
            if self.is_installed(art.kind) {
                info!(
                    kind = %art.kind.label(),
                    "voice_install.already_installed"
                );
                self.update_model(
                    &snapshot_tx,
                    art.kind,
                    ModelProgress::complete(art.kind, "already installed"),
                );
                continue;
            }
            self.install_one(art, &snapshot_tx, &cancel_rx).await?;
        }
        Ok(())
    }

    async fn install_one(
        &self,
        art: &ModelArtifact,
        snapshot_tx: &watch::Sender<VoiceInstallSnapshot>,
        cancel_rx: &watch::Receiver<bool>,
    ) -> Result<(), AppError> {
        info!(kind = %art.kind.label(), "voice_install.start");
        let tmp_dir = tempfile_dir(&self.models_root)?;
        let result = self
            .install_one_in(art, snapshot_tx, cancel_rx, &tmp_dir)
            .await;
        if let Err(error) = tokio::fs::remove_dir_all(&tmp_dir).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            warn!(path = %tmp_dir.display(), error = %error, "voice_install.temp_cleanup_failed");
        }
        result
    }

    async fn install_one_in(
        &self,
        art: &ModelArtifact,
        snapshot_tx: &watch::Sender<VoiceInstallSnapshot>,
        cancel_rx: &watch::Receiver<bool>,
        tmp_dir: &Path,
    ) -> Result<(), AppError> {
        let staging = tmp_dir.join("model");
        tokio::fs::create_dir(&staging).await?;
        let mut downloaded = 0;
        for file in art.files {
            let dest = safe_model_target(&staging, file.path)?;
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            downloaded += self
                .download_to(art, file, downloaded, &dest, snapshot_tx, cancel_rx)
                .await?;
        }
        if *cancel_rx.borrow() {
            return Err(AppError::Canceled("Voice install canceled".into()));
        }
        let final_target = safe_model_target(&self.models_root, art.target_subpath)?;
        if art.is_tarball {
            let archive = art
                .files
                .first()
                .ok_or_else(|| AppError::Internal("Missing voice archive".into()))?;
            let extracted = tmp_dir.join("extracted");
            tokio::fs::create_dir(&extracted).await?;
            extract_and_install(&staging.join(archive.path), &extracted, &final_target).await?;
        } else {
            tokio::fs::write(
                staging.join("NOTICE.txt"),
                include_str!("../../assets/pocket-tts-NOTICE.txt"),
            )
            .await?;
            if !artifact_installed_at(art, &staging) {
                return Err(AppError::Config("Voice pack is incomplete".into()));
            }
            install_directory(&staging, &final_target).await?;
        }
        self.update_model(
            snapshot_tx,
            art.kind,
            ModelProgress::complete(art.kind, "installed"),
        );
        info!(kind = %art.kind.label(), "voice_install.installed");
        Ok(())
    }

    async fn download_to(
        &self,
        art: &ModelArtifact,
        download: &ModelFile,
        previously_downloaded: u64,
        dest: &Path,
        snapshot_tx: &watch::Sender<VoiceInstallSnapshot>,
        cancel_rx: &watch::Receiver<bool>,
    ) -> Result<u64, AppError> {
        let mut cancel = cancel_rx.clone();
        let response = tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    return Err(AppError::Canceled("Voice install canceled".into()));
                }
                return Err(AppError::Internal("Voice install cancellation channel changed unexpectedly".into()));
            }
            response = self.http.get(download.url).send() => response?.error_for_status()?,
        };
        let file_total = response.content_length();
        let total = if art.is_tarball {
            file_total
        } else {
            Some(art.files.iter().map(|file| file.max_bytes).sum())
        };
        if file_total.is_some_and(|size| size > download.max_bytes) {
            return Err(AppError::Config(format!(
                "{} download Content-Length exceeds {} byte limit",
                art.kind.label(),
                download.max_bytes
            )));
        }
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(dest).await?;
        let mut integrity = crate::util::download_integrity::DownloadIntegrity::new(
            art.kind.label(),
            download.sha256,
            download.max_bytes,
        );
        use tokio::io::AsyncWriteExt;
        let throttle = Duration::from_millis(250);
        let mut last_emit = Instant::now();
        let mut last_percent: Option<u8> = None;
        let mut got: u64 = 0;
        let make_progress = |got: u64, percent: Option<u8>| ModelProgress {
            kind: art.kind,
            label: art.kind.label(),
            status: "downloading".into(),
            bytes_downloaded: previously_downloaded + got,
            bytes_total: total,
            percent,
        };
        loop {
            let next = tokio::select! {
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        return Err(AppError::Canceled("Voice install canceled".into()));
                    }
                    continue;
                }
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk?;
            integrity.update(&chunk)?;
            file.write_all(&chunk).await?;
            got = got.saturating_add(chunk.len() as u64);
            let percent = total.map(|t| {
                (((previously_downloaded + got) as f64 / t as f64) * 100.0).min(100.0) as u8
            });
            if percent != last_percent || last_emit.elapsed() >= throttle {
                self.update_model(snapshot_tx, art.kind, make_progress(got, percent));
                last_emit = Instant::now();
                last_percent = percent;
            }
        }
        let final_percent = total
            .map(|t| (((previously_downloaded + got) as f64 / t as f64) * 100.0).min(100.0) as u8);
        self.update_model(snapshot_tx, art.kind, make_progress(got, final_percent));
        file.flush().await?;
        let verified_bytes = integrity.finish()?;
        debug_assert_eq!(got, verified_bytes);
        Ok(verified_bytes)
    }

    fn update_model(
        &self,
        snapshot_tx: &watch::Sender<VoiceInstallSnapshot>,
        kind: VoiceModelKind,
        progress: ModelProgress,
    ) {
        snapshot_tx.send_modify(|snapshot| {
            if let Some(slot) = snapshot.models.iter_mut().find(|model| model.kind == kind) {
                *slot = progress;
            }
        });
    }

    pub(crate) async fn await_idle(&self) {
        let handle = {
            let mut state = self.state.lock().await;
            match state.in_flight.as_ref() {
                Some(h) if !h.is_finished() => state.in_flight.take(),
                _ => None,
            }
        };
        if let Some(h) = handle {
            drop(h.await);
        }
    }
}

fn unavailable_kinds(
    kinds: &[VoiceModelKind],
    availability: &VoiceEngineAvailability,
) -> Vec<VoiceModelKind> {
    kinds
        .iter()
        .copied()
        .filter(|kind| match kind {
            VoiceModelKind::Tts => !availability.tts,
            VoiceModelKind::Stt => !availability.stt,
        })
        .collect()
}
