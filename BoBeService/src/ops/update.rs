use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::util::text::hex_encode;

const RELEASE_PUBLIC_KEY_BASE64: &str = "rQS3dCsQx+IdGVnMSrkCJoP1WHuMoz3ft1jT0Z/UXXs=";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StandaloneLayout {
    pub(crate) prefix: PathBuf,
    pub(crate) versions_root: PathBuf,
    pub(crate) launcher: PathBuf,
}

impl StandaloneLayout {
    pub(crate) fn discover(executable: &Path) -> Result<Self, AppError> {
        let executable = executable.canonicalize()?;
        let bin = executable
            .parent()
            .ok_or_else(|| AppError::Config("standalone executable has no parent".into()))?;
        let version = bin.parent();
        let versions = version.and_then(Path::parent);
        let prefix = if versions
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("versions")
        {
            versions
                .and_then(Path::parent)
                .and_then(Path::parent)
                .and_then(Path::parent)
                .ok_or_else(|| AppError::Config("standalone version store has no prefix".into()))?
                .to_path_buf()
        } else {
            bin.parent()
                .ok_or_else(|| AppError::Config("standalone executable has no prefix".into()))?
                .to_path_buf()
        };
        Ok(Self {
            versions_root: prefix.join("share/bobe/versions"),
            launcher: prefix.join("bin/bobe"),
            prefix,
        })
    }
}

