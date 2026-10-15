use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::{Connection, Sqlite, SqliteConnection};
use tracing::{info, warn};

use crate::error::AppError;
use crate::provider::skill_store::{LEARNED_DIRECTORY, validate_skill_directory};
use crate::util::paths::DataPaths;
use crate::util::text::hex_encode;

const SCHEMA: &str = include_str!("../../migrations/schema.sql");
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 10;
pub(crate) const OLDEST_UPGRADABLE_SCHEMA_VERSION: i64 = 9;
pub(crate) const SUPPORTED_SCHEMA_VERSIONS: RangeInclusive<i64> =
    OLDEST_UPGRADABLE_SCHEMA_VERSION..=CURRENT_SCHEMA_VERSION;
const UPGRADES: [&str; 1] = [include_str!("../../migrations/upgrade-9-to-10.sql")];
const _: () =
    assert!(OLDEST_UPGRADABLE_SCHEMA_VERSION + UPGRADES.len() as i64 == CURRENT_SCHEMA_VERSION);
const RELEASE_0_8_1_CHAT_SKILL_SHA256: &str =
    "ceabdaaef2cf4de3dee425eb4afbd9c0183e005bff4dade74d02122700fa5f9a";
const INSTALLED_CHAT_SKILL: &str = "chat";
const RETIRED_CHAT_SKILL: &str = "chat.retired";

pub(crate) async fn read_schema_version(database: &Path) -> Result<i64, AppError> {
    let options = SqliteConnectOptions::new()
        .filename(database)
        .read_only(true)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let version = recorded_version(&mut connection).await?;
    connection.close().await?;
    version.ok_or_else(|| AppError::Conflict("database has no schema version".into()))
}

pub(crate) async fn open(paths: &DataPaths) -> Result<(SqlitePool, i64), AppError> {
    let pool = connect(paths.database_url()).await?;
    let mut connection = pool.acquire().await?;
    match stored_version(&mut connection).await? {
        None | Some(CURRENT_SCHEMA_VERSION) => apply_schema(&mut connection).await?,
        Some(version) if SUPPORTED_SCHEMA_VERSIONS.contains(&version) => {
            upgrade(connection, paths, version).await?;
        }
        Some(version) => return Err(unsupported_schema(Some(version))),
    }
    Ok((pool, CURRENT_SCHEMA_VERSION))
}

async fn connect(db_url: &str) -> Result<SqlitePool, AppError> {
    let options: SqliteConnectOptions = db_url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true)
        .pragma("journal_mode", "WAL")
        .pragma("foreign_keys", "ON")
        .pragma("busy_timeout", "5000");
    Ok(SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?)
}

async fn stored_version(connection: &mut SqliteConnection) -> Result<Option<i64>, AppError> {
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_one(&mut *connection)
    .await?;
    if tables == 0 {
        return Ok(None);
    }
    recorded_version(connection)
        .await?
        .map(Some)
        .ok_or_else(|| unsupported_schema(None))
}

async fn recorded_version(connection: &mut SqliteConnection) -> Result<Option<i64>, AppError> {
    let has_schema_table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'bobe_schema'",
    )
    .fetch_one(&mut *connection)
    .await?;
    if has_schema_table == 0 {
        return Ok(None);
    }
    Ok(
        sqlx::query_scalar("SELECT version FROM bobe_schema WHERE singleton = 1")
            .fetch_optional(&mut *connection)
            .await?,
    )
}

fn unsupported_schema(version: Option<i64>) -> AppError {
    AppError::Conflict(match version {
        Some(version) if version > CURRENT_SCHEMA_VERSION => format!(
            "database schema version {version} is newer than version {CURRENT_SCHEMA_VERSION}, which this daemon uses; run the BoBe release that wrote it"
        ),
        _ => format!(
            "database schema version {} cannot be upgraded to version {CURRENT_SCHEMA_VERSION}, which this daemon uses; start from a new data root",
            version.map_or_else(|| "unknown".to_owned(), |version| version.to_string())
        ),
    })
}

async fn apply_schema(connection: &mut SqliteConnection) -> Result<(), AppError> {
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::raw_sql(SCHEMA).execute(&mut *transaction).await?;
    stamp(&mut transaction, CURRENT_SCHEMA_VERSION).await?;
    transaction.commit().await?;
    info!(version = CURRENT_SCHEMA_VERSION, "database.schema_applied");
    Ok(())
}

