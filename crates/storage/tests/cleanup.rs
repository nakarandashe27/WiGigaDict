#![allow(linker_messages)]

use rusqlite::{Connection, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use wigigadict_storage::{
    CLEANUP_GLOSSARY_REVISION, CLEANUP_POLICY_HASH, CLEANUP_POLICY_MANIFEST,
    CLEANUP_POLICY_VERSION, CleanupCandidate, CleanupContract, CleanupEngine, CleanupEngineError,
    CleanupFallbackReason, CleanupRepository, CleanupRuleMetrics, Database,
    DeterministicCleanupEngine,
};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
struct CorpusCase {
    id: String,
    input: String,
    expected: String,
    tags: Vec<String>,
}

fn fixture_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wigigadict-step11-{label}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn migrated_database(root: &Path) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    let path = root.join("wigigadict.sqlite3");
    drop(Database::open(&path).unwrap());
    path
}

fn connection(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
}

fn insert_runtime(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO model_package(
                id,engine_family,model_name,model_version,source_uri,license_id,expected_size,
                checksum_algorithm,checksum,storage_key,install_state,installed_at,created_at,updated_at)
             VALUES('model-1','whisper','large-v3-turbo-q5','1','managed:model-1','MIT',100,
                'sha256',?1,'installed/model-1','installed',1,1,1)",
            [HASH],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO runtime_profile(
                id,profile_version,model_package_id,adapter_type,adapter_version,device_kind,
                settings,settings_hash,health_state,enabled,created_at,updated_at)
             VALUES('runtime-1',1,'model-1','transcribe-rs','0.3.11','vulkan',
                '{}',?1,'healthy',1,1,1)",
            [HASH],
        )
        .unwrap();
}

fn insert_raw(connection: &Connection, label: &str, content: &str) -> String {
    let session = format!("session-{label}");
    let audio = format!("audio-{label}");
    let attempt = format!("asr-{label}");
    let raw = format!("raw-{label}");
    connection
        .execute(
            "INSERT INTO dictation_session(
                id,pipeline_state,state_version,started_at,finalized_at,created_at,updated_at)
             VALUES(?1,'processing',3,1,2,1,2)",
            [&session],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO audio_artifact(
                id,session_id,commit_id,staging_storage_key,storage_key,format,duration_ms,
                reserved_byte_size,byte_size,content_hash,artifact_state,created_at)
             VALUES(?1,?2,?3,?4,?5,'wav_pcm_s16le_16000hz_1ch',1000,32000,32000,?6,
                'finalized',1)",
            params![
                audio,
                session,
                format!("commit-{label}"),
                format!("staging/{label}.wav.part"),
                format!("audio/{label}.wav"),
                HASH
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO asr_attempt(
                id,session_id,audio_artifact_id,runtime_profile_id,attempt_no,idempotency_key,
                status,queued_at,started_at,completed_at,metrics)
             VALUES(?1,?2,?3,'runtime-1',1,?4,'succeeded',2,2,3,'{}')",
            params![attempt, session, audio, format!("key-{label}")],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO transcript_version(
                id,session_id,kind,version_no,content,content_hash,source_asr_attempt_id,created_at)
             VALUES(?1,?2,'raw',1,?3,?4,?5,3)",
            params![raw, session, content, HASH, attempt],
        )
        .unwrap();
    raw
}

fn corpus() -> Vec<CorpusCase> {
    serde_json::from_str(include_str!("fixtures/cleanup-corpus-v1.json")).unwrap()
}