pub(crate) fn install_launcher(layout: &StandaloneLayout) -> Result<(), AppError> {
    std::fs::create_dir_all(
        layout
            .launcher
            .parent()
            .ok_or_else(|| AppError::Config("standalone launcher has no parent".into()))?,
    )?;
    let temporary = layout
        .launcher
        .with_file_name(format!(".bobe-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    std::os::unix::fs::symlink("../share/bobe/versions/current/bin/bobe", &temporary)?;
    #[cfg(not(unix))]
    return Err(AppError::Config("standalone launcher requires Unix".into()));
    std::fs::rename(&temporary, &layout.launcher)?;
    if let Some(parent) = layout.launcher.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub(crate) fn current_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        _ => "unsupported",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UpdatePhase {
    Staging,
    Stopping,
    BackupVerified,
    Activated,
    RollingBack,
    Committed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct UpdateJournal {
    pub(crate) phase: UpdatePhase,
    pub(crate) previous: PathBuf,
    pub(crate) previous_version: String,
    pub(crate) candidate: String,
    pub(crate) backup: Option<PathBuf>,
    pub(crate) authority_id: Option<uuid::Uuid>,
    pub(crate) managed_service: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) preserved: Vec<PathBuf>,
}

pub(crate) const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(1);
const READY_PROBE: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) trait UpdateHost {
    async fn stop_service(&self) -> Result<(), AppError>;
    async fn install_service(&self, version_root: &Path) -> Result<(), AppError>;
    async fn ready(
        &self,
        authority_id: Option<uuid::Uuid>,
        version: &str,
        timeout: std::time::Duration,
    ) -> bool;
    async fn restore_backup(&self, backup: &Path) -> Result<(), AppError>;
    async fn preserve_data(&self) -> Result<PathBuf, AppError>;
}

pub(crate) struct SystemUpdateHost<'a> {
    pub(crate) service_paths: &'a crate::ops::service::ServicePaths,
    pub(crate) paths: &'a crate::util::paths::DataPaths,
    pub(crate) config_file: &'a Path,
    pub(crate) target: crate::ops::status::OpsTarget,
}

impl UpdateHost for SystemUpdateHost<'_> {
    async fn stop_service(&self) -> Result<(), AppError> {
        crate::ops::service::stop(self.service_paths).await
    }

    async fn install_service(&self, version_root: &Path) -> Result<(), AppError> {
        let executable = version_root.join("bin/bobe");
        let agent = crate::agent::RuntimeFiles::installed(&executable);
        crate::ops::service::install(&executable, agent.as_ref(), self.service_paths).await
    }

    async fn ready(
        &self,
        authority_id: Option<uuid::Uuid>,
        version: &str,
        timeout: std::time::Duration,
    ) -> bool {
        let (exit, _) = crate::ops::status::wait_ready_for(
            &self.target,
            timeout,
            std::time::Duration::from_millis(500),
            crate::ops::status::ReadinessLevel::Process,
            authority_id,
            None,
            Some(version),
        )
        .await;
        exit == crate::ops::exit::ExitCode::Success
    }

    async fn restore_backup(&self, backup: &Path) -> Result<(), AppError> {
        let locks = crate::util::runtime_lock::RuntimeLocks::acquire_after_service_stop(
            self.paths,
            std::time::Duration::from_secs(15),
        )
        .await?;
        crate::ops::backup::restore(
            backup,
            self.paths,
            self.config_file,
            &crate::bootstrap::database::SUPPORTED_SCHEMA_VERSIONS,
        )
        .await?;
        drop(locks);
        Ok(())
    }

    async fn preserve_data(&self) -> Result<PathBuf, AppError> {
        let locks = crate::util::runtime_lock::RuntimeLocks::acquire_after_service_stop(
            self.paths,
            std::time::Duration::from_secs(15),
        )
        .await?;
        crate::ops::backup::reconcile_restore_journals(self.paths, self.config_file).await?;
        let backup = crate::ops::backup::create_offline(self.paths, self.config_file).await?;
        drop(locks);
        Ok(backup)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Recovered {
    Nothing,
    Finalized,
    RolledBack { preserved: Vec<PathBuf> },
    Abandoned,
}

pub(crate) async fn recover(
    journal_path: &Path,
    versions_root: &Path,
    host: &impl UpdateHost,
) -> Result<Recovered, AppError> {
    let Some(journal) = recover_staging_journal(journal_path, versions_root).await? else {
        return Ok(Recovered::Nothing);
    };
    match journal.phase {
        UpdatePhase::Committed => {
            clear_journal(journal_path).await?;
            return Ok(Recovered::Finalized);
        }
        UpdatePhase::RollingBack => {}
        _ if !is_current(versions_root, &journal.candidate) => {
            abandon(journal_path, versions_root, &journal, host).await?;
            return Ok(Recovered::Abandoned);
        }
        _ if !journal.managed_service || candidate_ready(versions_root, &journal, host).await => {
            commit(journal_path, &journal).await?;
            return Ok(Recovered::Finalized);
        }
        _ => {}
    }
    host.stop_service().await?;
    let mut preserved = journal.preserved.clone();
    preserved.push(host.preserve_data().await?);
    roll_back(
        journal_path,
        versions_root,
        &UpdateJournal {
            preserved: preserved.clone(),
            ..journal
        },
        host,
    )
    .await?;
    Ok(Recovered::RolledBack { preserved })
}

pub(crate) async fn commit(journal_path: &Path, journal: &UpdateJournal) -> Result<(), AppError> {
    write_journal(
        journal_path,
        &UpdateJournal {
            phase: UpdatePhase::Committed,
            ..journal.clone()
        },
    )
    .await?;
    clear_journal(journal_path).await
}

pub(crate) async fn roll_back(
    journal_path: &Path,
    versions_root: &Path,
    journal: &UpdateJournal,
    host: &impl UpdateHost,
) -> Result<(), AppError> {
    write_journal(
        journal_path,
        &UpdateJournal {
            phase: UpdatePhase::RollingBack,
            ..journal.clone()
        },
    )
    .await?;
    host.stop_service().await?;
    if let Some(backup) = &journal.backup {
        host.restore_backup(backup).await?;
    }
    abandon(journal_path, versions_root, journal, host).await
}

pub(crate) async fn abandon(
    journal_path: &Path,
    versions_root: &Path,
    journal: &UpdateJournal,
    host: &impl UpdateHost,
) -> Result<(), AppError> {
    rollback(versions_root, &journal.previous)?;
    if journal.managed_service {
        host.install_service(&versions_root.join(&journal.previous))
            .await?;
        if !host
            .ready(
                journal.authority_id,
                &journal.previous_version,
                READY_TIMEOUT,
            )
            .await
        {
            return Err(AppError::Internal(
                "previous service did not become ready after rollback".into(),
            ));
        }
    }
    discard_candidate(versions_root, &journal.candidate).await?;
    clear_journal(journal_path).await
}

async fn candidate_ready(
    versions_root: &Path,
    journal: &UpdateJournal,
    host: &impl UpdateHost,
) -> bool {
    if host
        .ready(journal.authority_id, &journal.candidate, READY_PROBE)
        .await
    {
        return true;
    }
    if let Err(error) = host
        .install_service(&versions_root.join(&journal.candidate))
        .await
    {
        tracing::warn!(%error, candidate = %journal.candidate, "update.candidate_restart_failed");
        return false;
    }
    host.ready(journal.authority_id, &journal.candidate, READY_TIMEOUT)
        .await
}

fn is_current(versions_root: &Path, candidate: &str) -> bool {
    std::fs::read_link(versions_root.join("current"))
        .is_ok_and(|target| target == Path::new(candidate))
}

pub(crate) async fn write_journal(path: &Path, journal: &UpdateJournal) -> Result<(), AppError> {
    crate::util::durable_fs::atomic_write(path, &serde_json::to_vec_pretty(journal)?).await
}

pub(crate) async fn read_journal(path: &Path) -> Result<Option<UpdateJournal>, AppError> {
    if !tokio::fs::try_exists(path).await? {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&tokio::fs::read(path).await?)?))
}

pub(crate) async fn clear_journal(path: &Path) -> Result<(), AppError> {
    crate::util::durable_fs::durable_remove_file(path).await?;
    Ok(())
}

pub(crate) async fn ensure_candidate_available(
    versions_root: &Path,
    candidate: &str,
) -> Result<(), AppError> {
    validate_version_component(candidate, "candidate")?;
    if tokio::fs::try_exists(versions_root.join(candidate)).await? {
        return Err(AppError::Conflict(format!(
            "standalone version {candidate} is already installed"
        )));
    }
    Ok(())
}

pub(crate) async fn recover_staging_journal(
    journal_path: &Path,
    versions_root: &Path,
) -> Result<Option<UpdateJournal>, AppError> {
    let journal = read_journal(journal_path).await?;
    if let Some(journal) = journal
        .as_ref()
        .filter(|journal| journal.phase == UpdatePhase::Staging)
    {
        discard_candidate(versions_root, &journal.candidate).await?;
        clear_journal(journal_path).await?;
        return Ok(None);
    }
    Ok(journal)
}