async fn upgrade(
    mut connection: PoolConnection<Sqlite>,
    paths: &DataPaths,
    from: i64,
) -> Result<(), AppError> {
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *connection)
        .await?;
    let upgraded = upgrade_steps(&mut connection, paths, from).await;
    let enforced = sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *connection)
        .await;
    if upgraded.is_err() || enforced.is_err() {
        connection.close_on_drop();
    }
    upgraded?;
    enforced?;
    Ok(())
}

async fn upgrade_steps(
    connection: &mut SqliteConnection,
    paths: &DataPaths,
    from: i64,
) -> Result<(), AppError> {
    for version in from..CURRENT_SCHEMA_VERSION {
        let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;
        if let Err(error) = upgrade_step(&mut transaction, paths, version).await {
            transaction.rollback().await?;
            return Err(error);
        }
        transaction.commit().await?;
        info!(from = version, to = version + 1, "database.schema_upgraded");
    }
    Ok(())
}

async fn upgrade_step(
    connection: &mut SqliteConnection,
    paths: &DataPaths,
    from: i64,
) -> Result<(), AppError> {
    let upgrade = usize::try_from(from - OLDEST_UPGRADABLE_SCHEMA_VERSION)
        .ok()
        .and_then(|index| UPGRADES.get(index))
        .ok_or_else(|| unsupported_schema(Some(from)))?;
    sqlx::raw_sql(*upgrade).execute(&mut *connection).await?;
    let broken_references: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut *connection)
            .await?;
    if broken_references != 0 {
        return Err(AppError::Conflict(format!(
            "upgrading database schema version {from} would leave {broken_references} broken references; the database was not changed"
        )));
    }
    stamp(connection, from + 1).await?;
    upgrade_data_root(paths, from + 1).await
}

async fn stamp(connection: &mut SqliteConnection, version: i64) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO bobe_schema (singleton, version) VALUES (1, ?1)
         ON CONFLICT(singleton) DO UPDATE SET version = excluded.version",
    )
    .bind(version)
    .execute(&mut *connection)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA user_version = {version}"
    )))
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn upgrade_data_root(paths: &DataPaths, version: i64) -> Result<(), AppError> {
    if version == 10 {
        let skills = paths.skills_dir();
        retire_installed_chat_skill(&skills, RELEASE_0_8_1_CHAT_SKILL_SHA256).await?;
        move_placed_skills_to_learned(&skills).await?;
    }
    Ok(())
}

async fn retire_installed_chat_skill(skills: &Path, shipped_sha256: &str) -> Result<(), AppError> {
    let installed = skills.join(INSTALLED_CHAT_SKILL);
    let metadata = match tokio::fs::symlink_metadata(&installed).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && holds_only_skill(&installed, shipped_sha256).await? {
        crate::util::durable_fs::durable_remove_dir_all(&installed).await?;
        info!(path = %installed.display(), "data_root.chat_skill_removed");
        return Ok(());
    }
    let retired = unused_retired_path(skills).await;
    tokio::fs::rename(&installed, &retired).await?;
    crate::util::durable_fs::sync_directory(skills).await?;
    info!(path = %retired.display(), "data_root.chat_skill_retired");
    Ok(())
}

async fn holds_only_skill(directory: &Path, sha256: &str) -> Result<bool, AppError> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        names.push(entry.file_name());
    }
    let skill = directory.join("SKILL.md");
    if names != ["SKILL.md"] || !tokio::fs::symlink_metadata(&skill).await?.is_file() {
        return Ok(false);
    }
    Ok(hex_encode(&Sha256::digest(tokio::fs::read(&skill).await?)) == sha256)
}

async fn unused_retired_path(skills: &Path) -> PathBuf {
    let mut candidate = skills.join(RETIRED_CHAT_SKILL);
    let mut attempt = 1;
    while tokio::fs::symlink_metadata(&candidate).await.is_ok() {
        attempt += 1;
        candidate = skills.join(format!("{RETIRED_CHAT_SKILL}-{attempt}"));
    }
    candidate
}