#[test]
fn versioned_corpus_is_deterministic_and_preserves_raw_and_protected_tokens() {
    let root = fixture_root("corpus");
    let database = migrated_database(&root);
    let cases = corpus();
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        for case in &cases {
            assert!(!case.tags.is_empty(), "{} has no regression tags", case.id);
            insert_raw(&connection, &case.id, &case.input);
        }
    }

    let mut repository = CleanupRepository::open(&database).unwrap();
    for (index, case) in cases.iter().enumerate() {
        let raw_id = format!("raw-{}", case.id);
        let first = repository.cleanup_raw(&raw_id, 100 + index as i64).unwrap();
        let repeated = repository.cleanup_raw(&raw_id, 200 + index as i64).unwrap();
        assert_eq!(first.raw.content, case.input, "raw changed for {}", case.id);
        assert_eq!(
            first.selected.content, case.expected,
            "unexpected cleanup for {}",
            case.id
        );
        assert_eq!(first, repeated, "cleanup is not idempotent for {}", case.id);
        assert_ne!(first.raw.transcript_id, first.selected.transcript_id);
        assert!(first.cleaned.is_some());
    }
    drop(repository);

    let connection = connection(&database);
    let raw_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM transcript_version WHERE kind='raw'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let cleaned_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM transcript_version WHERE kind='cleaned'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let attempt_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM cleanup_attempt", [], |row| row.get(0))
        .unwrap();
    assert_eq!(raw_count, cases.len() as i64);
    assert_eq!(cleaned_count, cases.len() as i64);
    assert_eq!(attempt_count, cases.len() as i64);
    assert!(
        connection
            .execute(
                "UPDATE transcript_version SET content='mutated' WHERE kind='raw'",
                [],
            )
            .is_err()
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

struct FailingEngine(CleanupEngineError);

impl CleanupEngine for FailingEngine {
    fn cleanup(
        &self,
        _input: &str,
        _contract: &CleanupContract,
    ) -> Result<CleanupCandidate, CleanupEngineError> {
        Err(self.0)
    }
}

#[test]
fn failure_and_timeout_return_raw_without_a_cleaned_version() {
    let root = fixture_root("fallback");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_raw(&connection, "failure", "keep raw failure");
        insert_raw(&connection, "timeout", "keep raw timeout");
    }
    let contract = CleanupContract::builtin();
    let mut repository = CleanupRepository::open(&database).unwrap();
    let failed = repository
        .cleanup_raw_with_contract(
            "raw-failure",
            &contract,
            100,
            &FailingEngine(CleanupEngineError::Failed),
        )
        .unwrap();
    let timed_out = repository
        .cleanup_raw_with_contract(
            "raw-timeout",
            &contract,
            101,
            &FailingEngine(CleanupEngineError::Timeout),
        )
        .unwrap();
    assert_eq!(failed.selected, failed.raw);
    assert_eq!(
        failed.fallback_reason,
        Some(CleanupFallbackReason::EngineFailure)
    );
    assert_eq!(timed_out.selected, timed_out.raw);
    assert_eq!(
        timed_out.fallback_reason,
        Some(CleanupFallbackReason::Timeout)
    );
    drop(repository);

    let connection = connection(&database);
    let cleaned: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM transcript_version WHERE kind='cleaned'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cleaned, 0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

struct DisagreeingEngine;

impl CleanupEngine for DisagreeingEngine {
    fn cleanup(
        &self,
        _input: &str,
        contract: &CleanupContract,
    ) -> Result<CleanupCandidate, CleanupEngineError> {
        Ok(CleanupCandidate {
            content: "change API".into(),
            contract: contract.clone(),
            metrics: CleanupRuleMetrics::default(),
            duration_ms: 7,
        })
    }
}

#[test]
fn contract_disagreement_falls_back_and_diagnostics_never_contain_content() {
    let root = fixture_root("disagreement");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_raw(&connection, "disagreement", "do not change API");
    }
    let mut repository = CleanupRepository::open(&database).unwrap();
    let selection = repository
        .cleanup_raw_with_contract(
            "raw-disagreement",
            &CleanupContract::builtin(),
            100,
            &DisagreeingEngine,
        )
        .unwrap();
    assert_eq!(selection.selected, selection.raw);
    assert_eq!(
        selection.fallback_reason,
        Some(CleanupFallbackReason::ContractDisagreement)
    );
    drop(repository);

    let connection = connection(&database);
    let (metadata, metrics): (String, String) = connection
        .query_row(
            "SELECT diagnostic_event.metadata,cleanup_attempt.metrics
             FROM diagnostic_event JOIN cleanup_attempt USING(session_id)
             WHERE diagnostic_event.event_type='cleanup_disagreement'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(metadata.contains("\"disagreement\":true"));
    assert!(metrics.contains("\"disagreement\":true"));
    for secret in ["do not change API", "change API"] {
        assert!(!metadata.contains(secret));
        assert!(!metrics.contains(secret));
    }
    assert!(
        connection
            .execute(
                "UPDATE diagnostic_event SET metadata='{\"text\":\"secret\"}'",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE cleanup_attempt SET metrics='{\"raw_content\":\"secret\"}'",
                [],
            )
            .is_err()
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

struct CountingEngine<'a>(&'a AtomicUsize);

impl CleanupEngine for CountingEngine<'_> {
    fn cleanup(
        &self,
        input: &str,
        contract: &CleanupContract,
    ) -> Result<CleanupCandidate, CleanupEngineError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        DeterministicCleanupEngine.cleanup(input, contract)
    }
}

