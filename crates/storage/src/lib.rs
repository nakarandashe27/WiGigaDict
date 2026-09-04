#![cfg_attr(test, allow(linker_messages))]

//! SQLite bootstrap and versioned migrations for WiGigaDict domain storage.
//!
//! This crate owns connection durability settings and schema evolution. UI and
//! ASR crates must consume repository contracts built on top of this boundary,
//! never open SQLite directly.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod asr_dispatcher;
mod catalog;
mod cleanup;
mod configuration;
mod delivery;
mod diagnostics;
mod model_manager;
mod recovery;
mod session_commit;

pub use asr_dispatcher::{
    ASR_LEASE_MS, AdmissionRejection, AsrCompletionMetrics, AsrDispatcher, AsrDispatcherError,
    AsrLease, AsrLeaseKey, CaptureDiskSpace, MAX_PENDING_PCM_BYTES, MAX_PENDING_SESSIONS,
    MAX_SESSION_PCM_BYTES, MIN_DISK_HEADROOM_BYTES, RawTranscriptReceipt, SystemCaptureDiskSpace,
};
pub use catalog::{
    CATALOG_SCHEMA_VERSION, CatalogEntry, CatalogRequirements, ModelCatalog, verify_catalog,
};
pub use cleanup::{
    CLEANUP_GLOSSARY_REVISION, CLEANUP_POLICY_HASH, CLEANUP_POLICY_MANIFEST,
    CLEANUP_POLICY_VERSION, CLEANUP_TIMEOUT_MS, CleanupCandidate, CleanupContract, CleanupEngine,
    CleanupEngineError, CleanupFallbackReason, CleanupRepository, CleanupRepositoryError,
    CleanupRuleMetrics, CleanupSelection, DeterministicCleanupEngine, TranscriptSnapshot,
};
pub use configuration::*;
pub use delivery::*;
pub use diagnostics::*;
pub use model_manager::{
    DiskSpace, DownloadObserver, FileCompatibilityProbe, IgnoreProgress, InstalledModel,
    ManifestFile, ModelInstallReceipt, ModelManager, ModelManagerError, ModelManifest,
    ModelPreview, RangeDownloader, ReqwestRangeDownloader, RuntimeManifest, RuntimeProbe,
    SignedManifest, SystemDiskSpace, TrustedKeyRing,
};
pub use recovery::*;

pub use session_commit::{
    CaptureCommitPlan, CommitCheckpoint, CommitReceipt, CommitResult, ManagedAudioStore, PcmFormat,
    PcmPartWriter, ReconciliationDisposition, ReconciliationRecord, RecoveryReceipt,
    SessionCommitCoordinator, SessionCommitError,
};

const MIGRATION_1_NAME: &str = "0001_initial";
const MIGRATION_1_SQL: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_2_NAME: &str = "0002_audio_commit_intent";
const MIGRATION_2_SQL: &str = include_str!("../migrations/0002_audio_commit_intent.sql");
const MIGRATION_3_NAME: &str = "0003_model_manager_verification";
const MIGRATION_3_SQL: &str = include_str!("../migrations/0003_model_manager_verification.sql");
const MIGRATION_4_NAME: &str = "0004_asr_dispatcher";
const MIGRATION_4_SQL: &str = include_str!("../migrations/0004_asr_dispatcher.sql");
const MIGRATION_5_NAME: &str = "0005_cleanup";
const MIGRATION_5_SQL: &str = include_str!("../migrations/0005_cleanup.sql");
const MIGRATION_6_NAME: &str = "0006_insertion_evidence";
const MIGRATION_6_SQL: &str = include_str!("../migrations/0006_insertion_evidence.sql");
const MIGRATION_7_NAME: &str = "0007_recovery_retention";
const MIGRATION_7_SQL: &str = include_str!("../migrations/0007_recovery_retention.sql");
const MIGRATION_8_NAME: &str = "0008_local_archive";
const MIGRATION_8_SQL: &str = include_str!("../migrations/0008_local_archive.sql");