pub(crate) async fn discard_candidate(
    versions_root: &Path,
    candidate: &str,
) -> Result<(), AppError> {
    validate_version_component(candidate, "candidate")?;
    crate::util::durable_fs::durable_remove_dir_all(&versions_root.join(candidate)).await?;
    let prefix = format!(".{candidate}-");
    let mut entries = match tokio::fs::read_dir(versions_root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            crate::util::durable_fs::durable_remove_dir_all(&entry.path()).await?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct StandaloneManifest {
    pub(crate) schema: u32,
    pub(crate) version: String,
    pub(crate) platform: String,
    pub(crate) database_schema: i64,
    pub(crate) database_schema_min: i64,
    pub(crate) attach_protocol_min: u32,
    pub(crate) attach_protocol_max: u32,
    pub(crate) archive_sha256: String,
    pub(crate) archive: String,
}

impl StandaloneManifest {
    pub(crate) fn supports_database_schema(&self, live: i64) -> bool {
        (self.database_schema_min..=self.database_schema).contains(&live)
    }
}

pub(crate) fn verify_manifest(
    manifest_bytes: &[u8],
    signature_base64: &str,
    expected_platform: &str,
    current_database_schema: i64,
) -> Result<StandaloneManifest, AppError> {
    verify_manifest_with_key(
        manifest_bytes,
        signature_base64,
        RELEASE_PUBLIC_KEY_BASE64,
        expected_platform,
        current_database_schema,
    )
}

fn verify_manifest_with_key(
    manifest_bytes: &[u8],
    signature_base64: &str,
    public_key_base64: &str,
    expected_platform: &str,
    current_database_schema: i64,
) -> Result<StandaloneManifest, AppError> {
    let mut key_bytes = [0_u8; 32];
    match base64::engine::general_purpose::STANDARD.decode_slice(public_key_base64, &mut key_bytes)
    {
        Ok(32) => {}
        Ok(_) | Err(base64::DecodeSliceError::OutputSliceTooSmall) => {
            return Err(AppError::Config(
                "update public key must be 32 bytes".into(),
            ));
        }
        Err(base64::DecodeSliceError::DecodeError(error)) => {
            return Err(AppError::Config(format!("update public key: {error}")));
        }
    }
    let key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|error| AppError::Config(format!("update public key: {error}")))?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_base64)
        .map_err(|error| AppError::Config(format!("update signature: {error}")))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| AppError::Config(format!("update signature: {error}")))?;
    key.verify_strict(manifest_bytes, &signature)
        .map_err(|_| AppError::Conflict("standalone update signature is invalid".into()))?;
    let manifest: StandaloneManifest = serde_json::from_slice(manifest_bytes)?;
    if manifest.schema != 1
        || manifest.platform != expected_platform
        || !manifest.supports_database_schema(current_database_schema)
        || manifest.attach_protocol_min > crate::constants::attach::PROTOCOL_MAX
        || manifest.attach_protocol_max < crate::constants::attach::PROTOCOL_MIN
    {
        return Err(AppError::Conflict(
            "standalone update is not compatible with this runtime".into(),
        ));
    }
    Ok(manifest)
}

pub(crate) async fn stage_archive(
    manifest: &StandaloneManifest,
    archive: &Path,
    versions_root: &Path,
) -> Result<PathBuf, AppError> {
    let bytes = tokio::fs::read(archive).await?;
    if hex_encode(&Sha256::digest(&bytes)) != manifest.archive_sha256 {
        return Err(AppError::Conflict(
            "standalone archive checksum mismatch".into(),
        ));
    }
    let destination = versions_root.join(&manifest.version);
    if tokio::fs::try_exists(&destination).await? {
        return Err(AppError::Conflict(format!(
            "standalone version {} is already installed",
            manifest.version
        )));
    }
    let staging = versions_root.join(format!(".{}-{}", manifest.version, uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&staging).await?;
    let archive = archive.to_path_buf();
    let staging_clone = staging.clone();
    let extracted = tokio::task::spawn_blocking(move || -> Result<PathBuf, AppError> {
        let file = std::fs::File::open(archive)?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(decoder);
        for entry in tar.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            if path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
                || entry.header().entry_type().is_symlink()
                || entry.header().entry_type().is_hard_link()
                || !(entry.header().entry_type().is_file() || entry.header().entry_type().is_dir())
            {
                return Err(AppError::Config(
                    "standalone archive contains an unsafe path".into(),
                ));
            }
            entry.unpack_in(&staging_clone)?;
        }
        let entries = std::fs::read_dir(&staging_clone)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .collect::<Vec<_>>();
        if entries.len() != 1 {
            return Err(AppError::Config(
                "standalone archive must contain one root directory".into(),
            ));
        }
        Ok(entries[0].path())
    })
    .await
    .map_err(|error| AppError::Internal(format!("update extraction join: {error}")))??;
    if validate_version_directory(&extracted).is_err() {
        drop(crate::util::durable_fs::durable_remove_dir_all(&staging).await);
        return Err(AppError::Conflict(
            "standalone archive is incomplete".into(),
        ));
    }
    crate::util::durable_fs::sync_tree(&extracted)?;
    tokio::fs::rename(extracted, &destination).await?;
    crate::util::durable_fs::sync_directory(versions_root).await?;
    crate::util::durable_fs::durable_remove_dir_all(&staging).await?;
    Ok(destination)
}

