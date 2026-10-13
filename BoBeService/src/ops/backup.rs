use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Connection;

use crate::error::AppError;
use crate::util::paths::DataPaths;
use crate::util::text::hex_encode;

const DATABASE_ARTIFACT: &str = "database/data.sqlite3";
const CONFIG_ARTIFACT: &str = "config/config.toml";
const DATA_PREFIX: &str = "data/";
const AGENT_SESSION_DIRECTORY: &str = "agent";
const ROOT_FILES: &[&str] = &[
    "memory.md",
    "mcp.json",
    "secrets.json",
    "headless-secrets.json",
    "owner-api-token",
];
const ROOT_DIRECTORIES: &[&str] = &["goals", "skills", "workspace"];
const TLS_DIRECTORY: &str = "tls";
const TLS_FILES: &[&str] = &[
    crate::util::paths::AUTOMATIC_TLS_CERTIFICATE_FILE,
    crate::util::paths::AUTOMATIC_TLS_PRIVATE_KEY_FILE,
    crate::util::paths::AUTOMATIC_TLS_METADATA_FILE,
];
const CURRENT_BACKUP_SCHEMA: u32 = 6;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct BackupManifest {
    pub(crate) schema: u32,
    pub(crate) database_schema: i64,
    pub(crate) runtime_authority_id: uuid::Uuid,
    pub(crate) runtime_authority_epoch: i64,
    pub(crate) created_at: chrono::DateTime<chrono::Utc>,
    pub(crate) files: Vec<BackupFile>,
    pub(crate) reauthentication_required: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct BackupFile {
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
}

pub(crate) async fn create_live(
    pool: &sqlx::SqlitePool,
    paths: &DataPaths,
    config_file: &Path,
    authority_id: uuid::Uuid,
    authority_epoch: i64,
    database_schema: i64,
) -> Result<PathBuf, AppError> {
    let database = paths
        .database_file()
        .ok_or_else(|| AppError::Config("backup requires a file-backed SQLite database".into()))?;
    let snapshot = database.with_extension(format!("bobe-snapshot-{}", uuid::Uuid::new_v4()));
    let escaped = snapshot.to_string_lossy().replace('\'', "''");
    sqlx::query(sqlx::AssertSqlSafe(format!("VACUUM INTO '{escaped}'")))
        .execute(pool)
        .await?;
    let result = create_with_database(
        paths,
        config_file,
        authority_id,
        authority_epoch,
        database_schema,
        &snapshot,
    )
    .await;
    drop(crate::util::durable_fs::durable_remove_file(&snapshot).await);
    result
}

pub(crate) async fn create(
    paths: &DataPaths,
    config_file: &Path,
    authority_id: uuid::Uuid,
    authority_epoch: i64,
    database_schema: i64,
) -> Result<PathBuf, AppError> {
    let database = paths
        .database_file()
        .ok_or_else(|| AppError::Config("backup requires a file-backed SQLite database".into()))?;
    create_with_database(
        paths,
        config_file,
        authority_id,
        authority_epoch,
        database_schema,
        &database,
    )
    .await
}

async fn create_with_database(
    paths: &DataPaths,
    config_file: &Path,
    authority_id: uuid::Uuid,
    authority_epoch: i64,
    database_schema: i64,
    database: &Path,
) -> Result<PathBuf, AppError> {
    let backups = paths.backups_dir();
    crate::util::paths::prepare_private_directory(&backups, "owner backups")?;
    let name = format!(
        "backup-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
        uuid::Uuid::new_v4().simple()
    );
    let staging = backups.join(format!(".{name}.tmp"));
    let published = backups.join(name);
    crate::util::paths::prepare_private_directory(&staging, "backup staging")?;

    let result = async {
        let mut files = Vec::new();
        copy_file_no_follow(database, &staging, DATABASE_ARTIFACT, &mut files).await?;
        if config_file.is_file() {
            copy_file_no_follow(config_file, &staging, CONFIG_ARTIFACT, &mut files).await?;
        }
        for name in ROOT_FILES {
            let source = paths.root().join(name);
            if tokio::fs::try_exists(&source).await? {
                copy_file_no_follow(
                    &source,
                    &staging,
                    &format!("{DATA_PREFIX}{name}"),
                    &mut files,
                )
                .await?;
            }
        }
        for name in ROOT_DIRECTORIES {
            let source = paths.root().join(name);
            if tokio::fs::try_exists(&source).await? {
                copy_tree_no_follow(paths.root(), &source, &staging, &mut files).await?;
            }
        }
        let agent = paths.agent_dir();
        if tokio::fs::try_exists(&agent).await? {
            paths.validate_agent_runtime_tree()?;
            for relative in ["agent/session.json"] {
                if !paths.root().join(relative).is_file() {
                    continue;
                }
                copy_file_no_follow(
                    &paths.root().join(relative),
                    &staging,
                    &format!("{DATA_PREFIX}{relative}"),
                    &mut files,
                )
                .await?;
            }
            if agent.join("sessions").is_dir() {
                copy_tree_no_follow(paths.root(), &agent.join("sessions"), &staging, &mut files)
                    .await?;
            }
        }
        let tls = paths.tls_dir();
        if tokio::fs::try_exists(&tls).await? {
            ensure_private_directory(&staging.join(DATA_PREFIX).join(TLS_DIRECTORY), true).await?;
            for name in TLS_FILES {
                let source = tls.join(name);
                if !source.is_file() {
                    return Err(AppError::Config(format!(
                        "automatic TLS identity is incomplete: {} is missing",
                        source.display()
                    )));
                }
                copy_file_no_follow(
                    &source,
                    &staging,
                    &format!("{DATA_PREFIX}{TLS_DIRECTORY}/{name}"),
                    &mut files,
                )
                .await?;
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = BackupManifest {
            schema: CURRENT_BACKUP_SCHEMA,
            database_schema,
            runtime_authority_id: authority_id,
            runtime_authority_epoch: authority_epoch,
            created_at: chrono::Utc::now(),
            files,
            reauthentication_required: vec![
                "GUI endpoint pairing (Data Protection Keychain)".into(),
                "Provider subscription sign-in (authority-root credential files)".into(),
            ],
        };
        crate::util::durable_fs::atomic_write(
            &staging.join("manifest.json"),
            &serde_json::to_vec_pretty(&manifest)?,
        )
        .await?;
        crate::util::paths::validate_private_tree(&staging, "owner backup")?;
        crate::util::durable_fs::sync_tree(&staging)?;
        tokio::fs::rename(&staging, &published).await?;
        crate::util::durable_fs::sync_directory(&backups).await?;
        Ok::<_, AppError>(published.clone())
    }
    .await;
    if result.is_err() {
        drop(crate::util::durable_fs::durable_remove_dir_all(&staging).await);
    }
    result
}

pub(crate) async fn verify(path: &Path, expected_schema: i64) -> Result<BackupManifest, AppError> {
    reject_symlink(path).await?;
    let manifest: BackupManifest =
        serde_json::from_slice(&tokio::fs::read(path.join("manifest.json")).await?)?;
    if manifest.schema != CURRENT_BACKUP_SCHEMA || manifest.database_schema != expected_schema {
        return Err(AppError::Conflict(format!(
            "backup format {} or database schema {} is not compatible with this daemon",
            manifest.schema, manifest.database_schema
        )));
    }
    let mut seen = BTreeSet::new();
    for file in &manifest.files {
        if !seen.insert(file.path.clone()) || !is_known_artifact(&file.path) {
            return Err(AppError::Config(format!(
                "unknown or duplicate backup artifact: {}",
                file.path
            )));
        }
        let target = safe_backup_path(path, &file.path)?;
        reject_symlink_ancestors(path, &target).await?;
        let bytes = read_regular_no_follow(&target).await?;
        if bytes.len() as u64 != file.bytes || hex_encode(&Sha256::digest(&bytes)) != file.sha256 {
            return Err(AppError::Conflict(format!(
                "backup file failed verification: {}",
                file.path
            )));
        }
    }
    if !seen.contains(DATABASE_ARTIFACT) {
        return Err(AppError::Conflict("backup has no database artifact".into()));
    }
    validate_canonical_session_artifacts(path, &seen).await?;
    let tls_artifacts = TLS_FILES
        .iter()
        .map(|name| format!("{DATA_PREFIX}{TLS_DIRECTORY}/{name}"))
        .collect::<Vec<_>>();
    if tls_artifacts.iter().any(|path| seen.contains(path))
        && !tls_artifacts.iter().all(|path| seen.contains(path))
    {
        return Err(AppError::Conflict(
            "backup has an incomplete automatic TLS identity".into(),
        ));
    }
    validate_database(&path.join(DATABASE_ARTIFACT), &manifest, expected_schema).await?;
    Ok(manifest)
}

async fn validate_canonical_session_artifacts(
    backup: &Path,
    files: &BTreeSet<String>,
) -> Result<(), AppError> {
    let pointer_artifact = format!("{DATA_PREFIX}{AGENT_SESSION_DIRECTORY}/session.json");
    if !files.contains(&pointer_artifact) {
        if files
            .iter()
            .any(|file| file.starts_with("data/agent/sessions/"))
        {
            return Err(AppError::Conflict(
                "backup has pi transcripts without a canonical pointer".into(),
            ));
        }
        return Ok(());
    }
    validate_session_pointer(backup, &pointer_artifact, files).await?;
    for artifact in files {
        if artifact.starts_with("data/agent/sessions/") && artifact.ends_with("/session.json") {
            validate_session_pointer(backup, artifact, files).await?;
        }
    }
    Ok(())
}

async fn validate_session_pointer(
    backup: &Path,
    pointer_artifact: &str,
    files: &BTreeSet<String>,
) -> Result<(), AppError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Pointer {
        version: u32,
        session_id: uuid::Uuid,
        file: String,
    }
    let pointer: Pointer = serde_json::from_slice(
        &read_regular_no_follow(&safe_backup_path(backup, pointer_artifact)?).await?,
    )?;
    let _safe = safe_backup_path(backup, &pointer.file)?;
    let relative = Path::new(&pointer.file);
    let is_canonical = relative.parent() == Some(Path::new("sessions"))
        && relative
            .extension()
            .is_some_and(|extension| extension == "jsonl");
    let parent = Path::new(pointer_artifact)
        .parent()
        .ok_or_else(|| AppError::Config("session pointer has no containing directory".into()))?;
    let expected = format!("{}/{}", parent.display(), pointer.file);
    if pointer.version != 1
        || pointer.session_id.is_nil()
        || !is_canonical
        || !files.contains(&expected)
    {
        return Err(AppError::Conflict(
            "backup canonical pi session pointer has no matching transcript".into(),
        ));
    }
    let transcript = read_regular_no_follow(&safe_backup_path(backup, &expected)?).await?;
    let header: serde_json::Value = serde_json::from_slice(
        transcript
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default(),
    )?;
    if header.get("type").and_then(serde_json::Value::as_str) != Some("session")
        || header
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            != Some(pointer.session_id)
    {
        return Err(AppError::Conflict(
            "backup pi transcript identity differs from its pointer".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct RestoreOutcome {
    pub(crate) manifest: BackupManifest,
    pub(crate) restored_authority_epoch: i64,
    pub(crate) cleanup_pending: bool,
}

pub(crate) async fn verify_restore_topology(
    backup: &Path,
    current_paths: &DataPaths,
    expected_schema: i64,
) -> Result<BackupManifest, AppError> {
    let manifest = verify(backup, expected_schema).await?;
    let config_artifact = backup.join(CONFIG_ARTIFACT);
    if config_artifact.is_file() {
        let loaded = crate::config::Config::load_from(&config_artifact, current_paths.root())?;
        if !same_existing_path(loaded.paths.root(), current_paths.root())
            || !same_database(&loaded.paths, current_paths)
        {
            return Err(AppError::Conflict(format!(
                "backup configuration would relocate the BoBe data root or database (current root '{}', backup root '{}'; current database '{}', backup database '{}')",
                current_paths.root().display(),
                loaded.paths.root().display(),
                current_paths.database_url(),
                loaded.paths.database_url(),
            )));
        }
    }
    Ok(manifest)
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .and_then(|left| std::fs::canonicalize(right).map(|right| left == right))
            .unwrap_or(false)
}

fn same_database(left: &DataPaths, right: &DataPaths) -> bool {
    let Some(left_file) = left.database_file() else {
        return left.database_url() == right.database_url();
    };
    let Some(right_file) = right.database_file() else {
        return false;
    };
    let left_query = left.database_url().split_once('?').map(|(_, query)| query);
    let right_query = right.database_url().split_once('?').map(|(_, query)| query);
    left_query == right_query && same_existing_path(&left_file, &right_file)
}

pub(crate) async fn restore(
    backup: &Path,
    paths: &DataPaths,
    config_file: &Path,
    expected_schema: i64,
) -> Result<RestoreOutcome, AppError> {
    let manifest = verify(backup, expected_schema).await?;
    let database = paths
        .database_file()
        .ok_or_else(|| AppError::Config("restore requires a file-backed SQLite database".into()))?;
    let journal_path = paths.root().join("restore-journal.json");
    recover_restore(paths, config_file, &database, &journal_path).await?;
    let (current_authority, current_epoch) = read_authority(&database).await?;
    if current_authority != manifest.runtime_authority_id {
        return Err(AppError::Conflict(
            "backup belongs to another BoBe authority; authority transfer is required".into(),
        ));
    }
    reject_destination_symlinks(paths, config_file, &database).await?;

    let transaction_id = uuid::Uuid::new_v4().simple().to_string();
    let stage_root = paths.root().join(format!(".restore-{transaction_id}"));
    tokio::fs::create_dir(&stage_root).await?;
    let preparation = async {
        for file in &manifest.files {
            let source = safe_backup_path(backup, &file.path)?;
            let staged = safe_backup_path(&stage_root, &file.path)?;
            ensure_artifact_parent(
                &stage_root,
                &file.path,
                is_session_state_artifact(&file.path),
            )
            .await?;
            write_artifact_file(&staged, &file.path, &read_regular_no_follow(&source).await?)
                .await?;
        }
        validate_database(
            &stage_root.join(DATABASE_ARTIFACT),
            &manifest,
            expected_schema,
        )
        .await?;

        let restored_epoch = current_epoch.max(manifest.runtime_authority_epoch) + 1;
        let mut replacements = prepare_replacements(
            &manifest,
            backup,
            paths,
            config_file,
            &database,
            &transaction_id,
        )
        .await?;
        let agent =
            prepare_agent_runtime_replacement(&manifest, backup, paths, &transaction_id).await?;
        replacements.push(agent);
        let staged_database = replacements
            .iter()
            .find(|replacement| replacement.target == database)
            .and_then(|replacement| replacement.staged.as_deref())
            .ok_or_else(|| AppError::Conflict("restore database stage is missing".into()))?;
        set_authority_epoch(staged_database, current_authority, restored_epoch).await?;
        validate_restored_database(
            staged_database,
            current_authority,
            restored_epoch,
            expected_schema,
        )
        .await?;
        Ok::<_, AppError>((restored_epoch, replacements))
    }
    .await;
    let (restored_epoch, replacements) = match preparation {
        Ok(prepared) => prepared,
        Err(error) => {
            drop(clean_prepared_replacements(paths, config_file, &database, &transaction_id).await);
            drop(crate::util::durable_fs::durable_remove_dir_all(&stage_root).await);
            return Err(error);
        }
    };
    let had_targets = replacements
        .iter()
        .map(|replacement| replacement.had_target)
        .collect::<Vec<_>>();
    write_restore_journal(
        &journal_path,
        &RestoreJournal {
            transaction_id: transaction_id.clone(),
            phase: "installing".into(),
            had_targets,
        },
    )
    .await?;
    let mut installed = Vec::<InstalledReplacement>::new();
    for replacement in replacements {
        if let Err(error) = install_replacement(replacement, &transaction_id, &mut installed).await
        {
            let rollback_error = rollback(&mut installed).await.err();
            return Err(AppError::Internal(match rollback_error {
                Some(rollback) => format!("restore failed: {error}; rollback failed: {rollback}"),
                None => format!("restore failed and was rolled back: {error}"),
            }));
        }
    }
    write_restore_journal(
        &journal_path,
        &RestoreJournal {
            transaction_id: transaction_id.clone(),
            phase: "installed".into(),
            had_targets: installed
                .iter()
                .map(|replacement| replacement.old.is_some())
                .collect(),
        },
    )
    .await?;
    let cleanup_pending = async {
        for replacement in installed {
            if let Some(old) = replacement.old {
                remove_entry(&old).await?;
            }
        }
        crate::util::durable_fs::durable_remove_dir_all(&stage_root).await?;
        crate::util::durable_fs::durable_remove_file(&journal_path).await?;
        Ok::<(), AppError>(())
    }
    .await
    .is_err();
    Ok(RestoreOutcome {
        manifest,
        restored_authority_epoch: restored_epoch,
        cleanup_pending,
    })
}

pub(crate) async fn list(paths: &DataPaths) -> Result<Vec<PathBuf>, AppError> {
    let mut entries = Vec::new();
    let directory = paths.backups_dir();
    if !tokio::fs::try_exists(&directory).await? {
        return Ok(entries);
    }
    let mut reader = tokio::fs::read_dir(directory).await?;
    while let Some(entry) = reader.next_entry().await? {
        if entry.file_type().await?.is_dir()
            && !entry.file_name().to_string_lossy().starts_with('.')
            && entry.path().join("manifest.json").is_file()
        {
            entries.push(entry.path());
        }
    }
    entries.sort();
    Ok(entries)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RestoreJournal {
    transaction_id: String,
    phase: String,
    had_targets: Vec<bool>,
}

#[derive(Clone)]
struct Replacement {
    staged: Option<PathBuf>,
    target: PathBuf,
    had_target: bool,
}

struct InstalledReplacement {
    target: PathBuf,
    old: Option<PathBuf>,
}

async fn write_artifact_file(path: &Path, artifact: &str, bytes: &[u8]) -> Result<(), AppError> {
    #[cfg(unix)]
    if matches!(
        artifact,
        "data/owner-api-token" | "data/secrets.json" | "data/headless-secrets.json"
    ) || is_tls_artifact(artifact)
        || is_session_state_artifact(artifact)
    {
        return crate::util::durable_fs::atomic_write_mode(path, bytes, 0o600).await;
    }
    crate::util::durable_fs::atomic_write(path, bytes).await
}

async fn clean_prepared_replacements(
    paths: &DataPaths,
    config_file: &Path,
    database: &Path,
    transaction_id: &str,
) -> Result<(), AppError> {
    let mut failures = Vec::new();
    for target in restore_targets(paths, config_file, database) {
        let staged = replacement_path(target.as_path(), "new", transaction_id)?;
        if let Err(error) = remove_entry(&staged).await {
            failures.push(format!("remove {}: {error}", staged.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Internal(failures.join("; ")))
    }
}

async fn prepare_replacements(
    manifest: &BackupManifest,
    backup: &Path,
    paths: &DataPaths,
    config_file: &Path,
    database: &Path,
    transaction_id: &str,
) -> Result<Vec<Replacement>, AppError> {
    let present = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut logical_targets = vec![
        (DATABASE_ARTIFACT.to_owned(), database.to_path_buf()),
        (
            "database-sidecar-wal".to_owned(),
            database_sidecar(database, "-wal"),
        ),
        (
            "database-sidecar-shm".to_owned(),
            database_sidecar(database, "-shm"),
        ),
        (CONFIG_ARTIFACT.to_owned(), config_file.to_path_buf()),
    ];
    for name in ROOT_FILES {
        logical_targets.push((format!("{DATA_PREFIX}{name}"), paths.root().join(name)));
    }
    for name in ROOT_DIRECTORIES {
        logical_targets.push((format!("{DATA_PREFIX}{name}"), paths.root().join(name)));
    }
    logical_targets.push((format!("{DATA_PREFIX}{TLS_DIRECTORY}"), paths.tls_dir()));

    let mut replacements = Vec::new();
    for (artifact, target) in logical_targets {
        let included = if artifact == "database-sidecar-wal" || artifact == "database-sidecar-shm" {
            false
        } else if ROOT_DIRECTORIES
            .iter()
            .copied()
            .chain(std::iter::once(TLS_DIRECTORY))
            .any(|name| artifact == format!("{DATA_PREFIX}{name}"))
        {
            present
                .iter()
                .any(|path| path.starts_with(&format!("{artifact}/")))
        } else {
            present.contains(artifact.as_str())
        };
        let staged = if included {
            let parent = target.parent().ok_or_else(|| {
                AppError::Config(format!(
                    "restore target has no parent: {}",
                    target.display()
                ))
            })?;
            tokio::fs::create_dir_all(parent).await?;
            let staged = replacement_path(&target, "new", transaction_id)?;
            remove_entry(&staged).await?;
            if ROOT_DIRECTORIES
                .iter()
                .copied()
                .chain(std::iter::once(TLS_DIRECTORY))
                .any(|name| artifact == format!("{DATA_PREFIX}{name}"))
            {
                create_private_directory(&staged, is_private_directory_artifact(&artifact)).await?;
                copy_artifact_directory(backup, &artifact, &staged, &manifest.files).await?;
                crate::util::durable_fs::sync_tree(&staged)?;
            } else {
                let source = safe_backup_path(backup, &artifact)?;
                write_artifact_file(&staged, &artifact, &read_regular_no_follow(&source).await?)
                    .await?;
            }
            Some(staged)
        } else {
            None
        };
        let had_target = tokio::fs::try_exists(&target).await?;
        replacements.push(Replacement {
            staged,
            target,
            had_target,
        });
    }
    Ok(replacements)
}

async fn prepare_agent_runtime_replacement(
    manifest: &BackupManifest,
    backup: &Path,
    paths: &DataPaths,
    transaction_id: &str,
) -> Result<Replacement, AppError> {
    let target = paths.agent_dir();
    let staged = replacement_path(&target, "new", transaction_id)?;
    remove_entry(&staged).await?;
    create_private_directory(&staged, true).await?;

    let authentication = target.join("auth");
    if tokio::fs::try_exists(&authentication).await? {
        crate::util::paths::validate_private_directory(&authentication, "pi authentication state")?;
        crate::util::paths::validate_private_tree(&authentication, "pi authentication state")?;
        copy_private_tree(&authentication, &staged.join("auth")).await?;
    }

    let prefix = format!("{DATA_PREFIX}{AGENT_SESSION_DIRECTORY}/");
    for file in manifest
        .files
        .iter()
        .filter(|file| file.path.starts_with(&prefix))
    {
        let relative = file.path.strip_prefix(&prefix).unwrap_or_default();
        let destination = safe_backup_path(&staged, relative)?;
        ensure_artifact_parent(&staged, relative, true).await?;
        let source = safe_backup_path(backup, &file.path)?;
        write_artifact_file(
            &destination,
            &file.path,
            &read_regular_no_follow(&source).await?,
        )
        .await?;
    }
    crate::util::durable_fs::sync_tree(&staged)?;
    Ok(Replacement {
        staged: Some(staged),
        had_target: tokio::fs::try_exists(&target).await?,
        target,
    })
}

async fn copy_private_tree(source: &Path, target: &Path) -> Result<(), AppError> {
    ensure_private_directory(target, true).await?;
    let mut entries = tokio::fs::read_dir(source).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.file_type().await?;
        if metadata.is_symlink() {
            return Err(AppError::Config(
                "pi authentication state contains a symlink".into(),
            ));
        }
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            Box::pin(copy_private_tree(&entry.path(), &destination)).await?;
        } else if metadata.is_file() {
            let bytes = read_regular_no_follow(&entry.path()).await?;
            #[cfg(unix)]
            crate::util::durable_fs::atomic_write_mode(&destination, &bytes, 0o600).await?;
            #[cfg(not(unix))]
            crate::util::durable_fs::atomic_write(&destination, &bytes).await?;
        } else {
            return Err(AppError::Config(
                "pi authentication state contains a non-file entry".into(),
            ));
        }
    }
    Ok(())
}

async fn ensure_private_directory(path: &Path, private: bool) -> Result<(), AppError> {
    tokio::fs::create_dir_all(path).await?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(())
}

async fn create_private_directory(path: &Path, private: bool) -> Result<(), AppError> {
    tokio::fs::create_dir(path).await?;
    ensure_private_directory(path, private).await
}

async fn ensure_artifact_parent(
    root: &Path,
    artifact: &str,
    private: bool,
) -> Result<(), AppError> {
    let relative = Path::new(artifact);
    let parent = relative
        .parent()
        .ok_or_else(|| AppError::Config(format!("backup artifact has no parent: {artifact}")))?;
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(AppError::Config("backup contains an unsafe path".into()));
        };
        current.push(component);
        ensure_private_directory(&current, private).await?;
    }
    Ok(())
}

async fn copy_artifact_directory(
    backup: &Path,
    artifact: &str,
    staged: &Path,
    files: &[BackupFile],
) -> Result<(), AppError> {
    let prefix = format!("{artifact}/");
    for file in files.iter().filter(|file| file.path.starts_with(&prefix)) {
        let relative = file.path.strip_prefix(&prefix).unwrap_or_default();
        let target = safe_backup_path(staged, relative)?;
        ensure_artifact_parent(staged, relative, is_session_state_artifact(&file.path)).await?;
        let source = safe_backup_path(backup, &file.path)?;
        write_artifact_file(&target, &file.path, &read_regular_no_follow(&source).await?).await?;
    }
    Ok(())
}

async fn install_replacement(
    replacement: Replacement,
    transaction_id: &str,
    installed: &mut Vec<InstalledReplacement>,
) -> Result<(), AppError> {
    if let Some(parent) = replacement.target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let old = replacement_path(&replacement.target, "old", transaction_id)?;
    let had_target = replacement.had_target;
    if had_target {
        tokio::fs::rename(&replacement.target, &old).await?;
        sync_parent(&replacement.target).await?;
    }
    installed.push(InstalledReplacement {
        target: replacement.target.clone(),
        old: had_target.then_some(old.clone()),
    });
    if let Some(staged) = replacement.staged
        && let Err(error) = tokio::fs::rename(staged, &replacement.target).await
    {
        return Err(error.into());
    }
    sync_parent(&replacement.target).await?;
    Ok(())
}

async fn rollback(installed: &mut Vec<InstalledReplacement>) -> Result<(), AppError> {
    let mut failures = Vec::new();
    while let Some(replacement) = installed.pop() {
        if let Err(error) = remove_entry(&replacement.target).await {
            failures.push(format!("remove {}: {error}", replacement.target.display()));
        }
        if let Some(old) = replacement.old
            && let Err(error) = tokio::fs::rename(&old, &replacement.target).await
        {
            failures.push(format!(
                "restore {} from {}: {error}",
                replacement.target.display(),
                old.display()
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Internal(failures.join("; ")))
    }
}

async fn write_restore_journal(path: &Path, journal: &RestoreJournal) -> Result<(), AppError> {
    crate::util::durable_fs::atomic_write(path, &serde_json::to_vec_pretty(journal)?).await
}

async fn recover_restore(
    paths: &DataPaths,
    config_file: &Path,
    database: &Path,
    journal_path: &Path,
) -> Result<(), AppError> {
    if !tokio::fs::try_exists(journal_path).await? {
        return Ok(());
    }
    let journal: RestoreJournal = serde_json::from_slice(&tokio::fs::read(journal_path).await?)?;
    let targets = restore_targets(paths, config_file, database);
    if journal.had_targets.len() != targets.len() {
        return Err(AppError::Conflict(
            "restore journal does not match the current artifact inventory".into(),
        ));
    }
    for (target, had_target) in targets.into_iter().zip(journal.had_targets).rev() {
        let old = replacement_path(&target, "old", &journal.transaction_id)?;
        let staged = replacement_path(&target, "new", &journal.transaction_id)?;
        if journal.phase == "installed" {
            remove_entry(&old).await?;
            remove_entry(&staged).await?;
            continue;
        }
        if tokio::fs::try_exists(&old).await? {
            remove_entry(&target).await?;
            tokio::fs::rename(&old, &target).await?;
            sync_parent(&target).await?;
        } else if !had_target {
            remove_entry(&target).await?;
        }
        remove_entry(&staged).await?;
    }
    crate::util::durable_fs::durable_remove_dir_all(
        &paths
            .root()
            .join(format!(".restore-{}", journal.transaction_id)),
    )
    .await?;
    crate::util::durable_fs::durable_remove_file(journal_path).await?;
    Ok(())
}

fn restore_targets(paths: &DataPaths, config_file: &Path, database: &Path) -> Vec<PathBuf> {
    let mut targets = vec![
        database.to_path_buf(),
        database_sidecar(database, "-wal"),
        database_sidecar(database, "-shm"),
        config_file.to_path_buf(),
    ];
    targets.extend(ROOT_FILES.iter().map(|name| paths.root().join(name)));
    targets.extend(ROOT_DIRECTORIES.iter().map(|name| paths.root().join(name)));
    targets.push(paths.tls_dir());
    targets.push(paths.agent_dir());
    targets
}

fn database_sidecar(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn replacement_path(target: &Path, kind: &str, transaction_id: &str) -> Result<PathBuf, AppError> {
    let parent = target.parent().ok_or_else(|| {
        AppError::Config(format!(
            "restore target has no parent: {}",
            target.display()
        ))
    })?;
    let file_name = target.file_name().ok_or_else(|| {
        AppError::Config(format!(
            "restore target has no file name: {}",
            target.display()
        ))
    })?;
    let mut replacement = std::ffi::OsString::from(".");
    replacement.push(file_name);
    replacement.push(format!(".bobe-{kind}-{transaction_id}"));
    Ok(parent.join(replacement))
}

async fn sync_parent(path: &Path) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Config(format!("restore target has no parent: {}", path.display()))
    })?;
    crate::util::durable_fs::sync_directory(parent).await
}

async fn validate_restored_database(
    database: &Path,
    authority: uuid::Uuid,
    epoch: i64,
    expected_schema: i64,
) -> Result<(), AppError> {
    let mut connection = read_only_database(database).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    let schema: i64 = sqlx::query_scalar("SELECT version FROM bobe_schema WHERE singleton = 1")
        .fetch_one(&mut connection)
        .await?;
    let (authority_bytes, actual_epoch): (Vec<u8>, i64) = sqlx::query_as(
        "SELECT authority_id, authority_epoch FROM runtime_authority WHERE singleton = 1",
    )
    .fetch_one(&mut connection)
    .await?;
    connection.close().await?;
    if integrity != "ok"
        || schema != expected_schema
        || uuid::Uuid::from_slice(&authority_bytes).ok() != Some(authority)
        || actual_epoch != epoch
    {
        return Err(AppError::Conflict(
            "staged restored database failed fencing validation".into(),
        ));
    }
    Ok(())
}

async fn validate_database(
    database: &Path,
    manifest: &BackupManifest,
    expected_schema: i64,
) -> Result<(), AppError> {
    let mut connection = read_only_database(database).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    if integrity != "ok" {
        return Err(AppError::Conflict(format!(
            "backup database integrity failed: {integrity}"
        )));
    }
    let foreign_keys: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&mut connection)
        .await?;
    let schema: i64 = sqlx::query_scalar("SELECT version FROM bobe_schema WHERE singleton = 1")
        .fetch_one(&mut connection)
        .await?;
    let (authority_bytes, epoch): (Vec<u8>, i64) = sqlx::query_as(
        "SELECT authority_id, authority_epoch FROM runtime_authority WHERE singleton = 1",
    )
    .fetch_one(&mut connection)
    .await?;
    connection.close().await?;
    let authority = uuid::Uuid::from_slice(&authority_bytes)
        .map_err(|error| AppError::Conflict(format!("invalid backup authority: {error}")))?;
    if foreign_keys != 0
        || schema != manifest.database_schema
        || schema != expected_schema
        || authority != manifest.runtime_authority_id
        || epoch != manifest.runtime_authority_epoch
    {
        return Err(AppError::Conflict(
            "backup database metadata does not match its manifest".into(),
        ));
    }
    Ok(())
}

async fn read_authority(database: &Path) -> Result<(uuid::Uuid, i64), AppError> {
    let mut connection = read_only_database(database).await?;
    let (bytes, epoch): (Vec<u8>, i64) = sqlx::query_as(
        "SELECT authority_id, authority_epoch FROM runtime_authority WHERE singleton = 1",
    )
    .fetch_one(&mut connection)
    .await?;
    connection.close().await?;
    Ok((
        uuid::Uuid::from_slice(&bytes).map_err(|error| AppError::Conflict(error.to_string()))?,
        epoch,
    ))
}

async fn set_authority_epoch(
    database: &Path,
    authority: uuid::Uuid,
    epoch: i64,
) -> Result<(), AppError> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(false)
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    let result = sqlx::query(
        "UPDATE runtime_authority SET authority_epoch = ?1,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
         last_incarnation_id = NULL, last_started_at = NULL WHERE singleton = 1 AND authority_id = ?2",
    )
    .bind(epoch)
    .bind(authority)
    .execute(&mut connection)
    .await?;
    connection.close().await?;
    if result.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "restored authority changed before fencing".into(),
        ));
    }
    Ok(())
}