#[derive(Clone, Copy)]
struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: [Migration; 8] = [
    Migration {
        version: 1,
        name: MIGRATION_1_NAME,
        sql: MIGRATION_1_SQL,
    },
    Migration {
        version: 2,
        name: MIGRATION_2_NAME,
        sql: MIGRATION_2_SQL,
    },
    Migration {
        version: 3,
        name: MIGRATION_3_NAME,
        sql: MIGRATION_3_SQL,
    },
    Migration {
        version: 4,
        name: MIGRATION_4_NAME,
        sql: MIGRATION_4_SQL,
    },
    Migration {
        version: 5,
        name: MIGRATION_5_NAME,
        sql: MIGRATION_5_SQL,
    },
    Migration {
        version: 6,
        name: MIGRATION_6_NAME,
        sql: MIGRATION_6_SQL,
    },
    Migration {
        version: 7,
        name: MIGRATION_7_NAME,
        sql: MIGRATION_7_SQL,
    },
    Migration {
        version: 8,
        name: MIGRATION_8_NAME,
        sql: MIGRATION_8_SQL,
    },
];

/// Latest schema version understood by this binary.
pub const LATEST_SCHEMA_VERSION: u32 = 8;

/// The 18 M1 domain tables. Migration metadata is intentionally not included.
pub const M1_DOMAIN_TABLES: [&str; 18] = [
    "dictation_session",
    "target_snapshot",
    "audio_artifact",
    "session_event",
    "asr_attempt",
    "transcript_version",
    "cleanup_attempt",
    "delivery_operation",
    "delivery_attempt",
    "model_package",
    "runtime_profile",
    "model_install_job",
    "app_configuration",
    "cleanup_profile",
    "app_profile",
    "glossary_entry",
    "diagnostic_event",
    "maintenance_run",
];

#[derive(Debug)]
pub enum StorageError {
    Sqlite(rusqlite::Error),
    UnsupportedSchema { found: u32, supported: u32 },
    SchemaMismatch(String),
    ClockBeforeUnixEpoch,
}

impl Display for StorageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "database schema version {found} is newer than supported version {supported}"
            ),
            Self::SchemaMismatch(detail) => write!(formatter, "database schema mismatch: {detail}"),
            Self::ClockBeforeUnixEpoch => {
                write!(formatter, "system clock is before the Unix epoch")
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Owned SQLite connection configured and migrated for domain repositories.
pub struct Database {
    connection: Connection,
}

impl Database {
    /// Opens a persistent database, enables durability settings, and migrates it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    /// Opens an in-memory database for repository/unit tests.
    ///
    /// SQLite reports `memory` instead of `wal` for this connection type; all
    /// persistent databases opened by [`Self::open`] are required to use WAL.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn schema_version(&self) -> Result<u32> {
        let version = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))?;
        Ok(version)
    }

    /// Rechecks migration identity, required tables, foreign keys and pragmas.
    pub fn verify(&self) -> Result<()> {
        verify_connection_configuration(&self.connection)?;
        verify_schema(&self.connection)
    }

    fn from_connection(mut connection: Connection) -> Result<Self> {
        configure_connection(&connection)?;
        migrate(&mut connection)?;
        verify_schema(&connection)?;
        Ok(Self { connection })
    }
}

fn configure_connection(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(Duration::from_secs(5))?;

    let main_path: String = connection.query_row(
        "SELECT file FROM pragma_database_list WHERE name = 'main'",
        [],
        |row| row.get(0),
    )?;
    let journal_mode: String =
        connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !main_path.is_empty() && !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::SchemaMismatch(format!(
            "persistent database refused WAL mode and reported {journal_mode}"
        )));
    }

    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "secure_delete", true)?;
    verify_connection_configuration(connection)
}