async fn move_placed_skills_to_learned(skills: &Path) -> Result<(), AppError> {
    let mut entries = match tokio::fs::read_dir(skills).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut placed = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let reserved = name.starts_with('.')
            || name == LEARNED_DIRECTORY
            || name == INSTALLED_CHAT_SKILL
            || name.starts_with(RETIRED_CHAT_SKILL);
        if !reserved
            && tokio::fs::symlink_metadata(entry.path().join("SKILL.md"))
                .await
                .is_ok()
        {
            placed.push(entry.path());
        }
    }
    let learned = skills.join(LEARNED_DIRECTORY);
    let mut moved = false;
    for source in placed {
        match move_to_learned(&source, &learned).await {
            Ok(target) => {
                moved = true;
                info!(
                    from = %source.display(),
                    to = %target.display(),
                    "data_root.skill_moved_to_learned"
                );
            }
            Err(error) => {
                warn!(path = %source.display(), %error, "data_root.skill_left_in_place");
            }
        }
    }
    if moved {
        crate::util::durable_fs::sync_directory(skills).await?;
        crate::util::durable_fs::sync_directory(&learned).await?;
    }
    Ok(())
}

async fn move_to_learned(source: &Path, learned: &Path) -> Result<PathBuf, AppError> {
    if !tokio::fs::symlink_metadata(source).await?.is_dir() {
        return Err(AppError::Validation(
            "a skill must be a directory, not a link or file".into(),
        ));
    }
    validate_skill_directory(source).await?;
    let target = learned.join(source.file_name().unwrap_or_default());
    if tokio::fs::symlink_metadata(&target).await.is_ok() {
        return Err(AppError::Conflict(format!(
            "{} already exists",
            target.display()
        )));
    }
    tokio::fs::create_dir_all(learned).await?;
    tokio::fs::rename(source, &target).await?;
    Ok(target)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on precondition failures")]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const SCHEMA_9: &str = include_str!("../../migrations/fixtures/schema-9.sql");
    const SCHEMA_9_DEFAULT_USER_PROFILE: &str =
        include_str!("../../migrations/fixtures/schema-9-default-user-profile.txt");
    const SCHEMA_9_ROWS: &str = r#"
        INSERT INTO runtime_authority
            (singleton, authority_id, authority_epoch, data_root,
             last_incarnation_id, last_started_at, last_daemon_version)
        VALUES (1, X'0f1e2d3c4b5a69788796a5b4c3d2e1f0', 3, '/home/owner/.bobe',
                X'00112233445566778899aabbccddeeff', '2026-10-01T08:00:00.000Z', '0.8.1');
        INSERT INTO runtime_initialization (singleton, state) VALUES (1, 'ready');
        INSERT INTO runtime_markers (key) VALUES ('sample_goal_seeded_v1');
        INSERT INTO bobe_schema (singleton, version, daemon_version) VALUES (1, 9, '0.8.1');
        INSERT INTO conversations (id, channel_id, state, closed_at, summary) VALUES
            (X'c0', X'00000000000000000000000000000000', 'closed',
             '2026-10-01T09:00:00.000Z', 'An unread summary'),
            (X'c1', X'00000000000000000000000000000001', 'active', NULL, NULL);
        INSERT INTO conversation_turns (id, role, content, conversation_id) VALUES
            (X'70', 'user', 'Hello BoBe', X'c0'),
            (X'71', 'assistant', 'Hello', X'c0');
        INSERT INTO conversation_presentations (turn_id, channel_id, presented_at)
        VALUES (X'71', X'00000000000000000000000000000000', '2026-10-01T09:00:01.000Z');
        INSERT INTO message_requests
            (request_id, delivery_kind, channel_id, content_sha256, content, status, failure_reason)
        VALUES
            (X'70', 'surface', X'00000000000000000000000000000000', X'aa', 'Hello BoBe',
             'completed', NULL),
            (X'72', 'device', X'00000000000000000000000000000001', X'bb', 'Are you there?',
             'failed', 'device_disconnected_before_execution');
        INSERT INTO message_request_tombstones (request_id) VALUES (X'60');
        INSERT INTO agent_jobs
            (id, goal_id, source_key, title, task, context, profile, status, output,
             delivery_id, delivery_status)
        VALUES (X'a0', X'90', 'goal:review:1', 'Review goal', 'Review the goal', 'Goal context',
                'goals', 'completed', 'Done', X'd0', 'committed');
        INSERT INTO agent_schedules
            (id, source_key, title, task, context, channel_id, next_run_at, every_seconds,
             definition_json)
        VALUES (X'b0', 'weekly-review', 'Weekly review', 'Review the week', 'Schedule context',
                X'00000000000000000000000000000000', '2026-10-20T08:00:00.000Z', 604800, '{}');
        INSERT INTO agent_work_tombstones (source_key) VALUES ('goal:review:0');
        INSERT INTO runtime_cadences (key, next_due_at, interval_seconds)
        VALUES ('goal_review', '2026-10-15T12:00:00.000Z', 3600);
        INSERT INTO owner_inputs
            (id, source_key, origin_run_id, channel_id, kind, question, action, prompt,
             choices_json, definition_json, expires_at)
        VALUES (X'e0', 'input:1', X'e1', X'00000000000000000000000000000000', 'approval',
                'Send the email?', 'send_email', 'Continue', '["yes","no"]', '{}',
                '2026-10-16T00:00:00.000Z');
        INSERT INTO owner_input_presentations (input_id, channel_id, turn_id, presented_at)
        VALUES (X'e0', X'00000000000000000000000000000000', X'71', '2026-10-15T00:00:00.000Z');
        INSERT INTO souls (id, name, content, is_default)
        VALUES (X'50', 'default_soul', 'You are BoBe.', 1);
        INSERT INTO user_profiles (id, name, content, enabled, is_default) VALUES
            (X'd2', 'edited', 'I work nights as a nurse.', 0, 1),
            (X'd3', 'custom', 'I like short answers.', 1, 0);
        INSERT INTO endpoints
            (id, kind, label, room, certificate_sha256, capabilities_json, revoked_at,
             revocation_reason)
        VALUES
            ('mac-a', 'mac_app', 'Office Mac', NULL, NULL, '{"scope_profile":"mac_app.v1"}',
             NULL, NULL),
            ('body-a', 'physical_body', 'Kitchen BoBe', 'Kitchen',
             X'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', '{}',
             '2026-10-02T00:00:00.000Z', 'owner_requested');
        INSERT INTO endpoint_credentials
            (id, endpoint_id, kind, lookup_id, verifier, issued_authority_epoch, generation,
             revoked_at, revocation_reason, replaced_by)
        VALUES
            ('cred-2', 'mac-a', 'bearer', 'fedcba9876543210fedcba9876543210', X'02', 3, 2,
             NULL, NULL, NULL),
            ('cred-1', 'mac-a', 'bearer', '0123456789abcdef0123456789abcdef', X'01', 3, 1,
             '2026-10-03T00:00:00.000Z', 'superseded', 'cred-2');
        INSERT INTO endpoint_credentials
            (id, endpoint_id, kind, certificate_sha256, generation, revoked_at, revocation_reason)
        VALUES ('body-cert-body-a', 'body-a', 'body_certificate',
                X'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1,
                '2026-10-02T00:00:00.000Z', 'owner_requested');
        INSERT INTO endpoint_credentials (id, endpoint_id, kind, public_key, not_before, expires_at)
        VALUES ('key-1', 'mac-a', 'public_key', X'03', '2026-01-01T00:00:00.000Z',
                '2027-01-01T00:00:00.000Z');
        INSERT INTO endpoint_scope_grants (endpoint_id, scope, granted_by, revoked_at) VALUES
            ('mac-a', 'conversation.read', 'pairing', NULL),
            ('body-a', 'conversation.write', 'owner', '2026-10-02T00:00:00.000Z');
        INSERT INTO security_audit_events
            (id, event_kind, endpoint_id, credential_id, source_summary, details_json)
        VALUES
            (X'f0', 'endpoint_bearer_enrolled', 'mac-a', 'cred-2', '127.0.0.1',
             '{"generation":2}'),
            (X'f1', 'endpoint_revoked', 'body-a', NULL, NULL, '{"reason":"owner_requested"}');
        INSERT INTO cooldown_state (id, last_engagement, last_user_response)
        VALUES (X'0c', '2026-10-15T07:00:00.000Z', '2026-10-15T07:01:00.000Z');
    "#;

    struct Fixture {
        root: PathBuf,
        paths: DataPaths,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("bobe-schema-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("data")).expect("data directory");
            let paths = DataPaths::resolve(
                &root,
                &format!("sqlite:{}", root.join("data/bobrust.db").display()),
            )
            .expect("paths");
            Self { root, paths }
        }

        async fn schema_9(statements: &'static str, foreign_keys: bool) -> Self {
            let fixture = Self::new();
            let mut connection = fixture.connection(foreign_keys).await;
            sqlx::raw_sql(SCHEMA_9)
                .execute(&mut connection)
                .await
                .expect("schema 9");
            sqlx::raw_sql(statements)
                .execute(&mut connection)
                .await
                .expect("schema 9 rows");
            connection.close().await.expect("close");
            fixture
        }

        async fn connection(&self, foreign_keys: bool) -> SqliteConnection {
            SqliteConnection::connect_with(
                &SqliteConnectOptions::new()
                    .filename(self.paths.database_file().expect("database file"))
                    .create_if_missing(true)
                    .foreign_keys(foreign_keys),
            )
            .await
            .expect("connect")
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
            std::fs::write(path, content).expect("write");
        }

        fn remove(self) {
            std::fs::remove_dir_all(self.root).expect("remove fixture");
        }
    }

    async fn normalized_schema(connection: &mut SqliteConnection) -> Vec<(String, String, String)> {
        let objects: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .fetch_all(&mut *connection)
        .await
        .expect("schema objects");
        objects
            .into_iter()
            .map(|(kind, name, sql)| {
                let sql = sql
                    .unwrap_or_default()
                    .chars()
                    .filter(|character| !character.is_whitespace() && *character != '"')
                    .collect();
                (kind, name, sql)
            })
            .collect()
    }

    async fn table_rows(
        connection: &mut SqliteConnection,
        layout: &[(String, Vec<String>)],
    ) -> BTreeMap<String, Vec<String>> {
        let mut rows = BTreeMap::new();
        for (table, columns) in layout {
            let values = columns
                .iter()
                .map(|column| format!("quote(\"{column}\")"))
                .collect::<Vec<_>>()
                .join(" || '|' || ");
            let table_rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT {values} FROM \"{table}\" ORDER BY rowid"
            )))
            .fetch_all(&mut *connection)
            .await
            .expect("table rows");
            rows.insert(table.clone(), table_rows);
        }
        rows
    }

    async fn current_layout() -> Vec<(String, Vec<String>)> {
        let mut connection = SqliteConnection::connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::raw_sql(SCHEMA)
            .execute(&mut connection)
            .await
            .expect("schema");
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&mut connection)
                .await
                .expect("tables");
        let mut layout = Vec::new();
        for table in tables {
            let columns: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
                    .bind(&table)
                    .fetch_all(&mut connection)
                    .await
                    .expect("columns");
            layout.push((table, columns));
        }
        layout
    }

    async fn column_exists(connection: &mut SqliteConnection, table: &str, column: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&mut *connection)
        .await
        .expect("column")
    }

    async fn object_exists(connection: &mut SqliteConnection, name: &str) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)")
            .bind(name)
            .fetch_one(&mut *connection)
            .await
            .expect("object")
    }

    fn placed_skill(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: Placed by the owner\n---\n\n# {name}\n\nFollow the steps.\n"
        )
    }

    #[tokio::test]
    async fn fresh_database_does_not_preclaim_sample_goal_seed() {
        let fixture = Fixture::new();
        let (pool, _) = open(&fixture.paths).await.unwrap();
        let markers: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runtime_markers WHERE key = 'sample_goal_seeded_v1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        pool.close().await;
        assert_eq!(markers, 0);
        fixture.remove();
    }

    #[tokio::test]
    async fn schema_version_can_be_read_without_opening_a_write_connection() {
        let fixture = Fixture::new();
        let (pool, version) = open(&fixture.paths).await.unwrap();
        pool.close().await;

        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            read_schema_version(&fixture.paths.database_file().unwrap())
                .await
                .unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        let (pool, _) = open(&fixture.paths).await.unwrap();
        pool.close().await;
        fixture.remove();
    }

    #[tokio::test]
    async fn a_new_database_stamps_the_table_and_pragma_versions() {
        let fixture = Fixture::new();
        let (pool, _) = open(&fixture.paths).await.unwrap();
        let table_version: i64 =
            sqlx::query_scalar("SELECT version FROM bobe_schema WHERE singleton = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let pragma_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        assert_eq!(table_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(pragma_version, CURRENT_SCHEMA_VERSION);
        fixture.remove();
    }

    #[tokio::test]
    async fn a_schema_9_database_upgrades_to_the_current_schema_and_keeps_its_data() {
        let fixture = Fixture::schema_9(SCHEMA_9_ROWS, true).await;
        let mut connection = fixture.connection(true).await;
        sqlx::query(
            "INSERT INTO user_profiles (id, name, content, is_default) VALUES (X'd1', 'default', ?1, 1)",
        )
        .bind(SCHEMA_9_DEFAULT_USER_PROFILE)
        .execute(&mut connection)
        .await
        .unwrap();
        let layout = current_layout().await;
        let before = table_rows(&mut connection, &layout).await;
        let kept_credentials: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM endpoint_credentials WHERE kind <> 'public_key' ORDER BY rowid",
        )
        .fetch_all(&mut connection)
        .await
        .unwrap();
        connection.close().await.unwrap();
        fixture.write("skills/chat/SKILL.md", "my own companion rules\n");
        fixture.write(
            "skills/release-check/SKILL.md",
            &placed_skill("release-check"),
        );

        let (pool, version) = open(&fixture.paths).await.unwrap();
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;

        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!(foreign_keys, 1);
        let mut connection = fixture.connection(true).await;
        let after = table_rows(&mut connection, &layout).await;
        for (table, rows) in &before {
            if !matches!(
                table.as_str(),
                "bobe_schema" | "user_profiles" | "endpoint_credentials"
            ) {
                assert_eq!(&after[table], rows, "{table}");
            }
        }
        assert_eq!(after["bobe_schema"], vec!["1|10".to_owned()]);
        let credentials: Vec<String> =
            sqlx::query_scalar("SELECT id FROM endpoint_credentials ORDER BY rowid")
                .fetch_all(&mut connection)
                .await
                .unwrap();
        assert_eq!(credentials, kept_credentials);
        assert_eq!(
            after["endpoint_credentials"],
            before["endpoint_credentials"]
                .iter()
                .filter(|row| !row.starts_with("'key-1'"))
                .cloned()
                .collect::<Vec<_>>()
        );
        let profiles: Vec<(String, String, bool, bool)> = sqlx::query_as(
            "SELECT name, content, enabled, is_default FROM user_profiles ORDER BY name",
        )
        .fetch_all(&mut connection)
        .await
        .unwrap();
        assert_eq!(
            profiles,
            vec![
                ("custom".into(), "I like short answers.".into(), true, false),
                (
                    "edited".into(),
                    "I work nights as a nurse.".into(),
                    false,
                    false
                ),
            ]
        );
        for (table, column) in [
            ("conversations", "summary"),
            ("runtime_authority", "last_incarnation_id"),
            ("runtime_authority", "last_started_at"),
            ("runtime_authority", "last_daemon_version"),
            ("runtime_initialization", "updated_at"),
            ("bobe_schema", "applied_at"),
            ("bobe_schema", "daemon_version"),
            ("runtime_markers", "created_at"),
            ("agent_work_tombstones", "retired_at"),
            ("message_request_tombstones", "retired_at"),
            ("message_requests", "failure_reason"),
            ("endpoints", "revocation_reason"),
            ("endpoint_scope_grants", "granted_by"),
            ("endpoint_credentials", "public_key"),
            ("endpoint_credentials", "not_before"),
            ("endpoint_credentials", "expires_at"),
            ("endpoint_credentials", "revocation_reason"),
            ("endpoint_credentials", "replaced_by"),
        ] {
            assert!(
                !column_exists(&mut connection, table, column).await,
                "{table}.{column}"
            );
        }
        for name in [
            "security_audit_events",
            "ix_security_audit_events_created_at",
            "ix_security_audit_events_endpoint_created_at",
            "ix_conversations_state",
            "ix_conversation_turns_conversation_id",
            "ix_souls_name",
            "ix_user_profiles_name",
            "ix_endpoints_certificate_active",
        ] {
            assert!(!object_exists(&mut connection, name).await, "{name}");
        }
        let broken_references: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        let pragma_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(broken_references, 0);
        assert_eq!(integrity, "ok");
        assert_eq!(pragma_version, CURRENT_SCHEMA_VERSION);
        let public_key = sqlx::query(
            "INSERT INTO endpoint_credentials (id, endpoint_id, kind, certificate_sha256) VALUES ('key-2', 'mac-a', 'public_key', X'04')",
        )
        .execute(&mut connection)
        .await;
        assert!(public_key.is_err());
        let upgraded_schema = normalized_schema(&mut connection).await;
        connection.close().await.unwrap();

        let mut fresh = SqliteConnection::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(SCHEMA).execute(&mut fresh).await.unwrap();
        assert_eq!(upgraded_schema, normalized_schema(&mut fresh).await);
        assert!(!fixture.root.join("skills/chat").exists());
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("skills/chat.retired/SKILL.md")).unwrap(),
            "my own companion rules\n"
        );
        assert!(!fixture.root.join("skills/release-check").exists());
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("skills/learned/release-check/SKILL.md"))
                .unwrap(),
            placed_skill("release-check")
        );

        let (pool, version) = open(&fixture.paths).await.unwrap();
        let profiles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_profiles")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!(profiles, 2);
        fixture.remove();
    }

    #[tokio::test]
    async fn a_failed_upgrade_leaves_the_database_at_its_previous_version() {
        let broken_reference = Fixture::schema_9(
            "INSERT INTO bobe_schema (singleton, version, daemon_version) VALUES (1, 9, '0.8.1');
             INSERT INTO conversation_turns (id, role, content, conversation_id)
             VALUES (X'70', 'user', 'orphan', X'c9');",
            false,
        )
        .await;
        broken_reference.write("skills/chat/SKILL.md", "kept\n");
        let unwritable_data_root = Fixture::schema_9(
            "INSERT INTO bobe_schema (singleton, version, daemon_version) VALUES (1, 9, '0.8.1');",
            true,
        )
        .await;
        unwritable_data_root.write("skills", "not a directory");

        for (fixture, expected) in [
            (broken_reference, "broken references"),
            (unwritable_data_root, "IO error"),
        ] {
            let error = open(&fixture.paths).await.expect_err("upgrade must fail");
            assert!(error.to_string().contains(expected), "{error}");

            assert_eq!(
                read_schema_version(&fixture.paths.database_file().unwrap())
                    .await
                    .unwrap(),
                OLDEST_UPGRADABLE_SCHEMA_VERSION
            );
            let mut connection = fixture.connection(true).await;
            assert!(column_exists(&mut connection, "conversations", "summary").await);
            assert!(column_exists(&mut connection, "bobe_schema", "daemon_version").await);
            assert!(object_exists(&mut connection, "security_audit_events").await);
            connection.close().await.unwrap();
            if fixture.root.join("skills").is_dir() {
                assert_eq!(
                    std::fs::read_to_string(fixture.root.join("skills/chat/SKILL.md")).unwrap(),
                    "kept\n"
                );
            }
            fixture.remove();
        }
    }

    #[tokio::test]
    async fn a_database_outside_the_supported_versions_is_refused_untouched() {
        for setup in [
            format!(
                "{SCHEMA_9} INSERT INTO bobe_schema (singleton, version, daemon_version) VALUES (1, {}, 'older');",
                OLDEST_UPGRADABLE_SCHEMA_VERSION - 1
            ),
            format!(
                "{SCHEMA} INSERT INTO bobe_schema (singleton, version) VALUES (1, {});",
                CURRENT_SCHEMA_VERSION + 1
            ),
            "CREATE TABLE conversations (id BLOB PRIMARY KEY);".to_owned(),
        ] {
            let fixture = Fixture::new();
            let mut connection = fixture.connection(true).await;
            sqlx::raw_sql(sqlx::AssertSqlSafe(setup))
                .execute(&mut connection)
                .await
                .expect("existing database");
            let schema_before = normalized_schema(&mut connection).await;
            let version_before = recorded_version(&mut connection).await.unwrap();
            connection.close().await.unwrap();
            fixture.write("skills/chat/SKILL.md", "kept\n");

            let error = open(&fixture.paths).await.expect_err("must refuse");

            assert!(matches!(error, AppError::Conflict(_)));
            let mut connection = fixture.connection(true).await;
            assert_eq!(normalized_schema(&mut connection).await, schema_before);
            assert_eq!(
                recorded_version(&mut connection).await.unwrap(),
                version_before
            );
            connection.close().await.unwrap();
            assert_eq!(
                std::fs::read_to_string(fixture.root.join("skills/chat/SKILL.md")).unwrap(),
                "kept\n"
            );
            fixture.remove();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn placed_skills_move_to_learned_only_when_valid_and_free() {
        let fixture = Fixture::new();
        let skills = fixture.root.join("skills");
        for (relative, content) in [
            (
                "skills/release-check/SKILL.md",
                placed_skill("release-check"),
            ),
            ("skills/release-check/notes.md", "owner notes\n".to_owned()),
            (
                "skills/Release_Check/SKILL.md",
                placed_skill("release-check"),
            ),
            (
                "skills/no-frontmatter/SKILL.md",
                "Just follow the steps.\n".to_owned(),
            ),
            ("skills/taken/SKILL.md", placed_skill("taken")),
            ("skills/learned/taken/SKILL.md", placed_skill("original")),
            ("skills/chat.retired/SKILL.md", "retired rules\n".to_owned()),
            ("skills/.archive/SKILL.md", placed_skill("archive")),
            ("skills/notes/readme.md", "not a skill\n".to_owned()),
            ("outside/linked/SKILL.md", placed_skill("linked")),
        ] {
            fixture.write(relative, &content);
        }
        std::os::unix::fs::symlink(fixture.root.join("outside/linked"), skills.join("linked"))
            .unwrap();

        move_placed_skills_to_learned(&skills).await.unwrap();
        move_placed_skills_to_learned(&skills).await.unwrap();

        assert!(!skills.join("release-check").exists());
        assert_eq!(
            std::fs::read_to_string(skills.join("learned/release-check/SKILL.md")).unwrap(),
            placed_skill("release-check")
        );
        assert_eq!(
            std::fs::read_to_string(skills.join("learned/release-check/notes.md")).unwrap(),
            "owner notes\n"
        );
        for kept in [
            "Release_Check/SKILL.md",
            "no-frontmatter/SKILL.md",
            "taken/SKILL.md",
            "chat.retired/SKILL.md",
            ".archive/SKILL.md",
            "notes/readme.md",
        ] {
            assert!(skills.join(kept).is_file(), "{kept}");
        }
        assert!(
            std::fs::symlink_metadata(skills.join("linked"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(skills.join("learned/taken/SKILL.md")).unwrap(),
            placed_skill("original")
        );
        let learned = crate::provider::SkillStore::prepare(&fixture.paths)
            .await
            .unwrap()
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|skill| (skill.id, skill.name))
            .collect::<Vec<_>>();
        assert_eq!(
            learned,
            [
                ("release-check".to_owned(), "release-check".to_owned()),
                ("taken".to_owned(), "original".to_owned()),
            ]
        );
        fixture.remove();
    }

    #[tokio::test]
    async fn the_installed_chat_skill_is_removed_only_when_unchanged() {
        let shipped = "shipped companion rules\n";
        let shipped_sha256 = hex_encode(&Sha256::digest(shipped.as_bytes()));
        for (files, occupied, retired_as) in [
            (vec![("chat/SKILL.md", shipped)], vec![], None),
            (
                vec![("chat/SKILL.md", "edited companion rules\n")],
                vec![],
                Some("chat.retired"),
            ),
            (
                vec![
                    ("chat/SKILL.md", shipped),
                    ("chat/notes.md", "owner notes\n"),
                ],
                vec![],
                Some("chat.retired"),
            ),
            (
                vec![("chat/SKILL.md", "edited companion rules\n")],
                vec!["chat.retired", "chat.retired-2"],
                Some("chat.retired-3"),
            ),
            (vec![("learned/notes/SKILL.md", "learned\n")], vec![], None),
        ] {
            let fixture = Fixture::new();
            let skills = fixture.root.join("skills");
            for (relative, content) in &files {
                fixture.write(&format!("skills/{relative}"), content);
            }
            for name in &occupied {
                std::fs::create_dir_all(skills.join(name)).unwrap();
            }

            retire_installed_chat_skill(&skills, &shipped_sha256)
                .await
                .unwrap();

            assert!(!skills.join("chat").exists());
            if let Some(retired_as) = retired_as {
                for (relative, content) in &files {
                    let retired = skills.join(relative.replacen("chat", retired_as, 1));
                    assert_eq!(std::fs::read_to_string(retired).unwrap(), *content);
                }
            } else {
                assert!(!skills.join("chat.retired").exists());
            }
            for name in &occupied {
                assert!(skills.join(name).is_dir());
            }
            if files[0].0.starts_with("learned/") {
                assert!(skills.join("learned/notes/SKILL.md").is_file());
            }
            fixture.remove();
        }
    }
}