async fn read_only_database(path: &Path) -> Result<sqlx::SqliteConnection, AppError> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .foreign_keys(true);
    Ok(sqlx::SqliteConnection::connect_with(&options).await?)
}

async fn copy_file_no_follow(
    source: &Path,
    backup: &Path,
    artifact: &str,
    files: &mut Vec<BackupFile>,
) -> Result<(), AppError> {
    reject_symlink(source).await?;
    let bytes = read_regular_no_follow(source).await?;
    let target = safe_backup_path(backup, artifact)?;
    ensure_artifact_parent(backup, artifact, is_session_state_artifact(artifact)).await?;
    write_artifact_file(&target, artifact, &bytes).await?;
    files.push(BackupFile {
        path: artifact.into(),
        sha256: hex_encode(&Sha256::digest(&bytes)),
        bytes: bytes.len() as u64,
    });
    Ok(())
}

async fn copy_tree_no_follow(
    data_root: &Path,
    source: &Path,
    backup: &Path,
    files: &mut Vec<BackupFile>,
) -> Result<(), AppError> {
    reject_symlink(source).await?;
    let relative = source
        .strip_prefix(data_root)
        .map_err(|_| AppError::Config("backup source escaped data root".into()))?;
    let artifact = format!("{DATA_PREFIX}{}", relative.to_string_lossy());
    let target = safe_backup_path(backup, &artifact)?;
    ensure_private_directory(&target, is_session_state_artifact(&artifact)).await?;
    let mut reader = tokio::fs::read_dir(source).await?;
    while let Some(entry) = reader.next_entry().await? {
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Config(format!(
                "backup refuses symlink: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            Box::pin(copy_tree_no_follow(data_root, &path, backup, files)).await?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(data_root)
                .map_err(|_| AppError::Config("backup source escaped data root".into()))?;
            copy_file_no_follow(
                &path,
                backup,
                &format!("{DATA_PREFIX}{}", relative.to_string_lossy()),
                files,
            )
            .await?;
        } else {
            return Err(AppError::Config(
                "backup refuses non-regular filesystem entries".into(),
            ));
        }
    }
    Ok(())
}

async fn reject_destination_symlinks(
    paths: &DataPaths,
    config: &Path,
    database: &Path,
) -> Result<(), AppError> {
    reject_symlink(paths.root()).await?;
    for name in ROOT_FILES
        .iter()
        .chain(ROOT_DIRECTORIES)
        .chain(std::iter::once(&TLS_DIRECTORY))
    {
        reject_symlink_ancestors(paths.root(), &paths.root().join(name)).await?;
    }
    for target in restore_targets(paths, config, database) {
        if let Some(parent) = target.parent()
            && tokio::fs::try_exists(parent).await?
        {
            reject_symlink(parent).await?;
        }
        if tokio::fs::try_exists(&target).await? {
            reject_symlink(&target).await?;
        }
    }
    Ok(())
}

async fn reject_symlink_ancestors(root: &Path, target: &Path) -> Result<(), AppError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| AppError::Config("backup path escaped root".into()))?;
    let mut current = root.to_path_buf();
    reject_symlink(&current).await?;
    for component in relative.components() {
        current.push(component.as_os_str());
        if tokio::fs::try_exists(&current).await? {
            reject_symlink(&current).await?;
        }
    }
    Ok(())
}

