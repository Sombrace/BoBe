use std::collections::BTreeSet;
use std::ops::RangeInclusive;
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
const SETTINGS_FILES: &[&str] = &["mcp.json", "secrets.json", "headless-secrets.json"];
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

pub(crate) struct SettingsLocks {
    pub(crate) config: std::sync::Arc<std::sync::Mutex<()>>,
    pub(crate) mcp: std::sync::Arc<tokio::sync::Mutex<()>>,
}

struct SettingsSnapshot {
    files: Vec<(String, Vec<u8>)>,
}

pub(crate) async fn create_live(
    pool: &sqlx::SqlitePool,
    paths: &DataPaths,
    config_file: &Path,
    locks: &SettingsLocks,
    authority_id: uuid::Uuid,
    authority_epoch: i64,
    database_schema: i64,
) -> Result<PathBuf, AppError> {
    let database = paths
        .database_file()
        .ok_or_else(|| AppError::Config("backup requires a file-backed SQLite database".into()))?;
    let snapshot = database.with_extension(format!("bobe-snapshot-{}", uuid::Uuid::new_v4()));
    sqlx::query("VACUUM INTO ?1")
        .bind(snapshot.to_string_lossy().as_ref())
        .execute(pool)
        .await?;
    let result = async {
        let settings = snapshot_settings_locked(paths, config_file, locks).await?;
        create_with_database(
            paths,
            settings,
            authority_id,
            authority_epoch,
            database_schema,
            &snapshot,
        )
        .await
    }
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
    let sources = settings_sources(paths, config_file);
    let settings = tokio::task::spawn_blocking(move || read_settings(sources))
        .await
        .map_err(|error| AppError::Internal(format!("backup settings join: {error}")))??;
    create_with_database(
        paths,
        settings,
        authority_id,
        authority_epoch,
        database_schema,
        &database,
    )
    .await
}

pub(crate) async fn create_offline(
    paths: &DataPaths,
    config_file: &Path,
) -> Result<PathBuf, AppError> {
    let database = paths
        .database_file()
        .ok_or_else(|| AppError::Config("backup requires a file-backed SQLite database".into()))?;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&database)
        .create_if_missing(false);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut connection)
        .await?;
    connection.close().await?;
    let schema = crate::bootstrap::database::read_schema_version(&database).await?;
    let (authority, epoch) = read_authority(&database).await?;
    create(paths, config_file, authority, epoch, schema).await
}

fn settings_sources(paths: &DataPaths, config_file: &Path) -> Vec<(String, PathBuf)> {
    std::iter::once((CONFIG_ARTIFACT.to_owned(), config_file.to_path_buf()))
        .chain(
            SETTINGS_FILES
                .iter()
                .map(|name| (format!("{DATA_PREFIX}{name}"), paths.root().join(name))),
        )
        .collect()
}

async fn snapshot_settings_locked(
    paths: &DataPaths,
    config_file: &Path,
    locks: &SettingsLocks,
) -> Result<SettingsSnapshot, AppError> {
    let _mcp = locks.mcp.lock().await;
    let config = std::sync::Arc::clone(&locks.config);
    let sources = settings_sources(paths, config_file);
    tokio::task::spawn_blocking(move || {
        let _config = config
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("config write lock poisoned")))?;
        read_settings(sources)
    })
    .await
    .map_err(|error| AppError::Internal(format!("backup settings join: {error}")))?
}

fn read_settings(sources: Vec<(String, PathBuf)>) -> Result<SettingsSnapshot, AppError> {
    let mut files = Vec::new();
    for (artifact, path) in sources {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() {
            return Err(AppError::Config(format!(
                "backup artifact is not a regular file: {}",
                path.display()
            )));
        }
        files.push((artifact, std::fs::read(&path)?));
    }
    Ok(SettingsSnapshot { files })
}