#[test]
fn duplicate_completion_reuses_one_attempt_and_policy_mismatch_is_rejected() {
    let root = fixture_root("duplicate");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_raw(&connection, "duplicate", "repeat repeat safely");
    }
    let calls = AtomicUsize::new(0);
    let engine = CountingEngine(&calls);
    let contract = CleanupContract::builtin();
    let mut repository = CleanupRepository::open(&database).unwrap();
    let first = repository
        .cleanup_raw_with_contract("raw-duplicate", &contract, 100, &engine)
        .unwrap();
    let duplicate = repository
        .cleanup_raw_with_contract("raw-duplicate", &contract, 101, &engine)
        .unwrap();
    assert_eq!(first, duplicate);
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    let mut mismatched = contract;
    mismatched.policy_hash =
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
    assert!(
        repository
            .cleanup_raw_with_contract(
                "raw-duplicate",
                &mismatched,
                102,
                &DeterministicCleanupEngine,
            )
            .is_err()
    );
    drop(repository);

    let connection = connection(&database);
    let (attempts, cleaned, version, hash, glossary): (i64, i64, i64, String, i64) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM cleanup_attempt),
                (SELECT COUNT(*) FROM transcript_version WHERE kind='cleaned'),
                policy_version,policy_hash,glossary_revision
             FROM cleanup_attempt",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!((attempts, cleaned), (1, 1));
    assert_eq!(version, i64::from(CLEANUP_POLICY_VERSION));
    assert_eq!(hash, CLEANUP_POLICY_HASH);
    assert_eq!(glossary, i64::from(CLEANUP_GLOSSARY_REVISION));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compiled_policy_manifest_has_the_pinned_hash() {
    let computed = format!("{:x}", Sha256::digest(CLEANUP_POLICY_MANIFEST.as_bytes()));
    assert_eq!(computed, CLEANUP_POLICY_HASH);
}
#[test]
fn idle_restart_scan_cleans_raw_committed_before_repository_start() {
    let root = fixture_root("restart-scan");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_raw(&connection, "restart-scan", "resume resume cleanup");
    }

    let mut repository = CleanupRepository::open(&database).unwrap();
    let selection = repository.cleanup_next_default(100).unwrap().unwrap();
    assert_eq!(selection.raw.transcript_id, "raw-restart-scan");
    assert_eq!(selection.selected.content, "resume cleanup.");
    assert!(repository.cleanup_next_default(101).unwrap().is_none());
    drop(repository);

    let connection = connection(&database);
    let versions: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM transcript_version WHERE session_id='session-restart-scan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(versions, 2);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