pub(crate) async fn seed_previous_version(
    versions_root: &Path,
    executable: &Path,
    agent: Option<&crate::agent::RuntimeFiles>,
    version: &str,
) -> Result<PathBuf, AppError> {
    let version = format!("previous-{version}");
    validate_version_component(&version, "previous")?;
    let destination = versions_root.join(&version);
    if tokio::fs::try_exists(&destination).await? {
        validate_version_directory(&destination)?;
    } else {
        let staging = versions_root.join(format!(".{version}-{}", uuid::Uuid::new_v4()));
        let result = async {
            tokio::fs::create_dir_all(staging.join("bin")).await?;
            copy_and_sync(executable, &staging.join("bin/bobe")).await?;
            let agent = agent.ok_or_else(|| {
                AppError::Config("cannot stage rollback without the installed pi runtime".into())
            })?;
            copy_and_sync(&agent.node, &staging.join("bin/node")).await?;
            copy_runtime_tree(agent.agent_dir()?, &staging.join("bin/agent")).await?;
            crate::util::durable_fs::atomic_write(
                &staging.join("compatibility.json"),
                br#"{"schema":1,"imported_previous":true}"#,
            )
            .await?;
            crate::util::durable_fs::sync_directory(&staging.join("bin")).await?;
            crate::util::durable_fs::sync_directory(&staging).await?;
            tokio::fs::rename(&staging, &destination).await?;
            crate::util::durable_fs::sync_directory(versions_root).await?;
            Ok::<(), AppError>(())
        }
        .await;
        if result.is_err() {
            drop(crate::util::durable_fs::durable_remove_dir_all(&staging).await);
        }
        result?;
    }
    let current = versions_root.join("current");
    if !current.exists() {
        activate_link(versions_root, &version)?;
    }
    std::fs::read_link(&current).or_else(|_| Ok(PathBuf::from(version)))
}

pub(crate) fn version_for_target(
    versions_root: &Path,
    target: &Path,
    imported_fallback: &str,
) -> Result<String, AppError> {
    let bytes = std::fs::read(versions_root.join(target).join("compatibility.json"))?;
    let compatibility: serde_json::Value = serde_json::from_slice(&bytes)?;
    if let Some(version) = compatibility
        .get("version")
        .and_then(|value| value.as_str())
    {
        return Ok(version.to_owned());
    }
    if compatibility
        .get("imported_previous")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Ok(imported_fallback.to_owned());
    }
    Err(AppError::Conflict(
        "standalone previous version has no version metadata".into(),
    ))
}

async fn copy_and_sync(source: &Path, destination: &Path) -> Result<(), AppError> {
    tokio::fs::copy(source, destination).await?;
    tokio::fs::File::open(destination).await?.sync_all().await?;
    Ok(())
}

