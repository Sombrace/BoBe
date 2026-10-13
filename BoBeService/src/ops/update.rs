use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct UpdateJournal {
    pub(crate) phase: String,
    pub(crate) previous: PathBuf,
    pub(crate) previous_version: String,
    pub(crate) candidate: String,
    pub(crate) backup: Option<PathBuf>,
    pub(crate) authority_id: Option<uuid::Uuid>,
    pub(crate) managed_service: bool,
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
        .filter(|journal| journal.phase == "staging")
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
    pub(crate) attach_protocol_min: u32,
    pub(crate) attach_protocol_max: u32,
    pub(crate) archive_sha256: String,
    pub(crate) archive: String,
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
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_base64)
        .map_err(|error| AppError::Config(format!("update public key: {error}")))?;
    let key = VerifyingKey::from_bytes(
        &key_bytes
            .try_into()
            .map_err(|_| AppError::Config("update public key must be 32 bytes".into()))?,
    )
    .map_err(|error| AppError::Config(format!("update public key: {error}")))?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_base64)
        .map_err(|error| AppError::Config(format!("update signature: {error}")))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| AppError::Config(format!("update signature: {error}")))?;
    key.verify(manifest_bytes, &signature)
        .map_err(|_| AppError::Conflict("standalone update signature is invalid".into()))?;
    let manifest: StandaloneManifest = serde_json::from_slice(manifest_bytes)?;
    if manifest.schema != 1
        || manifest.platform != expected_platform
        || current_database_schema != manifest.database_schema
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
                phase: "staging".into(),
                previous: PathBuf::from("previous-1.0.0"),
                previous_version: "1.0.0".into(),
                candidate: "2.0.0".into(),
                backup: None,
                authority_id: None,
                managed_service: false,
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
                phase: "backup_verified".into(),
                previous: PathBuf::from("previous-1.0.0"),
                previous_version: "1.0.0".into(),
                candidate: "2.0.0".into(),
                backup: Some(PathBuf::from("backup.tar.gz")),
                authority_id: Some(uuid::Uuid::nil()),
                managed_service: true,
            },
        )
        .await
        .unwrap();

        let remaining = recover_staging_journal(&journal_path, &root)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(remaining.phase, "backup_verified");
        assert!(journal_path.exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
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

    #[test]
    fn signed_manifest_verifies_and_tampering_fails() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let manifest = serde_json::to_vec(&StandaloneManifest {
            schema: 1,
            version: "1.2.3".into(),
            platform: "darwin-arm64".into(),
            database_schema: 2,
            attach_protocol_min: 1,
            attach_protocol_max: 1,
            archive_sha256: "00".repeat(32),
            archive: "bobe.tar.gz".into(),
        })
        .unwrap();
        let signature =
            base64::engine::general_purpose::STANDARD.encode(signing.sign(&manifest).to_bytes());
        let public =
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());
        assert!(
            verify_manifest_with_key(&manifest, &signature, &public, "darwin-arm64", 2).is_ok()
        );
        let mut tampered = manifest;
        tampered.push(b' ');
        assert!(
            verify_manifest_with_key(&tampered, &signature, &public, "darwin-arm64", 2).is_err()
        );
    }
}