async fn reject_symlink(path: &Path) -> Result<(), AppError> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::Config(format!(
            "symlink is not allowed: {}",
            path.display()
        )));
    }
    Ok(())
}

async fn read_regular_no_follow(path: &Path) -> Result<Vec<u8>, AppError> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AppError::Config(format!(
            "backup artifact is not a regular file: {}",
            path.display()
        )));
    }
    Ok(tokio::fs::read(path).await?)
}

async fn remove_entry(path: &Path) -> Result<(), AppError> {
    if !tokio::fs::try_exists(path).await? {
        return Ok(());
    }
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        crate::util::durable_fs::durable_remove_dir_all(path).await?;
    } else {
        crate::util::durable_fs::durable_remove_file(path).await?;
    }
    Ok(())
}

fn is_session_state_artifact(path: &str) -> bool {
    path == format!("{DATA_PREFIX}{AGENT_SESSION_DIRECTORY}")
        || path.starts_with(&format!("{DATA_PREFIX}{AGENT_SESSION_DIRECTORY}/"))
}

fn is_private_directory_artifact(path: &str) -> bool {
    path == format!("{DATA_PREFIX}{TLS_DIRECTORY}") || is_session_state_artifact(path)
}

fn is_tls_artifact(path: &str) -> bool {
    path.starts_with(&format!("{DATA_PREFIX}{TLS_DIRECTORY}/"))
}