fn validate_version_directory(path: &Path) -> Result<(), AppError> {
    if !path.join("bin/bobe").is_file()
        || !path.join("compatibility.json").is_file()
        || !path.join("bin/node").is_file()
        || !path.join("bin/agent/dist/main.js").is_file()
        || !path.join("bin/agent/package.json").is_file()
        || !path.join("bin/agent/assets/chat.md").is_file()
        || !path.join("bin/agent/bin/rg").is_file()
        || !path.join("bin/agent/bin/fd").is_file()
        || !path
            .join("bin/agent/node_modules/@earendil-works/pi-coding-agent/package.json")
            .is_file()
    {
        return Err(AppError::Conflict(format!(
            "standalone rollback version is incomplete: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn activate(versions_root: &Path, version: &str) -> Result<Option<PathBuf>, AppError> {
    validate_version_component(version, "candidate")?;
    let target = versions_root.join(version);
    validate_version_directory(&target)?;
    let current = versions_root.join("current");
    let previous = std::fs::read_link(&current).ok();
    activate_link(versions_root, version)?;
    Ok(previous)
}

async fn copy_runtime_tree(source: &Path, target: &Path) -> Result<(), AppError> {
    let metadata = tokio::fs::symlink_metadata(source).await?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(AppError::Config(
            "pi runtime must be a regular directory".into(),
        ));
    }
    tokio::fs::create_dir_all(target).await?;
    let mut entries = tokio::fs::read_dir(source).await?;
    while let Some(entry) = entries.next_entry().await? {
        let kind = entry.file_type().await?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            Box::pin(copy_runtime_tree(&entry.path(), &destination)).await?;
        } else if kind.is_file() {
            copy_and_sync(&entry.path(), &destination).await?;
        } else {
            return Err(AppError::Config(
                "pi runtime contains an unsupported filesystem entry".into(),
            ));
        }
    }
    crate::util::durable_fs::sync_directory(target).await
}

fn activate_link(versions_root: &Path, version: &str) -> Result<(), AppError> {
    validate_version_component(version, "activation")?;
    let current = versions_root.join("current");
    let temporary = versions_root.join(format!(".current-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    std::os::unix::fs::symlink(version, &temporary)?;
    #[cfg(not(unix))]
    return Err(AppError::Config(
        "standalone activation requires Unix".into(),
    ));
    std::fs::rename(temporary, current)?;
    std::fs::File::open(versions_root)?.sync_all()?;
    Ok(())
}

pub(crate) fn rollback(versions_root: &Path, previous: &Path) -> Result<(), AppError> {
    let previous = previous
        .to_str()
        .ok_or_else(|| AppError::Config("previous version is not valid UTF-8".into()))?;
    validate_version_component(previous, "previous")?;
    validate_version_directory(&versions_root.join(previous))?;
    activate_link(versions_root, previous)
}

fn validate_version_component(value: &str, field: &str) -> Result<(), AppError> {
    let path = Path::new(value);
    if path.components().count() != 1
        || !matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(AppError::Config(format!(
            "update journal contains an invalid {field} version"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[tokio::test]
    async fn staging_recovery_removes_installed_candidate_before_retry() {
        let root = std::env::temp_dir().join(format!("bobe-update-{}", uuid::Uuid::new_v4()));
        let versions = root.join("versions");
        tokio::fs::create_dir_all(versions.join("2.0.0/bin"))
            .await
            .unwrap();
        tokio::fs::write(versions.join("2.0.0/bin/bobe"), b"candidate")
            .await
            .unwrap();
        tokio::fs::create_dir_all(versions.join(".2.0.0-interrupted"))
            .await
            .unwrap();
        let journal_path = versions.join("update-journal.json");
        write_journal(
            &journal_path,
            &UpdateJournal {
                phase: UpdatePhase::Staging,
                previous: PathBuf::from("previous-1.0.0"),
                previous_version: "1.0.0".into(),
                candidate: "2.0.0".into(),
                backup: None,
                authority_id: None,
                managed_service: false,
                preserved: Vec::new(),
            },
        )
        .await
        .unwrap();

        let remaining = recover_staging_journal(&journal_path, &versions)
            .await
            .unwrap();

        assert!(remaining.is_none());
        assert!(!journal_path.exists());
        assert!(!versions.join("2.0.0").exists());
        assert!(!versions.join(".2.0.0-interrupted").exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn non_staging_recovery_is_preserved_for_transaction_rollback() {
        let root = std::env::temp_dir().join(format!("bobe-update-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let journal_path = root.join("update-journal.json");
        write_journal(
            &journal_path,
            &UpdateJournal {
                phase: UpdatePhase::BackupVerified,
                previous: PathBuf::from("previous-1.0.0"),
                previous_version: "1.0.0".into(),
                candidate: "2.0.0".into(),
                backup: Some(PathBuf::from("backup.tar.gz")),
                authority_id: Some(uuid::Uuid::nil()),
                managed_service: true,
                preserved: Vec::new(),
            },
        )
        .await
        .unwrap();

        let remaining = recover_staging_journal(&journal_path, &root)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(remaining.phase, UpdatePhase::BackupVerified);
        assert!(journal_path.exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    struct FakeHost {
        ready: std::collections::HashMap<String, Vec<bool>>,
        failing_install: Option<&'static str>,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl FakeHost {
        fn new(ready: &[(&str, &[bool])]) -> Self {
            Self {
                ready: ready
                    .iter()
                    .map(|(version, answers)| ((*version).to_owned(), answers.to_vec()))
                    .collect(),
                failing_install: None,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: String) -> usize {
            let mut calls = self.calls.lock().unwrap();
            calls.push(call.clone());
            calls.iter().filter(|recorded| **recorded == call).count()
        }
    }

    impl UpdateHost for FakeHost {
        async fn stop_service(&self) -> Result<(), AppError> {
            self.record("stop".into());
            Ok(())
        }

        async fn install_service(&self, version_root: &Path) -> Result<(), AppError> {
            let version = version_root.file_name().unwrap().to_string_lossy();
            let attempt = self.record(format!("install {version}"));
            if attempt == 1 && self.failing_install == Some(version.as_ref()) {
                return Err(AppError::Internal(format!("install {version} failed")));
            }
            Ok(())
        }

        async fn ready(
            &self,
            _authority_id: Option<uuid::Uuid>,
            version: &str,
            _timeout: std::time::Duration,
        ) -> bool {
            let attempt = self.record(format!("ready {version}"));
            self.ready
                .get(version)
                .and_then(|answers| answers.get(attempt - 1).or(answers.last()))
                .copied()
                .unwrap_or(false)
        }

        async fn restore_backup(&self, backup: &Path) -> Result<(), AppError> {
            self.record(format!("restore {}", backup.display()));
            Ok(())
        }

        async fn preserve_data(&self) -> Result<PathBuf, AppError> {
            let attempt = self.record("preserve".into());
            Ok(PathBuf::from(format!("live-data-{attempt}")))
        }
    }

    fn installed_version(versions: &Path, name: &str) {
        let root = versions.join(name);
        for file in [
            "bin/bobe",
            "compatibility.json",
            "bin/node",
            "bin/agent/dist/main.js",
            "bin/agent/package.json",
            "bin/agent/assets/chat.md",
            "bin/agent/bin/rg",
            "bin/agent/bin/fd",
            "bin/agent/node_modules/@earendil-works/pi-coding-agent/package.json",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, name).unwrap();
        }
    }

    async fn update_fixture(
        phase: UpdatePhase,
        current: &str,
        managed_service: bool,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("bobe-update-{}", uuid::Uuid::new_v4()));
        let versions = root.join("versions");
        installed_version(&versions, "previous-1.0.0");
        installed_version(&versions, "2.0.0");
        activate_link(&versions, current).unwrap();
        let journal_path = versions.join("update-journal.json");
        write_journal(
            &journal_path,
            &UpdateJournal {
                phase,
                previous: PathBuf::from("previous-1.0.0"),
                previous_version: "1.0.0".into(),
                candidate: "2.0.0".into(),
                backup: Some(root.join("backup-before-update")),
                authority_id: Some(uuid::Uuid::nil()),
                managed_service,
                preserved: Vec::new(),
            },
        )
        .await
        .unwrap();
        (root, versions, journal_path)
    }

    fn current(versions: &Path) -> PathBuf {
        std::fs::read_link(versions.join("current")).unwrap()
    }

    #[tokio::test]
    async fn a_committed_update_is_finalized_without_restoring_data() {
        let (root, versions, journal) = update_fixture(UpdatePhase::Committed, "2.0.0", true).await;
        let host = FakeHost::new(&[]);

        let recovered = recover(&journal, &versions, &host).await.unwrap();

        assert_eq!(recovered, Recovered::Finalized);
        assert_eq!(host.calls(), Vec::<String>::new());
        assert!(!journal.exists());
        assert_eq!(current(&versions), PathBuf::from("2.0.0"));
        assert!(versions.join("2.0.0/bin/bobe").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_ready_candidate_killed_before_commit_is_finalized_without_restoring_data() {
        let (root, versions, journal) = update_fixture(UpdatePhase::Activated, "2.0.0", true).await;
        let host = FakeHost::new(&[("2.0.0", &[true])]);

        let recovered = recover(&journal, &versions, &host).await.unwrap();

        assert_eq!(recovered, Recovered::Finalized);
        assert_eq!(host.calls(), ["ready 2.0.0"]);
        assert!(!journal.exists());
        assert_eq!(current(&versions), PathBuf::from("2.0.0"));
        assert!(versions.join("2.0.0/bin/bobe").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_stopped_candidate_is_restarted_and_committed_when_it_becomes_ready() {
        let (root, versions, journal) = update_fixture(UpdatePhase::Activated, "2.0.0", true).await;
        let host = FakeHost::new(&[("2.0.0", &[false, true])]);

        let recovered = recover(&journal, &versions, &host).await.unwrap();

        assert_eq!(recovered, Recovered::Finalized);
        assert_eq!(
            host.calls(),
            ["ready 2.0.0", "install 2.0.0", "ready 2.0.0"]
        );
        assert!(!journal.exists());
        assert_eq!(current(&versions), PathBuf::from("2.0.0"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn an_unmanaged_activation_is_its_own_commit() {
        for phase in [UpdatePhase::BackupVerified, UpdatePhase::Activated] {
            let (root, versions, journal) = update_fixture(phase, "2.0.0", false).await;
            let host = FakeHost::new(&[]);

            let recovered = recover(&journal, &versions, &host).await.unwrap();

            assert_eq!(recovered, Recovered::Finalized, "{phase:?}");
            assert_eq!(host.calls(), Vec::<String>::new());
            assert!(!journal.exists());
            assert_eq!(current(&versions), PathBuf::from("2.0.0"));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn a_candidate_that_never_becomes_ready_is_rolled_back_to_its_backup() {
        let (root, versions, journal) = update_fixture(UpdatePhase::Activated, "2.0.0", true).await;
        let host = FakeHost::new(&[("2.0.0", &[false]), ("1.0.0", &[true])]);

        let recovered = recover(&journal, &versions, &host).await.unwrap();

        assert_eq!(
            recovered,
            Recovered::RolledBack {
                preserved: vec![PathBuf::from("live-data-1")]
            }
        );
        assert_eq!(
            host.calls(),
            [
                "ready 2.0.0".to_owned(),
                "install 2.0.0".to_owned(),
                "ready 2.0.0".to_owned(),
                "stop".to_owned(),
                "preserve".to_owned(),
                "stop".to_owned(),
                format!("restore {}", root.join("backup-before-update").display()),
                "install previous-1.0.0".to_owned(),
                "ready 1.0.0".to_owned(),
            ]
        );
        assert!(!journal.exists());
        assert_eq!(current(&versions), PathBuf::from("previous-1.0.0"));
        assert!(!versions.join("2.0.0").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_rollback_interrupted_after_its_restore_is_finished_without_rechecking_the_candidate()
    {
        let (root, versions, journal) = update_fixture(UpdatePhase::Activated, "2.0.0", true).await;
        let interrupted = read_journal(&journal).await.unwrap().unwrap();
        write_journal(
            &journal,
            &UpdateJournal {
                phase: UpdatePhase::RollingBack,
                preserved: vec![PathBuf::from("live-data-before-the-crash")],
                ..interrupted
            },
        )
        .await
        .unwrap();
        let host = FakeHost::new(&[("2.0.0", &[true]), ("1.0.0", &[true])]);

        let recovered = recover(&journal, &versions, &host).await.unwrap();

        assert_eq!(
            recovered,
            Recovered::RolledBack {
                preserved: vec![
                    PathBuf::from("live-data-before-the-crash"),
                    PathBuf::from("live-data-1"),
                ]
            }
        );
        assert_eq!(
            host.calls(),
            [
                "stop".to_owned(),
                "preserve".to_owned(),
                "stop".to_owned(),
                format!("restore {}", root.join("backup-before-update").display()),
                "install previous-1.0.0".to_owned(),
                "ready 1.0.0".to_owned(),
            ]
        );
        assert!(!journal.exists());
        assert_eq!(current(&versions), PathBuf::from("previous-1.0.0"));
        assert!(!versions.join("2.0.0").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_failed_previous_install_leaves_a_rollback_that_resumes_and_reports_its_copies() {
        let (root, versions, journal) = update_fixture(UpdatePhase::Activated, "2.0.0", true).await;
        let mut host = FakeHost::new(&[("2.0.0", &[false]), ("1.0.0", &[true])]);
        host.failing_install = Some("previous-1.0.0");

        recover(&journal, &versions, &host)
            .await
            .expect_err("the previous service failed to install");

        let pending = read_journal(&journal).await.unwrap().unwrap();
        assert_eq!(pending.phase, UpdatePhase::RollingBack);
        assert_eq!(pending.preserved, [PathBuf::from("live-data-1")]);
        let restores = |calls: &[String]| {
            calls
                .iter()
                .filter(|call| call.starts_with("restore "))
                .count()
        };
        assert_eq!(restores(&host.calls()), 1);

        let recovered = recover(&journal, &versions, &host).await.unwrap();

        assert_eq!(
            recovered,
            Recovered::RolledBack {
                preserved: vec![PathBuf::from("live-data-1"), PathBuf::from("live-data-2")]
            }
        );
        let calls = host.calls();
        assert_eq!(restores(&calls), 2);
        assert_eq!(
            calls.iter().filter(|call| *call == "ready 2.0.0").count(),
            2
        );
        assert!(!journal.exists());
        assert_eq!(current(&versions), PathBuf::from("previous-1.0.0"));
        assert!(!versions.join("2.0.0").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn an_update_interrupted_before_activation_never_restores_its_backup() {
        for phase in [UpdatePhase::Stopping, UpdatePhase::BackupVerified] {
            let (root, versions, journal) = update_fixture(phase, "previous-1.0.0", true).await;
            let host = FakeHost::new(&[("1.0.0", &[true])]);

            let recovered = recover(&journal, &versions, &host).await.unwrap();

            assert_eq!(recovered, Recovered::Abandoned, "{phase:?}");
            assert_eq!(host.calls(), ["install previous-1.0.0", "ready 1.0.0"]);
            assert!(!journal.exists());
            assert_eq!(current(&versions), PathBuf::from("previous-1.0.0"));
            assert!(!versions.join("2.0.0").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn a_released_journal_never_reaches_recovery_again() {
        let (root, versions, journal) = update_fixture(UpdatePhase::Activated, "2.0.0", true).await;
        let activated = read_journal(&journal).await.unwrap().unwrap();
        write_journal(
            &journal,
            &UpdateJournal {
                phase: UpdatePhase::Committed,
                ..activated
            },
        )
        .await
        .unwrap();
        let committed = std::fs::read_to_string(&journal).unwrap();
        assert!(committed.contains("\"committed\""));

        let host = FakeHost::new(&[]);
        assert_eq!(
            recover(&journal, &versions, &host).await.unwrap(),
            Recovered::Finalized
        );
        assert_eq!(
            recover(&journal, &versions, &host).await.unwrap(),
            Recovered::Nothing
        );
        assert_eq!(host.calls(), Vec::<String>::new());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn journals_written_by_the_previous_release_still_parse() {
        for phase in ["staging", "stopping", "backup_verified", "activated"] {
            let journal: UpdateJournal = serde_json::from_value(serde_json::json!({
                "phase": phase,
                "previous": "previous-0.9.0",
                "previous_version": "0.9.0",
                "candidate": "0.9.1",
                "backup": null,
                "authority_id": null,
                "managed_service": true,
            }))
            .unwrap();
            assert_eq!(serde_json::to_value(journal.phase).unwrap(), phase);
        }
    }

    #[tokio::test]
    async fn previous_seed_is_atomic_and_reuses_active_version() {
        let root = std::env::temp_dir().join(format!("bobe-update-{}", uuid::Uuid::new_v4()));
        let versions = root.join("versions");
        let source = root.join("source");
        tokio::fs::create_dir_all(&versions).await.unwrap();
        tokio::fs::create_dir_all(&source).await.unwrap();
        tokio::fs::write(source.join("bobe"), b"daemon")
            .await
            .unwrap();
        let agent = source.join("agent");
        for directory in [
            "dist",
            "assets",
            "bin",
            "node_modules/@earendil-works/pi-coding-agent",
        ] {
            tokio::fs::create_dir_all(agent.join(directory))
                .await
                .unwrap();
        }
        for (name, bytes) in [
            ("node", b"node".as_slice()),
            ("dist/main.js", b"worker".as_slice()),
            ("assets/chat.md", b"behavior".as_slice()),
            ("bin/rg", b"search".as_slice()),
            ("bin/fd", b"find".as_slice()),
            ("package.json", b"{}".as_slice()),
            (
                "node_modules/@earendil-works/pi-coding-agent/package.json",
                b"{}".as_slice(),
            ),
        ] {
            tokio::fs::write(agent.join(name), bytes).await.unwrap();
        }

        let previous = seed_previous_version(
            &versions,
            &source.join("bobe"),
            Some(&crate::agent::RuntimeFiles {
                node: agent.join("node"),
                entry: agent.join("dist/main.js"),
            }),
            "1.0.0",
        )
        .await
        .unwrap();

        assert_eq!(previous, PathBuf::from("previous-1.0.0"));
        assert_eq!(
            tokio::fs::read(versions.join("previous-1.0.0/bin/bobe"))
                .await
                .unwrap(),
            b"daemon"
        );
        assert_eq!(
            tokio::fs::read(versions.join("previous-1.0.0/bin/agent/dist/main.js"))
                .await
                .unwrap(),
            b"worker"
        );
        assert_eq!(
            std::fs::read_link(versions.join("current")).unwrap(),
            PathBuf::from("previous-1.0.0")
        );
        assert!(
            tokio::fs::read_dir(&versions)
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_some()
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn incomplete_previous_seed_is_rejected() {
        let root = std::env::temp_dir().join(format!("bobe-update-{}", uuid::Uuid::new_v4()));
        let versions = root.join("versions");
        let source = root.join("bobe");
        tokio::fs::create_dir_all(versions.join("previous-1.0.0/bin"))
            .await
            .unwrap();
        tokio::fs::write(&source, b"daemon").await.unwrap();

        let result = seed_previous_version(&versions, &source, None, "1.0.0").await;

        assert!(result.is_err());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn discovers_flat_and_versioned_custom_prefixes() {
        let root = std::env::temp_dir().join(format!("bobe-layout-{}", uuid::Uuid::new_v4()));
        let flat = root.join("custom prefix/bin/bobe");
        std::fs::create_dir_all(flat.parent().unwrap()).unwrap();
        std::fs::write(&flat, b"daemon").unwrap();
        let flat_layout = StandaloneLayout::discover(&flat).unwrap();
        assert_eq!(
            flat_layout.prefix,
            root.join("custom prefix").canonicalize().unwrap()
        );

        let versioned = root.join("custom prefix/share/bobe/versions/2.0.0/bin/bobe");
        std::fs::create_dir_all(versioned.parent().unwrap()).unwrap();
        std::fs::write(&versioned, b"daemon").unwrap();
        let versioned_layout = StandaloneLayout::discover(&versioned).unwrap();
        assert_eq!(versioned_layout, flat_layout);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launcher_follows_atomic_current_link() {
        let root = std::env::temp_dir().join(format!("bobe-layout-{}", uuid::Uuid::new_v4()));
        let layout = StandaloneLayout {
            prefix: root.clone(),
            versions_root: root.join("share/bobe/versions"),
            launcher: root.join("bin/bobe"),
        };
        std::fs::create_dir_all(layout.versions_root.join("2.0.0/bin")).unwrap();
        std::fs::write(layout.versions_root.join("2.0.0/bin/bobe"), b"daemon").unwrap();
        activate_link(&layout.versions_root, "2.0.0").unwrap();
        install_launcher(&layout).unwrap();
        assert_eq!(
            std::fs::read_link(&layout.launcher).unwrap(),
            PathBuf::from("../share/bobe/versions/current/bin/bobe")
        );
        assert_eq!(std::fs::read(&layout.launcher).unwrap(), b"daemon");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_rejects_previous_path_escape() {
        let root = std::env::temp_dir().join(format!("bobe-update-{}", uuid::Uuid::new_v4()));
        assert!(rollback(&root, Path::new("../outside")).is_err());
    }

    fn signed(manifest: &serde_json::Value) -> (Vec<u8>, String, String) {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let manifest = serde_json::to_vec(manifest).unwrap();
        let signature =
            base64::engine::general_purpose::STANDARD.encode(signing.sign(&manifest).to_bytes());
        let public =
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());
        (manifest, signature, public)
    }

    fn candidate(database_schema: i64, database_schema_min: i64) -> serde_json::Value {
        serde_json::to_value(StandaloneManifest {
            schema: 1,
            version: "1.2.3".into(),
            platform: "darwin-arm64".into(),
            database_schema,
            database_schema_min,
            attach_protocol_min: 1,
            attach_protocol_max: 1,
            archive_sha256: "00".repeat(32),
            archive: "bobe.tar.gz".into(),
        })
        .unwrap()
    }

    #[test]
    fn signed_manifest_verifies_and_tampering_fails() {
        let (manifest, signature, public) = signed(&candidate(2, 2));
        assert!(
            verify_manifest_with_key(&manifest, &signature, &public, "darwin-arm64", 2).is_ok()
        );
        let mut tampered = manifest;
        tampered.push(b' ');
        assert!(
            verify_manifest_with_key(&tampered, &signature, &public, "darwin-arm64", 2).is_err()
        );
    }

    #[test]
    fn a_candidate_accepts_every_live_schema_it_can_upgrade() {
        let (manifest, signature, public) = signed(&candidate(10, 9));
        for (live, accepted) in [(8, false), (9, true), (10, true), (11, false)] {
            assert_eq!(
                verify_manifest_with_key(&manifest, &signature, &public, "darwin-arm64", live)
                    .is_ok(),
                accepted,
                "live schema {live}"
            );
        }
        let mut without_minimum = candidate(10, 9);
        without_minimum
            .as_object_mut()
            .unwrap()
            .remove("database_schema_min");
        let (manifest, signature, public) = signed(&without_minimum);
        assert!(
            verify_manifest_with_key(&manifest, &signature, &public, "darwin-arm64", 10).is_err()
        );
    }
}