fn verify_connection_configuration(connection: &Connection) -> Result<()> {
    let foreign_keys: u8 = connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if foreign_keys != 1 {
        return Err(StorageError::SchemaMismatch(
            "PRAGMA foreign_keys must be ON".to_owned(),
        ));
    }

    let synchronous: u8 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    if synchronous != 2 {
        return Err(StorageError::SchemaMismatch(format!(
            "PRAGMA synchronous must be FULL (2), found {synchronous}"
        )));
    }

    let secure_delete: u8 =
        connection.pragma_query_value(None, "secure_delete", |row| row.get(0))?;
    if secure_delete != 1 {
        return Err(StorageError::SchemaMismatch(format!(
            "PRAGMA secure_delete must be ON (1), found {secure_delete}"
        )));
    }

    let main_path: String = connection.query_row(
        "SELECT file FROM pragma_database_list WHERE name = 'main'",
        [],
        |row| row.get(0),
    )?;
    let journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if !main_path.is_empty() && !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::SchemaMismatch(format!(
            "persistent database must use WAL, found {journal_mode}"
        )));
    }

    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let current =
        connection.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))?;
    if current > LATEST_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchema {
            found: current,
            supported: LATEST_SCHEMA_VERSION,
        });
    }

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version > current)
    {
        let applied_at = unix_time_ms()?;
        let checksum = migration_checksum(migration.sql);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, name, checksum_sha256, applied_at) VALUES (?1, ?2, ?3, ?4)",
            params![migration.version, migration.name, checksum, applied_at],
        )?;
        transaction.pragma_update(None, "user_version", migration.version)?;
        transaction.commit()?;
    }

    verify_migration_identity(connection)
}

fn verify_migration_identity(connection: &Connection) -> Result<()> {
    for migration in MIGRATIONS {
        let recorded: Option<(String, String)> = connection
            .query_row(
                "SELECT name, checksum_sha256 FROM schema_migrations WHERE version = ?1",
                [migration.version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let expected_checksum = migration_checksum(migration.sql);
        match recorded {
            Some((name, checksum)) if name == migration.name && checksum == expected_checksum => {}
            Some((name, checksum)) => {
                return Err(StorageError::SchemaMismatch(format!(
                    "migration {} identity differs: name={name}, checksum={checksum}",
                    migration.version
                )));
            }
            None => {
                return Err(StorageError::SchemaMismatch(format!(
                    "migration {} is absent from schema_migrations",
                    migration.version
                )));
            }
        }
    }
    Ok(())
}

fn verify_schema(connection: &Connection) -> Result<()> {
    verify_migration_identity(connection)?;

    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let missing = M1_DOMAIN_TABLES
        .iter()
        .filter(|table| !actual.contains(**table))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(StorageError::SchemaMismatch(format!(
            "missing M1 domain tables: {}",
            missing.join(", ")
        )));
    }

    let foreign_key_violations: u32 =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_violations != 0 {
        return Err(StorageError::SchemaMismatch(format!(
            "foreign_key_check reported {foreign_key_violations} violation(s)"
        )));
    }

    Ok(())
}

fn migration_checksum(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.as_bytes()))
}