async fn create_with_database(
    paths: &DataPaths,
    settings: SettingsSnapshot,
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
        for (artifact, bytes) in &settings.files {
            store_artifact(&staging, artifact, bytes, &mut files).await?;
        }
        for name in ROOT_FILES
            .iter()
            .filter(|name| !SETTINGS_FILES.contains(name))
        {
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

pub(crate) async fn verify(
    path: &Path,
    database_schemas: &RangeInclusive<i64>,
) -> Result<BackupManifest, AppError> {
    reject_symlink(path).await?;
    let manifest: BackupManifest =
        serde_json::from_slice(&tokio::fs::read(path.join("manifest.json")).await?)?;
    if manifest.schema != CURRENT_BACKUP_SCHEMA
        || !database_schemas.contains(&manifest.database_schema)
    {
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
    validate_database(&path.join(DATABASE_ARTIFACT), &manifest).await?;
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
    database_schemas: &RangeInclusive<i64>,
) -> Result<BackupManifest, AppError> {
    let manifest = verify(backup, database_schemas).await?;
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
    database_schemas: &RangeInclusive<i64>,
) -> Result<RestoreOutcome, AppError> {
    restore_interruptible(backup, paths, config_file, database_schemas, |_| false).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestorePoint {
    Installing,
    MovedAside,
    Installed,
    Committed,
    Cleaned,
}

struct Checkpoints<F> {
    interrupt: F,
}

enum StepError {
    Interrupted,
    Failed(AppError),
}

impl From<AppError> for StepError {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
}

impl From<std::io::Error> for StepError {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(error.into())
    }
}

impl From<StepError> for AppError {
    fn from(error: StepError) -> Self {
        match error {
            StepError::Interrupted => AppError::Internal("restore interrupted".into()),
            StepError::Failed(error) => error,
        }
    }
}

impl<F: FnMut(RestorePoint) -> bool> Checkpoints<F> {
    fn reach(&mut self, point: RestorePoint) -> Result<(), StepError> {
        if (self.interrupt)(point) {
            return Err(StepError::Interrupted);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reconciled {
    Absent,
    Completed,
    RolledBack,
    Discarded { kept: Vec<PathBuf> },
}

impl Reconciled {
    pub(crate) fn changed_live_data(&self) -> bool {
        matches!(self, Self::Completed | Self::RolledBack)
    }
}

const RESTORE_JOURNAL: &str = "restore-journal.json";

async fn restore_interruptible(
    backup: &Path,
    paths: &DataPaths,
    config_file: &Path,
    database_schemas: &RangeInclusive<i64>,
    interrupt: impl FnMut(RestorePoint) -> bool,
) -> Result<RestoreOutcome, AppError> {
    let mut checkpoints = Checkpoints { interrupt };
    let manifest = verify(backup, database_schemas).await?;
    let database = paths
        .database_file()
        .ok_or_else(|| AppError::Config("restore requires a file-backed SQLite database".into()))?;
    reconcile_restore_journals(paths, config_file).await?;
    let journal_path = restore_journal_path(config_file)?;
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
        validate_database(&stage_root.join(DATABASE_ARTIFACT), &manifest).await?;

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
            manifest.database_schema,
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
    let mut journal = RestoreJournal {
        transaction_id: transaction_id.clone(),
        phase: "installing".into(),
        had_targets: replacements
            .iter()
            .map(|replacement| replacement.had_target)
            .collect(),
        layout: Some(RestoreLayout {
            data_root: paths.root().to_path_buf(),
            database_url: paths.database_url().to_owned(),
            stage_root: stage_root.clone(),
            targets: replacements
                .iter()
                .map(|replacement| RestoreTarget {
                    path: replacement.target.clone(),
                    had_target: replacement.had_target,
                })
                .collect(),
        }),
    };
    if let Err(error) = write_restore_journal(&journal_path, &journal).await {
        drop(clean_prepared_replacements(paths, config_file, &database, &transaction_id).await);
        drop(crate::util::durable_fs::durable_remove_dir_all(&stage_root).await);
        drop(crate::util::durable_fs::durable_remove_file(&journal_path).await);
        return Err(error);
    }
    checkpoints.reach(RestorePoint::Installing)?;
    for replacement in replacements {
        match install_replacement(replacement, &transaction_id, &mut checkpoints).await {
            Ok(()) => {}
            Err(StepError::Interrupted) => return Err(StepError::Interrupted.into()),
            Err(StepError::Failed(error)) => return Err(abort_restore(&journal_path, error).await),
        }
    }
    journal.phase = "installed".into();
    if let Err(error) = write_restore_journal(&journal_path, &journal).await {
        return Err(abort_restore(&journal_path, error).await);
    }
    checkpoints.reach(RestorePoint::Committed)?;
    let cleanup = async {
        for target in journal.layout.iter().flat_map(|layout| &layout.targets) {
            remove_entry(&replacement_path(&target.path, "old", &transaction_id)?).await?;
            checkpoints.reach(RestorePoint::Cleaned)?;
        }
        crate::util::durable_fs::durable_remove_dir_all(&stage_root).await?;
        crate::util::durable_fs::durable_remove_file(&journal_path).await?;
        Ok::<(), StepError>(())
    }
    .await;
    let cleanup_pending = match cleanup {
        Ok(()) => false,
        Err(StepError::Interrupted) => return Err(StepError::Interrupted.into()),
        Err(StepError::Failed(_)) => true,
    };
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<RestoreLayout>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RestoreLayout {
    data_root: PathBuf,
    database_url: String,
    stage_root: PathBuf,
    targets: Vec<RestoreTarget>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RestoreTarget {
    path: PathBuf,
    had_target: bool,
}

#[derive(Clone)]
struct Replacement {
    staged: Option<PathBuf>,
    target: PathBuf,
    had_target: bool,
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
    checkpoints: &mut Checkpoints<impl FnMut(RestorePoint) -> bool>,
) -> Result<(), StepError> {
    if let Some(parent) = replacement.target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if replacement.had_target {
        let old = replacement_path(&replacement.target, "old", transaction_id)?;
        tokio::fs::rename(&replacement.target, &old).await?;
        sync_parent(&replacement.target).await?;
        checkpoints.reach(RestorePoint::MovedAside)?;
    }
    if let Some(staged) = replacement.staged {
        tokio::fs::rename(staged, &replacement.target).await?;
        sync_parent(&replacement.target).await?;
        checkpoints.reach(RestorePoint::Installed)?;
    }
    Ok(())
}

async fn write_restore_journal(path: &Path, journal: &RestoreJournal) -> Result<(), AppError> {
    crate::util::durable_fs::atomic_write(path, &serde_json::to_vec_pretty(journal)?).await
}

fn restore_journal_path(config_file: &Path) -> Result<PathBuf, AppError> {
    Ok(config_file
        .parent()
        .ok_or_else(|| AppError::Config("config file has no parent directory".into()))?
        .join(RESTORE_JOURNAL))
}

pub(crate) fn restore_journal_pending(paths: &DataPaths) -> bool {
    std::fs::symlink_metadata(paths.root().join(RESTORE_JOURNAL)).is_ok()
}

async fn read_restore_journal(path: &Path) -> Result<Option<RestoreJournal>, AppError> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn reconcile_interrupted_restore(
    config_root: &Path,
) -> Result<Reconciled, AppError> {
    let journal_path = config_root.join(RESTORE_JOURNAL);
    let Some(layout) = read_restore_journal(&journal_path)
        .await?
        .and_then(|journal| journal.layout)
    else {
        return Ok(Reconciled::Absent);
    };
    let paths = DataPaths::resolve(&layout.data_root, &layout.database_url)?;
    let _locks = crate::util::runtime_lock::RuntimeLocks::acquire(&paths).map_err(|error| {
        AppError::Conflict(format!(
            "an interrupted restore cannot be reconciled while another BoBe process owns its data: {error}"
        ))
    })?;
    reconcile_journal(&journal_path, None).await
}

pub(crate) async fn reconcile_restore_journals(
    paths: &DataPaths,
    config_file: &Path,
) -> Result<Reconciled, AppError> {
    let database = paths
        .database_file()
        .ok_or_else(|| AppError::Config("restore requires a file-backed SQLite database".into()))?;
    let legacy = LegacyLayout {
        paths,
        config_file,
        database: &database,
    };
    let configured = reconcile_journal(&restore_journal_path(config_file)?, Some(&legacy)).await?;
    let legacy = reconcile_journal(&paths.root().join(RESTORE_JOURNAL), Some(&legacy)).await?;
    Ok(if configured == Reconciled::Absent {
        legacy
    } else {
        configured
    })
}

struct LegacyLayout<'a> {
    paths: &'a DataPaths,
    config_file: &'a Path,
    database: &'a Path,
}

async fn abort_restore(journal_path: &Path, error: AppError) -> AppError {
    match reconcile_journal(journal_path, None).await {
        Ok(Reconciled::RolledBack) => {
            AppError::Internal(format!("restore failed and was rolled back: {error}"))
        }
        Ok(_) => AppError::Internal(format!(
            "restore was committed but could not be confirmed: {error}"
        )),
        Err(rollback) => AppError::Internal(format!(
            "restore failed: {error}; rollback failed: {rollback}; BoBe rolls it back the next time it starts"
        )),
    }
}

async fn reconcile_journal(
    journal_path: &Path,
    legacy: Option<&LegacyLayout<'_>>,
) -> Result<Reconciled, AppError> {
    let Some(journal) = read_restore_journal(journal_path).await? else {
        return Ok(Reconciled::Absent);
    };
    let Some(layout) = journal.layout.as_ref() else {
        return match legacy {
            Some(legacy) => discard_legacy_journal(journal_path, &journal, legacy).await,
            None => Ok(Reconciled::Absent),
        };
    };
    let transaction_id = journal.transaction_id.as_str();
    let committed = journal.phase == "installed";
    for target in layout.targets.iter().rev() {
        let old = replacement_path(&target.path, "old", transaction_id)?;
        if committed {
            remove_entry(&old).await?;
        } else if entry_exists(&old) {
            remove_entry(&target.path).await?;
            tokio::fs::rename(&old, &target.path).await?;
            sync_parent(&target.path).await?;
        } else if !target.had_target {
            remove_entry(&target.path).await?;
        }
        remove_entry(&replacement_path(&target.path, "new", transaction_id)?).await?;
    }
    crate::util::durable_fs::durable_remove_dir_all(&layout.stage_root).await?;
    crate::util::durable_fs::durable_remove_file(journal_path).await?;
    Ok(if committed {
        Reconciled::Completed
    } else {
        Reconciled::RolledBack
    })
}

async fn discard_legacy_journal(
    journal_path: &Path,
    journal: &RestoreJournal,
    legacy: &LegacyLayout<'_>,
) -> Result<Reconciled, AppError> {
    let transaction_id = journal.transaction_id.as_str();
    let mut kept = Vec::new();
    for target in restore_targets(legacy.paths, legacy.config_file, legacy.database) {
        remove_entry(&replacement_path(&target, "new", transaction_id)?).await?;
        let old = replacement_path(&target, "old", transaction_id)?;
        if entry_exists(&old) {
            kept.push(old);
        }
    }
    crate::util::durable_fs::durable_remove_dir_all(
        &legacy
            .paths
            .root()
            .join(format!(".restore-{transaction_id}")),
    )
    .await?;
    crate::util::durable_fs::durable_remove_file(journal_path).await?;
    if !kept.is_empty() {
        tracing::warn!(?kept, "backup.stale_restore_copies_kept");
    }
    Ok(Reconciled::Discarded { kept })
}

fn entry_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
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
    database_schema: i64,
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
        || schema != database_schema
        || uuid::Uuid::from_slice(&authority_bytes).ok() != Some(authority)
        || actual_epoch != epoch
    {
        return Err(AppError::Conflict(
            "staged restored database failed fencing validation".into(),
        ));
    }
    Ok(())
}

async fn validate_database(database: &Path, manifest: &BackupManifest) -> Result<(), AppError> {
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
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1 AND authority_id = ?2",
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
    store_artifact(backup, artifact, &bytes, files).await
}

async fn store_artifact(
    backup: &Path,
    artifact: &str,
    bytes: &[u8],
    files: &mut Vec<BackupFile>,
) -> Result<(), AppError> {
    let target = safe_backup_path(backup, artifact)?;
    ensure_artifact_parent(backup, artifact, is_session_state_artifact(artifact)).await?;
    write_artifact_file(&target, artifact, bytes).await?;
    files.push(BackupFile {
        path: artifact.into(),
        sha256: hex_encode(&Sha256::digest(bytes)),
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
    use crate::bootstrap::database::{
        CURRENT_SCHEMA_VERSION, OLDEST_UPGRADABLE_SCHEMA_VERSION, SUPPORTED_SCHEMA_VERSIONS,
    };

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
        sqlx::query("INSERT INTO bobe_schema (singleton, version) VALUES (1, ?1)")
            .bind(CURRENT_SCHEMA_VERSION)
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        let config = data.join("config.toml");
        tokio::fs::write(&config, "").await.unwrap();
        let backup = create(&paths, &config, authority, 7, CURRENT_SCHEMA_VERSION)
            .await
            .unwrap();
        let manifest = verify(&backup, &SUPPORTED_SCHEMA_VERSIONS).await.unwrap();
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
        let outcome = restore(&backup, &paths, &config, &SUPPORTED_SCHEMA_VERSIONS)
            .await
            .unwrap();
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
        assert!(verify(&backup, &SUPPORTED_SCHEMA_VERSIONS).await.is_err());
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
        sqlx::query("INSERT OR REPLACE INTO bobe_schema (singleton, version) VALUES (1, ?1)")
            .bind(CURRENT_SCHEMA_VERSION)
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
        let backup = create(&paths, &config, authority, 1, CURRENT_SCHEMA_VERSION)
            .await
            .unwrap();

        let error = verify_restore_topology(&backup, &paths, &SUPPORTED_SCHEMA_VERSIONS)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("relocate"));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn a_backup_at_an_upgradable_schema_restores_and_upgrades_on_the_next_open() {
        let root =
            std::env::temp_dir().join(format!("bobe-schema-backup-{}", uuid::Uuid::new_v4()));
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
        sqlx::raw_sql(include_str!("../../migrations/fixtures/schema-9.sql"))
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO runtime_authority (singleton, authority_id, authority_epoch, data_root)
             VALUES (1, ?1, 3, ?2)",
        )
        .bind(authority)
        .bind(data.to_string_lossy().as_ref())
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::raw_sql(
            "INSERT INTO bobe_schema (singleton, version, daemon_version) VALUES (1, 9, '0.8.1');
             INSERT INTO conversations (id, state, summary) VALUES (X'c0', 'closed', 'unread');",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        connection.close().await.unwrap();
        crate::util::durable_fs::atomic_write(&data.join("skills/chat/SKILL.md"), b"owner rules")
            .await
            .unwrap();
        let config = data.join("config.toml");
        tokio::fs::write(&config, "").await.unwrap();
        let backup = create(
            &paths,
            &config,
            authority,
            3,
            OLDEST_UPGRADABLE_SCHEMA_VERSION,
        )
        .await
        .unwrap();
        let (pool, _) = crate::bootstrap::database::open(&paths).await.unwrap();
        pool.close().await;

        let outcome = restore(&backup, &paths, &config, &SUPPORTED_SCHEMA_VERSIONS)
            .await
            .unwrap();

        assert_eq!(
            outcome.manifest.database_schema,
            OLDEST_UPGRADABLE_SCHEMA_VERSION
        );
        assert_eq!(outcome.restored_authority_epoch, 4);
        assert_eq!(
            crate::bootstrap::database::read_schema_version(&database)
                .await
                .unwrap(),
            OLDEST_UPGRADABLE_SCHEMA_VERSION
        );
        assert_eq!(
            tokio::fs::read(data.join("skills/chat/SKILL.md"))
                .await
                .unwrap(),
            b"owner rules"
        );
        let (pool, version) = crate::bootstrap::database::open(&paths).await.unwrap();
        let (epoch, conversations): (i64, i64) = sqlx::query_as(
            "SELECT authority_epoch, (SELECT COUNT(*) FROM conversations) FROM runtime_authority",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        pool.close().await;
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!((epoch, conversations), (4, 1));
        assert!(!data.join("skills/chat").exists());
        assert_eq!(
            tokio::fs::read(data.join("skills/chat.retired/SKILL.md"))
                .await
                .unwrap(),
            b"owner rules"
        );

        let mut manifest = outcome.manifest;
        for database_schema in [
            OLDEST_UPGRADABLE_SCHEMA_VERSION - 1,
            CURRENT_SCHEMA_VERSION + 1,
        ] {
            manifest.database_schema = database_schema;
            crate::util::durable_fs::atomic_write(
                &backup.join("manifest.json"),
                &serde_json::to_vec(&manifest).unwrap(),
            )
            .await
            .unwrap();
            let error = verify(&backup, &SUPPORTED_SCHEMA_VERSIONS)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("not compatible"), "{error}");
        }
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn an_old_format_restore_journal_is_cleared_without_touching_live_files() {
        for phase in ["installing", "installed"] {
            let root =
                std::env::temp_dir().join(format!("bobe-restore-test-{}", uuid::Uuid::new_v4()));
            let data = root.join("data");
            let database = data.join("bobe.sqlite3");
            let config = data.join("config.toml");
            let paths = DataPaths::resolve(&data, &format!("sqlite:{}", database.display()))
                .unwrap()
                .prepare_root()
                .unwrap();
            let live = [
                (
                    database.clone(),
                    b"database-written-after-the-failed-restore".as_slice(),
                ),
                (database_sidecar(&database, "-wal"), b"wal".as_slice()),
                (config.clone(), b"[voice]\nenabled = true\n".as_slice()),
                (data.join("memory.md"), b"memory written later".as_slice()),
                (data.join("headless-secrets.json"), b"{}".as_slice()),
                (data.join("tls/server-cert.pem"), b"certificate".as_slice()),
            ];
            for (path, bytes) in &live {
                tokio::fs::create_dir_all(path.parent().unwrap())
                    .await
                    .unwrap();
                tokio::fs::write(path, bytes).await.unwrap();
            }
            let transaction_id = "stale";
            let old_config = replacement_path(&config, "old", transaction_id).unwrap();
            tokio::fs::write(&old_config, b"config before the restore")
                .await
                .unwrap();
            let new_database = replacement_path(&database, "new", transaction_id).unwrap();
            let new_goals = replacement_path(&data.join("goals"), "new", transaction_id).unwrap();
            tokio::fs::write(&new_database, b"backup database")
                .await
                .unwrap();
            tokio::fs::create_dir_all(new_goals.join("nested"))
                .await
                .unwrap();
            let stage = data.join(format!(".restore-{transaction_id}"));
            tokio::fs::create_dir_all(stage.join("data")).await.unwrap();
            let journal_path = data.join(RESTORE_JOURNAL);
            write_restore_journal(
                &journal_path,
                &RestoreJournal {
                    transaction_id: transaction_id.into(),
                    phase: phase.into(),
                    had_targets: vec![true; 3],
                    layout: None,
                },
            )
            .await
            .unwrap();

            assert_eq!(
                reconcile_interrupted_restore(&data).await.unwrap(),
                Reconciled::Absent
            );
            assert!(journal_path.exists());
            let reconciled = reconcile_restore_journals(&paths, &config).await.unwrap();

            assert_eq!(
                reconciled,
                Reconciled::Discarded {
                    kept: vec![old_config.clone()]
                },
                "{phase}"
            );
            assert!(!reconciled.changed_live_data());
            for (path, bytes) in &live {
                assert_eq!(tokio::fs::read(path).await.unwrap(), *bytes, "{phase}");
            }
            assert_eq!(
                tokio::fs::read(&old_config).await.unwrap(),
                b"config before the restore"
            );
            assert!(!new_database.exists());
            assert!(!new_goals.exists());
            assert!(!stage.exists());
            assert!(!journal_path.exists());
            assert!(!restore_journal_pending(&paths));
            tokio::fs::remove_dir_all(root).await.unwrap();
        }
    }

    struct RestoreFixture {
        root: PathBuf,
        paths: DataPaths,
        config: PathBuf,
        database: PathBuf,
        backup: PathBuf,
    }

    const TRACKED: [&str; 9] = [
        "config.toml",
        "memory.md",
        "goals/old.md",
        "goals/new.md",
        "skills/learned/notes/SKILL.md",
        "workspace/notes.md",
        "tls/server-cert.pem",
        "agent/session.json",
        "agent/sessions/main.jsonl",
    ];

    async fn restore_fixture() -> RestoreFixture {
        let root =
            std::env::temp_dir().join(format!("bobe-restore-crash-{}", uuid::Uuid::new_v4()));
        let data = root.join("authority");
        let paths = DataPaths::resolve(
            &data,
            &format!("sqlite:{}", data.join("data/bobrust.db").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let database = paths.database_file().unwrap();
        crate::agent::state::prepare(&paths).unwrap();
        let pointer = br#"{"version":1,"session_id":"019a3a19-4410-7000-8000-000000000001","file":"sessions/main.jsonl"}"#;
        let transcript = br#"{"type":"session","id":"019a3a19-4410-7000-8000-000000000001"}"#;
        for (name, bytes) in [
            ("config.toml", b"[voice]\nenabled = true\n".as_slice()),
            ("memory.md", b"memory-backup".as_slice()),
            ("goals/old.md", b"goal-backup".as_slice()),
            ("skills/learned/notes/SKILL.md", b"skill-backup".as_slice()),
            ("workspace/notes.md", b"workspace-backup".as_slice()),
            ("tls/server-cert.pem", b"certificate".as_slice()),
            ("tls/server-key.pem", b"key".as_slice()),
            ("tls/identity.json", b"identity".as_slice()),
            ("agent/session.json", pointer.as_slice()),
            ("agent/sessions/main.jsonl", transcript.as_slice()),
        ] {
            crate::util::durable_fs::atomic_write(&data.join(name), bytes)
                .await
                .unwrap();
        }
        let authority = uuid::Uuid::new_v4();
        tokio::fs::create_dir_all(database.parent().unwrap())
            .await
            .unwrap();
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
        )
        .bind(authority)
        .bind(data.to_string_lossy().as_ref())
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::query("INSERT INTO bobe_schema (singleton, version) VALUES (1, ?1)")
            .bind(CURRENT_SCHEMA_VERSION)
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        let config = data.join("config.toml");
        let backup = create(&paths, &config, authority, 7, CURRENT_SCHEMA_VERSION)
            .await
            .unwrap();
        for (name, bytes) in [
            ("config.toml", b"[voice]\nenabled = false\n".as_slice()),
            ("memory.md", b"memory-live".as_slice()),
            ("goals/old.md", b"goal-live".as_slice()),
            ("goals/new.md", b"goal-written-after-backup".as_slice()),
            ("skills/learned/notes/SKILL.md", b"skill-live".as_slice()),
            ("workspace/notes.md", b"workspace-live".as_slice()),
            ("tls/server-cert.pem", b"certificate".as_slice()),
            ("agent/sessions/main.jsonl", transcript.as_slice()),
        ] {
            crate::util::durable_fs::atomic_write(&data.join(name), bytes)
                .await
                .unwrap();
        }
        let mut connection = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new().filename(&database),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE runtime_authority SET data_root = 'live'")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        RestoreFixture {
            root,
            paths,
            config,
            database,
            backup,
        }
    }

    async fn tracked_state(fixture: &RestoreFixture) -> Vec<(String, Option<Vec<u8>>)> {
        let mut state = Vec::new();
        for name in TRACKED {
            state.push((
                name.to_owned(),
                tokio::fs::read(fixture.paths.root().join(name)).await.ok(),
            ));
        }
        state.push((
            "database".into(),
            Some(tokio::fs::read(&fixture.database).await.unwrap()),
        ));
        state
    }

    fn leftovers(directory: &Path) -> Vec<String> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(directory).unwrap().filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains(".bobe-old-")
                || name.contains(".bobe-new-")
                || name.starts_with(".restore-")
                || name == RESTORE_JOURNAL
            {
                found.push(entry.path().display().to_string());
            }
            if entry.file_type().unwrap().is_dir() && !name.starts_with("backups") {
                found.extend(leftovers(&entry.path()));
            }
        }
        found
    }

    #[tokio::test]
    async fn a_restore_interrupted_at_any_step_is_reconciled_at_startup() {
        let reference = restore_fixture().await;
        let mut points = Vec::new();
        restore_interruptible(
            &reference.backup,
            &reference.paths,
            &reference.config,
            &SUPPORTED_SCHEMA_VERSIONS,
            |point| {
                points.push(point);
                false
            },
        )
        .await
        .unwrap();
        let restored = tracked_state(&reference).await;
        tokio::fs::remove_dir_all(&reference.root).await.unwrap();
        let committed = points
            .iter()
            .position(|point| *point == RestorePoint::Committed)
            .unwrap();
        assert!(matches!(
            points[committed - 1],
            RestorePoint::Installed | RestorePoint::MovedAside
        ));
        assert!(points.len() > committed + 1);

        for (interrupted_at, point) in points.iter().enumerate() {
            let fixture = restore_fixture().await;
            let live = tracked_state(&fixture).await;
            let mut reached = 0;
            let error = restore_interruptible(
                &fixture.backup,
                &fixture.paths,
                &fixture.config,
                &SUPPORTED_SCHEMA_VERSIONS,
                |_| {
                    reached += 1;
                    reached == interrupted_at + 1
                },
            )
            .await
            .expect_err("the restore must stop at the interruption");
            assert!(error.to_string().contains("interrupted"), "{error}");
            assert!(
                fixture.config.with_file_name(RESTORE_JOURNAL).exists(),
                "step {interrupted_at}: {point:?}"
            );

            reconcile_interrupted_restore(fixture.config.parent().unwrap())
                .await
                .unwrap();

            let state = tracked_state(&fixture).await;
            if interrupted_at >= committed {
                let mut expected = restored.clone();
                let database = expected.pop().unwrap();
                let mut actual = state.clone();
                actual.pop();
                assert_eq!(actual, expected, "step {interrupted_at}: {point:?}");
                assert!(database.1.is_some());
                assert_eq!(read_authority(&fixture.database).await.unwrap().1, 8);
            } else {
                assert_eq!(state, live, "step {interrupted_at}: {point:?}");
                assert_eq!(read_authority(&fixture.database).await.unwrap().1, 7);
            }
            assert!(
                leftovers(fixture.paths.root()).is_empty(),
                "step {interrupted_at}: {:?}",
                leftovers(fixture.paths.root())
            );
            reconcile_interrupted_restore(fixture.config.parent().unwrap())
                .await
                .unwrap();
            reconcile_restore_journals(&fixture.paths, &fixture.config)
                .await
                .unwrap();
            let (pool, _) = crate::bootstrap::initialize_storage(&fixture.paths)
                .await
                .unwrap();
            pool.close().await;
            tokio::fs::remove_dir_all(&fixture.root).await.unwrap();
        }
    }

    #[tokio::test]
    async fn an_interrupted_restore_waits_while_another_process_holds_the_authority() {
        let fixture = restore_fixture().await;
        let mut reached = 0;
        restore_interruptible(
            &fixture.backup,
            &fixture.paths,
            &fixture.config,
            &SUPPORTED_SCHEMA_VERSIONS,
            |_| {
                reached += 1;
                reached == 3
            },
        )
        .await
        .expect_err("interrupted");
        let journal = fixture.config.with_file_name(RESTORE_JOURNAL);
        let before = tokio::fs::read(&journal).await.unwrap();

        let holder = crate::util::runtime_lock::RuntimeLocks::acquire(&fixture.paths).unwrap();
        let error = reconcile_interrupted_restore(fixture.config.parent().unwrap())
            .await
            .expect_err("a held authority must block reconciliation");
        assert!(matches!(error, AppError::Conflict(_)), "{error}");
        assert_eq!(tokio::fs::read(&journal).await.unwrap(), before);
        assert!(
            crate::bootstrap::initialize_storage(&fixture.paths)
                .await
                .is_err()
        );
        drop(holder);

        reconcile_interrupted_restore(fixture.config.parent().unwrap())
            .await
            .unwrap();
        assert!(!journal.exists());
        assert_eq!(read_authority(&fixture.database).await.unwrap().1, 7);
        tokio::fs::remove_dir_all(&fixture.root).await.unwrap();
    }

    #[tokio::test]
    async fn an_offline_copy_keeps_committed_rows_that_are_still_in_the_write_ahead_log() {
        let root = std::env::temp_dir().join(format!("bobe-offline-copy-{}", uuid::Uuid::new_v4()));
        let paths = DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data/bobrust.db").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let (pool, identity) = crate::bootstrap::initialize_storage(&paths).await.unwrap();
        sqlx::query("PRAGMA wal_autocheckpoint = 0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO runtime_markers (key) VALUES ('written-after-update')")
            .execute(&pool)
            .await
            .unwrap();
        let database = paths.database_file().unwrap();
        let wal = database_sidecar(&database, "-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 0);

        let backup = create_offline(&paths, &root.join("config.toml"))
            .await
            .unwrap();
        pool.close().await;

        let manifest = verify(&backup, &SUPPORTED_SCHEMA_VERSIONS).await.unwrap();
        assert_eq!(manifest.runtime_authority_id, identity.runtime_authority_id);
        let mut connection = read_only_database(&backup.join(DATABASE_ARTIFACT))
            .await
            .unwrap();
        let kept: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runtime_markers WHERE key = 'written-after-update'",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        connection.close().await.unwrap();
        assert_eq!(kept, 1);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn a_live_backup_never_separates_a_credential_reference_from_its_secret() {
        let root = std::env::temp_dir().join(format!("bobe-live-backup-{}", uuid::Uuid::new_v4()));
        let paths = DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data/bobrust.db").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let (pool, identity) = crate::bootstrap::initialize_storage(&paths).await.unwrap();
        let config = root.join("config.toml");
        let secrets = paths.headless_secrets_file();
        let referenced =
            b"[providers.profiles.openai]\ncredential = 'bobe-secret://provider-openai'\n";
        let cleared = b"[providers.profiles.openai]\n";
        std::fs::write(&config, referenced).unwrap();
        std::fs::write(&secrets, br#"{"provider-openai":"sk-test"}"#).unwrap();
        let locks = SettingsLocks {
            config: std::sync::Arc::new(std::sync::Mutex::new(())),
            mcp: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        };

        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel::<()>();
        let clear_credential = std::thread::spawn({
            let lock = std::sync::Arc::clone(&locks.config);
            let (config, secrets) = (config.clone(), secrets.clone());
            move || {
                let _guard = lock.lock().unwrap();
                std::fs::write(&secrets, b"{}").unwrap();
                held_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                std::fs::write(&config, cleared).unwrap();
            }
        });
        tokio::task::spawn_blocking(move || held_rx.recv().unwrap())
            .await
            .unwrap();
        let backup = tokio::spawn({
            let (pool, paths, config) = (pool.clone(), paths.clone(), config.clone());
            let locks = SettingsLocks {
                config: std::sync::Arc::clone(&locks.config),
                mcp: std::sync::Arc::clone(&locks.mcp),
            };
            async move {
                create_live(
                    &pool,
                    &paths,
                    &config,
                    &locks,
                    identity.runtime_authority_id,
                    identity.runtime_authority_epoch,
                    identity.schema_version,
                )
                .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !backup.is_finished(),
            "the backup must wait for the in-flight credential change"
        );
        finish_tx.send(()).unwrap();
        clear_credential.join().unwrap();
        let backup = backup.await.unwrap().unwrap();

        verify(&backup, &SUPPORTED_SCHEMA_VERSIONS).await.unwrap();
        assert_eq!(
            tokio::fs::read(backup.join(CONFIG_ARTIFACT)).await.unwrap(),
            cleared
        );
        assert_eq!(
            tokio::fs::read(backup.join("data/headless-secrets.json"))
                .await
                .unwrap(),
            b"{}"
        );
        pool.close().await;
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