fn is_known_artifact(path: &str) -> bool {
    path == DATABASE_ARTIFACT
        || path == CONFIG_ARTIFACT
        || ROOT_FILES
            .iter()
            .any(|name| path == format!("{DATA_PREFIX}{name}"))
        || ROOT_DIRECTORIES
            .iter()
            .any(|name| path.starts_with(&format!("{DATA_PREFIX}{name}/")))
        || path == format!("{DATA_PREFIX}{AGENT_SESSION_DIRECTORY}/session.json")
        || path.starts_with(&format!("{DATA_PREFIX}{AGENT_SESSION_DIRECTORY}/sessions/"))
        || TLS_FILES
            .iter()
            .any(|name| path == format!("{DATA_PREFIX}{TLS_DIRECTORY}/{name}"))
}

fn safe_backup_path(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(AppError::Config("backup contains an unsafe path".into()));
    }
    Ok(root.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backup_accepts_the_actual_pi_worker_pointer_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../BoBeAgent/fixtures/host-contract-v1.json"
        ))
        .unwrap();
        let root = std::env::temp_dir().join(format!("bobe-pi-contract-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(root.join("data/agent"))
            .await
            .unwrap();
        let pointer = "data/agent/session.json".to_owned();
        let content = &fixture["session_pointer_example"];
        tokio::fs::write(root.join(&pointer), serde_json::to_vec(content).unwrap())
            .await
            .unwrap();
        let file = content["file"].as_str().unwrap();
        let transcript = root.join("data/agent").join(file);
        tokio::fs::create_dir_all(transcript.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &transcript,
            serde_json::to_vec(&serde_json::json!({
                "type":"session","id":content["session_id"],
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let files = BTreeSet::from([pointer, format!("data/agent/{file}")]);
        let result = validate_canonical_session_artifacts(&root, &files).await;
        tokio::fs::remove_dir_all(root).await.unwrap();
        result.unwrap();
    }

    #[cfg(unix)]
    fn assert_private_mode(path: &Path, expected: u32) {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            expected,
            "unexpected mode for {}",
            path.display()
        );
    }

    #[tokio::test]
    async fn real_backup_restores_pi_and_product_state_but_keeps_current_authentication() {
        let root = std::env::temp_dir().join(format!("bobe-pi-backup-{}", uuid::Uuid::new_v4()));
        let data = root.join("authority");
        let database = root.join("external/database.sqlite3");
        tokio::fs::create_dir_all(database.parent().unwrap())
            .await
            .unwrap();
        let paths = DataPaths::resolve(&data, &format!("sqlite:{}", database.display()))
            .unwrap()
            .prepare_root()
            .unwrap();
        crate::agent::state::prepare(&paths).unwrap();
        for directory in ["goals", "skills/chat", "workspace", "tls"] {
            tokio::fs::create_dir_all(data.join(directory))
                .await
                .unwrap();
        }
        let pointer = br#"{"version":1,"session_id":"019a3a19-4410-7000-8000-000000000001","file":"sessions/main.jsonl"}"#;
        let transcript = br#"{"type":"session","id":"019a3a19-4410-7000-8000-000000000001"}"#;
        for (name, bytes) in [
            ("memory.md", b"memory-before".as_slice()),
            ("goals/old.md", b"goal-before".as_slice()),
            ("skills/chat/SKILL.md", b"skill-before".as_slice()),
            ("workspace/notes.md", b"workspace-before".as_slice()),
            ("agent/session.json", pointer.as_slice()),
            ("agent/sessions/main.jsonl", transcript.as_slice()),
            (
                "agent/sessions/channels/00000000-0000-0000-0000-000000000001/session.json",
                pointer.as_slice(),
            ),
            (
                "agent/sessions/channels/00000000-0000-0000-0000-000000000001/sessions/main.jsonl",
                transcript.as_slice(),
            ),
            ("agent/auth/openai.json", b"old-auth".as_slice()),
            ("tls/server-cert.pem", b"certificate-before".as_slice()),
            ("tls/server-key.pem", b"key-before".as_slice()),
            ("tls/identity.json", b"identity-before".as_slice()),
        ] {
            crate::util::durable_fs::atomic_write(&data.join(name), bytes)
                .await
                .unwrap();
        }
        let authority = uuid::Uuid::new_v4();
        let mut connection = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&database)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/schema.sql"))
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO runtime_authority (singleton, authority_id, authority_epoch, data_root) VALUES (1, ?1, 7, ?2)",
        ).bind(authority).bind(data.to_string_lossy().as_ref())
            .execute(&mut connection).await.unwrap();
        sqlx::query(
            "INSERT INTO bobe_schema (singleton, version, daemon_version) VALUES (1, 2, 'test')",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        connection.close().await.unwrap();
        let config = data.join("config.toml");
        tokio::fs::write(&config, "").await.unwrap();
        let backup = create(&paths, &config, authority, 7, 2).await.unwrap();
        let manifest = verify(&backup, 2).await.unwrap();
        assert_eq!(manifest.schema, CURRENT_BACKUP_SCHEMA);
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.path == "data/agent/session.json")
        );
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.path == "data/agent/sessions/main.jsonl")
        );
        assert!(
            !manifest
                .files
                .iter()
                .any(|file| file.path.starts_with("data/agent/auth/"))
        );
        assert!(!backup.join("data/agent/auth").exists());
        #[cfg(unix)]
        {
            assert_private_mode(&paths.backups_dir(), 0o700);
            assert_private_mode(&backup, 0o700);
            for artifact in &manifest.files {
                let path = backup.join(&artifact.path);
                assert_private_mode(&path, 0o600);
                let mut parent = path.parent().unwrap();
                while parent != backup {
                    assert_private_mode(parent, 0o700);
                    parent = parent.parent().unwrap();
                }
            }
            assert_private_mode(&backup.join("manifest.json"), 0o600);
        }
        for name in ["agent/session.json", "agent/sessions/main.jsonl"] {
            #[cfg(unix)]
            assert_private_mode(&backup.join("data").join(name), 0o600);
        }
        for name in [
            "memory.md",
            "skills/chat/SKILL.md",
            "workspace/notes.md",
            "agent/session.json",
            "agent/sessions/main.jsonl",
            "tls/server-cert.pem",
        ] {
            tokio::fs::write(data.join(name), "changed").await.unwrap();
        }
        tokio::fs::write(data.join("goals/new.md"), "new")
            .await
            .unwrap();
        tokio::fs::write(data.join("agent/sessions/stale.jsonl"), "stale")
            .await
            .unwrap();
        crate::util::durable_fs::atomic_write(
            &data.join("agent/auth/openai.json"),
            b"current-auth",
        )
        .await
        .unwrap();
        let wal = database_sidecar(&database, "-wal");
        let shm = database_sidecar(&database, "-shm");
        tokio::fs::write(&wal, b"stale-wal").await.unwrap();
        tokio::fs::write(&shm, b"stale-shm").await.unwrap();
        let outcome = restore(&backup, &paths, &config, 2).await.unwrap();
        assert_eq!(outcome.restored_authority_epoch, 8);
        for (name, expected) in [
            ("memory.md", b"memory-before".as_slice()),
            ("goals/old.md", b"goal-before".as_slice()),
            ("skills/chat/SKILL.md", b"skill-before".as_slice()),
            ("workspace/notes.md", b"workspace-before".as_slice()),
            ("agent/session.json", pointer.as_slice()),
            ("agent/sessions/main.jsonl", transcript.as_slice()),
            (
                "agent/sessions/channels/00000000-0000-0000-0000-000000000001/session.json",
                pointer.as_slice(),
            ),
            (
                "agent/sessions/channels/00000000-0000-0000-0000-000000000001/sessions/main.jsonl",
                transcript.as_slice(),
            ),
            ("agent/auth/openai.json", b"current-auth".as_slice()),
            ("tls/server-cert.pem", b"certificate-before".as_slice()),
            ("tls/server-key.pem", b"key-before".as_slice()),
            ("tls/identity.json", b"identity-before".as_slice()),
        ] {
            assert_eq!(
                tokio::fs::read(data.join(name)).await.unwrap(),
                expected,
                "{name}"
            );
        }
        assert!(!data.join("goals/new.md").exists());
        assert!(!data.join("agent/sessions/stale.jsonl").exists());
        assert!(!wal.exists());
        assert!(!shm.exists());
        #[cfg(unix)]
        {
            assert_private_mode(&data.join("agent"), 0o700);
            assert_private_mode(&data.join("agent/auth"), 0o700);
            assert_private_mode(&data.join("agent/auth/openai.json"), 0o600);
            assert_private_mode(&data.join("agent/sessions/main.jsonl"), 0o600);
        }
        assert_eq!(read_authority(&database).await.unwrap().1, 8);
        let mut old_format = manifest;
        old_format.schema = CURRENT_BACKUP_SCHEMA - 1;
        crate::util::durable_fs::atomic_write(
            &backup.join("manifest.json"),
            &serde_json::to_vec(&old_format).unwrap(),
        )
        .await
        .unwrap();
        assert!(verify(&backup, 2).await.is_err());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn restore_preflight_rejects_storage_topology_change() {
        let root =
            std::env::temp_dir().join(format!("bobe-topology-test-{}", uuid::Uuid::new_v4()));
        let data = root.join("data-root");
        let database = data.join("data/bobrust.db");
        tokio::fs::create_dir_all(database.parent().unwrap())
            .await
            .unwrap();
        let paths = DataPaths::resolve(&data, &format!("sqlite:{}", database.display()))
            .unwrap()
            .prepare_root()
            .unwrap();
        let authority = uuid::Uuid::new_v4();
        let mut connection = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&database)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/schema.sql"))
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("DELETE FROM runtime_authority")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO runtime_authority (singleton, authority_id, authority_epoch, data_root)
             VALUES (1, ?1, 1, ?2)",
        )
        .bind(authority)
        .bind(data.to_string_lossy().as_ref())
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::query("INSERT OR REPLACE INTO bobe_schema (singleton, version, daemon_version) VALUES (1, 2, 'test')")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        let config = root.join("bootstrap/config.toml");
        tokio::fs::create_dir_all(config.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &config,
            format!("data_dir = '{}'\n", root.join("other-root").display()),
        )
        .await
        .unwrap();
        let backup = create(&paths, &config, authority, 1, 2).await.unwrap();

        let error = verify_restore_topology(&backup, &paths, 2)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("relocate"));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn interrupted_restore_recovers_old_database_before_retry() {
        let root = std::env::temp_dir().join(format!("bobe-restore-test-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let database = data.join("bobe.sqlite3");
        let config = root.join("config/config.toml");
        tokio::fs::create_dir_all(&data).await.unwrap();
        tokio::fs::create_dir_all(config.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&database, b"old-database").await.unwrap();
        tokio::fs::write(&config, b"old-config").await.unwrap();
        let paths = DataPaths::resolve(&data, &format!("sqlite:{}", database.display()))
            .unwrap()
            .prepare_root()
            .unwrap();
        let transaction_id = "crash";
        let old_database = replacement_path(&database, "old", transaction_id).unwrap();
        let old_config = replacement_path(&config, "old", transaction_id).unwrap();
        tokio::fs::rename(&database, &old_database).await.unwrap();
        tokio::fs::rename(&config, &old_config).await.unwrap();
        let journal_path = data.join("restore-journal.json");
        let targets = restore_targets(&paths, &config, &database);
        let mut had_targets = vec![false; targets.len()];
        had_targets[targets
            .iter()
            .position(|target| target == &database)
            .unwrap()] = true;
        had_targets[targets.iter().position(|target| target == &config).unwrap()] = true;
        write_restore_journal(
            &journal_path,
            &RestoreJournal {
                transaction_id: transaction_id.into(),
                phase: "installing".into(),
                had_targets,
            },
        )
        .await
        .unwrap();

        recover_restore(&paths, &config, &database, &journal_path)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&database).await.unwrap(), b"old-database");
        assert_eq!(tokio::fs::read(&config).await.unwrap(), b"old-config");
        assert!(!old_database.exists());
        assert!(!old_config.exists());
        assert!(!journal_path.exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn completed_restore_recovery_keeps_new_target_and_cleans_old() {
        let root = std::env::temp_dir().join(format!("bobe-restore-test-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let database = data.join("bobe.sqlite3");
        let config = root.join("config/config.toml");
        tokio::fs::create_dir_all(&data).await.unwrap();
        tokio::fs::create_dir_all(config.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&database, b"new-database").await.unwrap();
        let paths = DataPaths::resolve(&data, &format!("sqlite:{}", database.display()))
            .unwrap()
            .prepare_root()
            .unwrap();
        let transaction_id = "committed";
        let old_database = replacement_path(&database, "old", transaction_id).unwrap();
        tokio::fs::write(&old_database, b"old-database")
            .await
            .unwrap();
        let journal_path = data.join("restore-journal.json");
        let targets = restore_targets(&paths, &config, &database);
        let mut had_targets = vec![false; targets.len()];
        had_targets[targets
            .iter()
            .position(|target| target == &database)
            .unwrap()] = true;
        write_restore_journal(
            &journal_path,
            &RestoreJournal {
                transaction_id: transaction_id.into(),
                phase: "installed".into(),
                had_targets,
            },
        )
        .await
        .unwrap();

        recover_restore(&paths, &config, &database, &journal_path)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&database).await.unwrap(), b"new-database");
        assert!(!old_database.exists());
        assert!(!journal_path.exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn topology_comparison_accepts_filesystem_aliases() {
        use std::os::unix::fs::symlink;

        let parent =
            std::env::temp_dir().join(format!("bobe-path-alias-test-{}", uuid::Uuid::new_v4()));
        let canonical_root = parent.join("canonical");
        let alias_root = parent.join("alias");
        std::fs::create_dir_all(canonical_root.join("data")).unwrap();
        std::fs::write(canonical_root.join("data/bobrust.db"), []).unwrap();
        symlink(&canonical_root, &alias_root).unwrap();

        let canonical = DataPaths::resolve(
            &canonical_root,
            &format!("sqlite:{}/data/bobrust.db", canonical_root.display()),
        )
        .unwrap();
        let alias = DataPaths::resolve(
            &alias_root,
            &format!("sqlite:{}/data/bobrust.db", alias_root.display()),
        )
        .unwrap();

        assert!(same_existing_path(canonical.root(), alias.root()));
        assert!(same_database(&canonical, &alias));
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn backup_paths_cannot_escape_root() {
        assert!(safe_backup_path(Path::new("/tmp/root"), "../secret").is_err());
        assert!(safe_backup_path(Path::new("/tmp/root"), "/etc/passwd").is_err());
        assert_eq!(
            safe_backup_path(Path::new("/tmp/root"), "data/goals/a.md").unwrap(),
            PathBuf::from("/tmp/root/data/goals/a.md")
        );
    }

    #[tokio::test]
    async fn pi_backup_rejects_dangling_and_escaping_session_pointers() {
        let root = std::env::temp_dir().join(format!("bobe-pi-pointer-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(root.join("data/agent"))
            .await
            .unwrap();
        let pointer = "data/agent/session.json".to_owned();
        tokio::fs::write(
            root.join(&pointer),
            br#"{"version":1,"session_id":"019a3a19-4410-7000-8000-000000000001","file":"sessions/main.jsonl"}"#,
        )
        .await
        .unwrap();
        assert!(
            validate_canonical_session_artifacts(&root, &BTreeSet::from([pointer.clone()]))
                .await
                .is_err()
        );
        let files = BTreeSet::from([pointer.clone(), "data/agent/sessions/main.jsonl".to_owned()]);
        assert!(
            validate_canonical_session_artifacts(
                &root,
                &BTreeSet::from(["data/agent/sessions/main.jsonl".to_owned()]),
            )
            .await
            .is_err()
        );
        tokio::fs::create_dir_all(root.join("data/agent/sessions"))
            .await
            .unwrap();
        tokio::fs::write(
            root.join("data/agent/sessions/main.jsonl"),
            br#"{"type":"session","id":"019a3a19-4410-7000-8000-000000000001"}"#,
        )
        .await
        .unwrap();
        assert!(
            validate_canonical_session_artifacts(&root, &files)
                .await
                .is_ok()
        );
        tokio::fs::write(
            root.join("data/agent/sessions/main.jsonl"),
            br#"{"type":"session","id":"019a3a19-4410-7000-8000-000000000002"}"#,
        )
        .await
        .unwrap();
        assert!(
            validate_canonical_session_artifacts(&root, &files)
                .await
                .is_err()
        );
        tokio::fs::write(
            root.join(&pointer),
            br#"{"version":1,"session_id":"019a3a19-4410-7000-8000-000000000001","file":"../escape.jsonl"}"#,
        )
        .await
        .unwrap();
        assert!(
            validate_canonical_session_artifacts(&root, &files)
                .await
                .is_err()
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn artifact_inventory_is_explicit() {
        assert!(is_known_artifact(DATABASE_ARTIFACT));
        assert!(is_known_artifact("data/goals/a.md"));
        assert!(is_known_artifact("data/skills/chat/SKILL.md"));
        assert!(is_known_artifact("data/workspace/notes.md"));
        assert!(is_known_artifact("data/secrets.json"));
        assert!(is_known_artifact("data/headless-secrets.json"));
        assert!(is_known_artifact("data/agent/sessions/main.jsonl"));
        assert!(!is_known_artifact("data/agent/auth/openai.json"));
        assert!(!is_known_artifact("data/models/model.onnx"));
    }
}