fn unix_time_ms() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StorageError::ClockBeforeUnixEpoch)?;
    i64::try_from(duration.as_millis()).map_err(|_| {
        StorageError::SchemaMismatch("current Unix timestamp does not fit into i64".to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn temp_database_path(label: &str) -> std::path::PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "wigigadict-storage-{label}-{}-{unique}.sqlite3",
            std::process::id()
        ))
    }

    fn remove_database_files(path: &Path) {
        for candidate in [
            path.to_path_buf(),
            std::path::PathBuf::from(format!("{}-wal", path.display())),
            std::path::PathBuf::from(format!("{}-shm", path.display())),
        ] {
            if candidate.exists() {
                fs::remove_file(&candidate)
                    .expect("generated test database file must be removable");
            }
        }
    }

    fn sqlite_constraint(result: rusqlite::Result<usize>) {
        let error = result.expect_err("statement must be rejected");
        assert!(
            matches!(error, rusqlite::Error::SqliteFailure(_, _)),
            "unexpected error: {error}"
        );
    }

    fn insert_session(connection: &Connection, id: &str, state: &str) {
        connection
            .execute(
                "INSERT INTO dictation_session(
                    id, pipeline_state, state_version, started_at, created_at, updated_at
                 ) VALUES (?1, ?2, 1, 100, 100, 100)",
                params![id, state],
            )
            .expect("session fixture must insert");
    }

    fn insert_model_and_runtime(connection: &Connection) {
        connection
            .execute(
                "INSERT INTO model_package(
                    id, engine_family, model_name, model_version, source_uri, license_id,
                    expected_size, checksum_algorithm, checksum, storage_key, install_state,
                    installed_at, created_at, updated_at
                 ) VALUES (
                    'model-1', 'whisper', 'large-v3-turbo-q5', '1', 'managed:model-1', 'MIT',
                    100, 'sha256', ?1, 'models/model-1.bin', 'installed', 100, 100, 100
                 )",
                [HASH],
            )
            .expect("model fixture must insert");
        connection
            .execute(
                "INSERT INTO runtime_profile(
                    id, profile_version, model_package_id, adapter_type, adapter_version,
                    device_kind, settings, settings_hash, health_state, enabled, created_at, updated_at
                 ) VALUES (
                    'runtime-1', 1, 'model-1', 'transcribe-rs', '0.3.11',
                    'cpu', '{}', ?1, 'healthy', 1, 100, 100
                 )",
                [HASH],
            )
            .expect("runtime fixture must insert");
    }

    fn insert_finalized_audio(connection: &Connection, session_id: &str, id: &str) {
        connection
            .execute(
                "INSERT INTO audio_artifact(
                    id, session_id, commit_id, staging_storage_key, storage_key, format,
                    duration_ms, reserved_byte_size, byte_size, content_hash,
                    artifact_state, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'pcm_s16le_16khz_mono',
                    1000, 32000, 32000, ?6, 'finalized', 100)",
                params![
                    id,
                    session_id,
                    format!("commit-{id}"),
                    format!("staging/{id}.part"),
                    format!("audio/{id}.wav"),
                    HASH
                ],
            )
            .expect("audio fixture must insert");
    }

    fn insert_succeeded_asr(connection: &Connection, session_id: &str, audio_id: &str) {
        connection
            .execute(
                "INSERT INTO asr_attempt(
                    id, session_id, audio_artifact_id, runtime_profile_id, attempt_no,
                    idempotency_key, status, queued_at, started_at, completed_at
                 ) VALUES ('asr-1', ?1, ?2, 'runtime-1', 1, 'asr-key-1',
                    'succeeded', 100, 110, 120)",
                params![session_id, audio_id],
            )
            .expect("ASR fixture must insert");
    }

    fn insert_raw_transcript(connection: &Connection, session_id: &str) {
        connection
            .execute(
                "INSERT INTO transcript_version(
                    id, session_id, kind, version_no, content, content_hash,
                    source_asr_attempt_id, created_at
                 ) VALUES ('transcript-1', ?1, 'raw', 1, 'fixture text', ?2, 'asr-1', 120)",
                params![session_id, HASH],
            )
            .expect("transcript fixture must insert");
    }

    #[test]
    fn fresh_database_has_all_18_entities_and_required_pragmas() {
        let path = temp_database_path("fresh");
        let database = Database::open(&path).expect("fresh database must migrate");
        assert_eq!(database.schema_version().unwrap(), LATEST_SCHEMA_VERSION);
        database.verify().expect("fresh schema must verify");

        let actual = database
            .connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<BTreeSet<_>, _>>()
            .unwrap();
        assert_eq!(M1_DOMAIN_TABLES.len(), 18);
        assert!(M1_DOMAIN_TABLES.iter().all(|table| actual.contains(*table)));
        assert!(actual.contains("schema_migrations"));
        assert!(actual.contains("audio_commit_intent"));
        assert_eq!(actual.len(), M1_DOMAIN_TABLES.len() + 2);
        assert!(actual.iter().all(|table| !table.starts_with("notetaker_")));

        let journal: String = database
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "wal");
        drop(database);
        remove_database_files(&path);
    }

    #[test]
    fn nonempty_v0_fixture_upgrades_automatically_without_losing_sentinel() {
        let path = temp_database_path("upgrade-v0");
        let fixture = Connection::open(&path).unwrap();
        fixture
            .execute_batch(include_str!("../tests/fixtures/v0.sql"))
            .unwrap();
        drop(fixture);

        let database = Database::open(&path).expect("v0 fixture must auto-upgrade");
        assert_eq!(database.schema_version().unwrap(), LATEST_SCHEMA_VERSION);
        let value: String = database
            .connection
            .query_row(
                "SELECT value FROM upgrade_fixture_sentinel WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "preserve-me");
        drop(database);
        remove_database_files(&path);
    }

    #[test]
    fn schema_v1_upgrades_to_audio_commit_ledger_automatically() {
        let path = temp_database_path("upgrade-v1");
        let fixture = Connection::open(&path).unwrap();
        fixture.execute_batch(MIGRATION_1_SQL).unwrap();
        fixture
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum_sha256, applied_at)
                 VALUES (1, ?1, ?2, 1)",
                params![MIGRATION_1_NAME, migration_checksum(MIGRATION_1_SQL)],
            )
            .unwrap();
        fixture.pragma_update(None, "user_version", 1).unwrap();
        drop(fixture);

        let database = Database::open(&path).expect("v1 fixture must auto-upgrade");
        assert_eq!(database.schema_version().unwrap(), LATEST_SCHEMA_VERSION);
        let ledger_exists: bool = database
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='audio_commit_intent')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(ledger_exists);
        drop(database);
        remove_database_files(&path);
    }

    #[test]
    fn migration_checksum_mismatch_fails_verification() {
        let database = Database::open_in_memory().unwrap();
        database
            .connection
            .execute(
                "UPDATE schema_migrations SET checksum_sha256 = ?1 WHERE version = 1",
                ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
            )
            .unwrap();
        assert!(matches!(
            database.verify(),
            Err(StorageError::SchemaMismatch(_))
        ));
    }

    #[test]
    fn newer_schema_version_fails_closed() {
        let path = temp_database_path("future");
        let connection = Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);

        let error = Database::open(&path)
            .err()
            .expect("future schema must fail");
        assert!(matches!(
            error,
            StorageError::UnsupportedSchema {
                found: 99,
                supported: LATEST_SCHEMA_VERSION
            }
        ));
        remove_database_files(&path);
    }

    #[test]
    fn foreign_keys_and_same_session_relations_fail_closed() {
        let database = Database::open_in_memory().unwrap();
        let connection = &database.connection;
        sqlite_constraint(connection.execute(
            "INSERT INTO target_snapshot(
                id, session_id, purpose, process_identity, process_id, window_handle,
                window_class, integrity_level, captured_at
             ) VALUES ('target-missing', 'missing', 'initial', 'codex', 1, '0x1',
                'fixture', 'medium', 100)",
            [],
        ));

        insert_session(connection, "session-1", "processing");
        insert_session(connection, "session-2", "processing");
        insert_model_and_runtime(connection);
        insert_finalized_audio(connection, "session-1", "audio-1");
        sqlite_constraint(connection.execute(
            "INSERT INTO asr_attempt(
                id, session_id, audio_artifact_id, runtime_profile_id, attempt_no,
                idempotency_key, status, queued_at
             ) VALUES ('asr-cross-session', 'session-2', 'audio-1', 'runtime-1', 1,
                'asr-cross-key', 'queued', 100)",
            [],
        ));
    }

    #[test]
    fn transcript_source_xor_and_immutability_are_enforced() {
        let database = Database::open_in_memory().unwrap();
        let connection = &database.connection;
        insert_session(connection, "session-1", "processing");
        insert_model_and_runtime(connection);
        insert_finalized_audio(connection, "session-1", "audio-1");
        insert_succeeded_asr(connection, "session-1", "audio-1");

        sqlite_constraint(connection.execute(
            "INSERT INTO transcript_version(
                id, session_id, kind, version_no, content, content_hash, created_at
             ) VALUES ('bad-transcript', 'session-1', 'raw', 1, '', ?1, 120)",
            [HASH],
        ));

        insert_raw_transcript(connection, "session-1");
        sqlite_constraint(connection.execute(
            "UPDATE transcript_version SET content = 'mutated' WHERE id = 'transcript-1'",
            [],
        ));
        sqlite_constraint(connection.execute(
            "INSERT INTO transcript_version(
                id, session_id, kind, version_no, content, content_hash,
                source_asr_attempt_id, created_at
             ) VALUES ('transcript-2', 'session-1', 'raw', 2, 'duplicate source', ?1,
                'asr-1', 121)",
            [HASH],
        ));
    }

    #[test]
    fn session_event_sequence_is_unique_and_append_only() {
        let database = Database::open_in_memory().unwrap();
        let connection = &database.connection;
        insert_session(connection, "session-1", "processing");
        connection
            .execute(
                "INSERT INTO session_event(
                    id, session_id, sequence_no, event_type, source, occurred_at
                 ) VALUES ('event-1', 'session-1', 1, 'created', 'system', 100)",
                [],
            )
            .unwrap();
        sqlite_constraint(connection.execute(
            "INSERT INTO session_event(
                id, session_id, sequence_no, event_type, source, occurred_at
             ) VALUES ('event-2', 'session-1', 1, 'duplicate', 'system', 101)",
            [],
        ));
        sqlite_constraint(connection.execute(
            "UPDATE session_event SET event_type = 'mutated' WHERE id = 'event-1'",
            [],
        ));
    }

    #[test]
    fn glossary_scope_xor_is_enforced() {
        let database = Database::open_in_memory().unwrap();
        sqlite_constraint(database.connection.execute(
            "INSERT INTO glossary_entry(
                id, entry_key, entry_version, glossary_revision, spoken_form,
                preferred_form, scope_kind, mode, case_policy, enabled, created_at
             ) VALUES ('glossary-1', 'api', 1, 1, 'апи', 'API',
                'global', 'dictation', 'preferred', 1, 100)",
            [],
        ));
    }

    #[test]
    fn delivery_false_success_is_rejected() {
        let database = Database::open_in_memory().unwrap();
        let connection = &database.connection;
        insert_session(connection, "session-1", "processing");
        insert_model_and_runtime(connection);
        insert_finalized_audio(connection, "session-1", "audio-1");
        insert_succeeded_asr(connection, "session-1", "audio-1");
        insert_raw_transcript(connection, "session-1");
        connection
            .execute(
                "INSERT INTO target_snapshot(
                    id, session_id, purpose, process_identity, process_id, window_handle,
                    window_class, integrity_level, captured_at
                 ) VALUES ('target-1', 'session-1', 'initial', 'codex', 1, '0x1',
                    'fixture', 'medium', 100)",
                [],
            )
            .unwrap();

        sqlite_constraint(connection.execute(
            "INSERT INTO delivery_operation(
                id, session_id, transcript_version_id, target_snapshot_id, operation_no,
                initiated_by, status, confirmation_level, started_at, completed_at
             ) VALUES ('delivery-1', 'session-1', 'transcript-1', 'target-1', 1,
                'system', 'delivered', 'transport_only', 130, 140)",
            [],
        ));
    }

    #[test]
    fn only_one_active_configuration_and_install_job_are_allowed() {
        let database = Database::open_in_memory().unwrap();
        let connection = &database.connection;
        connection
            .execute(
                "INSERT INTO app_configuration(
                    id, schema_version, config_version, is_active, hotkey_binding,
                    startup_enabled, warmup_enabled, diagnostic_mode, created_at, activated_at
                 ) VALUES ('config-1', 1, 1, 1, 'Ctrl+Space', 0, 0, 0, 100, 100)",
                [],
            )
            .unwrap();
        sqlite_constraint(connection.execute(
            "INSERT INTO app_configuration(
                id, schema_version, config_version, is_active, hotkey_binding,
                startup_enabled, warmup_enabled, diagnostic_mode, created_at, activated_at
             ) VALUES ('config-2', 1, 2, 1, 'Ctrl+Shift+Space', 0, 0, 0, 110, 110)",
            [],
        ));

        insert_model_and_runtime(connection);
        connection
            .execute(
                "INSERT INTO model_install_job(
                    id, model_package_id, state, total_bytes, partial_storage_key,
                    started_at, updated_at
                 ) VALUES ('install-1', 'model-1', 'downloading', 100, 'models/model-1.part',
                    100, 100)",
                [],
            )
            .unwrap();
        sqlite_constraint(connection.execute(
            "INSERT INTO model_install_job(
                id, model_package_id, state, total_bytes, partial_storage_key,
                started_at, updated_at
             ) VALUES ('install-2', 'model-1', 'queued', 100, 'models/model-1-b.part',
                    110, 110)",
            [],
        ));
    }

    #[test]
    fn invalid_terminal_session_state_is_rejected() {
        let database = Database::open_in_memory().unwrap();
        sqlite_constraint(database.connection.execute(
            "INSERT INTO dictation_session(
                id, pipeline_state, state_version, outcome, started_at, created_at, updated_at
             ) VALUES ('session-bad', 'done', 1, 'uncertain', 100, 100, 100)",
            [],
        ));
    }
}
