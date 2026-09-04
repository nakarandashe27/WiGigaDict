#![allow(linker_messages)]

use rusqlite::{Connection, params};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use wigigadict_storage::{
    AdmissionRejection, AsrCompletionMetrics, AsrDispatcher, CaptureCommitPlan, CaptureDiskSpace,
    Database, MAX_PENDING_PCM_BYTES, MAX_SESSION_PCM_BYTES, PcmFormat, SessionCommitCoordinator,
    SessionCommitError,
};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wigigadict-step10-{label}-{}-{}",
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
                '{\"worker_path\":\"worker.exe\",\"model_path\":\"model.bin\",\"profile\":\"gpu\"}',
                ?1,'healthy',1,1,1)",
            [HASH],
        )
        .unwrap();
}

fn insert_queued(connection: &Connection, label: &str, finalized_at: i64, bytes: i64) {
    let session = format!("session-{label}");
    let audio = format!("audio-{label}");
    connection
        .execute(
            "INSERT INTO dictation_session(
                id,pipeline_state,state_version,started_at,finalized_at,created_at,updated_at)
             VALUES(?1,'processing',3,1,?2,1,?2)",
            params![session, finalized_at],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO audio_artifact(
                id,session_id,commit_id,staging_storage_key,storage_key,format,duration_ms,
                reserved_byte_size,byte_size,content_hash,artifact_state,created_at)
             VALUES(?1,?2,?3,?4,?5,'wav_pcm_s16le_16000hz_1ch',1000,?6,?6,?7,'finalized',1)",
            params![
                audio,
                session,
                format!("commit-{label}"),
                format!("staging/{label}.wav.part"),
                format!("audio/{label}.wav"),
                bytes,
                HASH
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO asr_attempt(
                id,session_id,audio_artifact_id,runtime_profile_id,attempt_no,idempotency_key,
                status,queued_at)
             VALUES(?1,?2,?3,'runtime-1',1,?4,'queued',?5)",
            params![
                format!("asr-{label}"),
                session,
                audio,
                format!("key-{label}"),
                finalized_at
            ],
        )
        .unwrap();
}

fn plan(label: &str, reserved_byte_size: u64) -> CaptureCommitPlan {
    CaptureCommitPlan {
        session_id: format!("new-session-{label}"),
        artifact_id: format!("new-audio-{label}"),
        commit_id: format!("new-commit-{label}"),
        prepare_event_id: format!("new-prepare-{label}"),
        finalizing_event_id: format!("new-finalizing-{label}"),
        finalized_event_id: format!("new-finalized-{label}"),
        runtime_profile_id: "runtime-1".into(),
        asr_attempt_id: format!("new-asr-{label}"),
        asr_idempotency_key: format!("new-key-{label}"),
        started_at: 1_000,
        finalized_at: 1_000,
        reserved_byte_size,
        format: PcmFormat::MONO_16KHZ_S16,
    }
}

struct FixedDisk(u64);

impl CaptureDiskSpace for FixedDisk {
    fn available_bytes(&self, _path: &Path) -> std::io::Result<u64> {
        Ok(self.0)
    }
}

fn admission(error: SessionCommitError) -> AdmissionRejection {
    match error {
        SessionCommitError::Admission(reason) => reason,
        other => panic!("expected admission rejection, got {other}"),
    }
}

#[test]
fn expired_worker_lease_returns_the_same_attempt_and_completion_is_idempotent() {
    let root = fixture_root("lease-restart");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_queued(&connection, "one", 100, 32_000);
    }

    let mut dispatcher = AsrDispatcher::open(&database).unwrap();
    let first = dispatcher.lease_next("worker-a", 100).unwrap().unwrap();
    assert_eq!(first.key.attempt_id, "asr-one");
    assert_eq!(first.key.generation, 1);
    dispatcher.mark_running(&first.key, 101).unwrap();

    let restarted = dispatcher.lease_next("worker-b", 30_101).unwrap().unwrap();
    assert_eq!(restarted.key.attempt_id, first.key.attempt_id);
    assert_eq!(restarted.key.generation, 2);
    let metrics = AsrCompletionMetrics {
        inference_ms: 250,
        worker_restarts: 1,
        profile: "gpu".into(),
    };
    let receipt = dispatcher
        .complete_raw(&restarted.key, "raw-one", "immutable raw", &metrics, 30_102)
        .unwrap();
    let repeated = dispatcher
        .complete_raw(&restarted.key, "raw-one", "immutable raw", &metrics, 30_103)
        .unwrap();
    assert_eq!(receipt, repeated);
    assert!(dispatcher.lease_next("worker-c", 30_104).unwrap().is_none());
    drop(dispatcher);

    let connection = connection(&database);
    let attempts: i64 = connection
        .query_row("SELECT COUNT(*) FROM asr_attempt", [], |row| row.get(0))
        .unwrap();
    assert_eq!(attempts, 1);
    assert!(
        connection
            .execute(
                "UPDATE transcript_version SET content='mutated' WHERE id='raw-one'",
                [],
            )
            .is_err()
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dispatcher_is_fifo_and_never_issues_a_second_active_lease() {
    let root = fixture_root("fifo");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_queued(&connection, "later", 200, 32_000);
        insert_queued(&connection, "first", 100, 32_000);
    }
    let mut dispatcher = AsrDispatcher::open(&database).unwrap();
    let first = dispatcher.lease_next("worker-a", 1_000).unwrap().unwrap();
    assert_eq!(first.key.attempt_id, "asr-first");
    assert!(dispatcher.lease_next("worker-b", 1_001).unwrap().is_none());
    assert!(
        dispatcher
            .release_failure(&first.key, "worker_crash", true, 1_002)
            .unwrap()
    );
    let retry = dispatcher.lease_next("worker-b", 1_003).unwrap().unwrap();
    assert_eq!(retry.key.attempt_id, first.key.attempt_id);
    assert_eq!(retry.key.generation, 2);
    drop(dispatcher);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn twenty_first_pending_session_is_rejected_before_capture_state_is_created() {
    let root = fixture_root("session-limit");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        for index in 0..20 {
            insert_queued(&connection, &format!("pending-{index}"), 100 + index, 1);
        }
    }
    let mut coordinator = SessionCommitCoordinator::open(&database, &root).unwrap();
    let error = coordinator
        .prepare_pcm_writer_with_disk(
            &plan("twenty-one", MAX_SESSION_PCM_BYTES),
            &FixedDisk(u64::MAX),
        )
        .err()
        .expect("capture must be rejected");
    assert_eq!(admission(error), AdmissionRejection::PendingSessionLimit);
    drop(coordinator);
    let connection = connection(&database);
    let created: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM dictation_session WHERE id='new-session-twenty-one'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(created, 0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aggregate_pcm_budget_and_oversize_session_are_rejected() {
    let root = fixture_root("pcm-limit");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        for index in 0..8 {
            insert_queued(
                &connection,
                &format!("large-{index}"),
                100 + index,
                MAX_SESSION_PCM_BYTES as i64,
            );
        }
    }
    let mut coordinator = SessionCommitCoordinator::open(&database, &root).unwrap();
    let aggregate = coordinator
        .prepare_pcm_writer_with_disk(
            &plan("aggregate", MAX_SESSION_PCM_BYTES),
            &FixedDisk(u64::MAX),
        )
        .err()
        .expect("capture must be rejected");
    assert_eq!(admission(aggregate), AdmissionRejection::PendingPcmLimit);
    let oversize = coordinator
        .prepare_pcm_writer_with_disk(
            &plan("oversize", MAX_SESSION_PCM_BYTES + 1),
            &FixedDisk(u64::MAX),
        )
        .err()
        .expect("capture must be rejected");
    assert_eq!(admission(oversize), AdmissionRejection::SessionTooLarge);
    drop(coordinator);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_sessions_release_the_pending_asr_budget() {
    let root = fixture_root("recovery-budget");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
    }
    let mut coordinator = SessionCommitCoordinator::open(&database, &root).unwrap();
    for index in 0..8 {
        let recovery_plan = plan(&format!("recovery-{index}"), MAX_SESSION_PCM_BYTES);
        let writer = coordinator
            .prepare_pcm_writer_with_disk(&recovery_plan, &FixedDisk(u64::MAX))
            .unwrap();
        coordinator
            .recover_pcm_writer(
                &recovery_plan,
                writer,
                recovery_plan.started_at,
                "test_recovery",
            )
            .unwrap();
    }
    let next = plan("after-recovery", MAX_SESSION_PCM_BYTES);
    let writer = coordinator
        .prepare_pcm_writer_with_disk(&next, &FixedDisk(u64::MAX))
        .expect("recovery artifacts are not pending ASR work");
    drop(writer);
    drop(coordinator);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn low_disk_is_rejected_before_sqlite_capture_prepare() {
    let root = fixture_root("low-disk");
    let database = migrated_database(&root);
    let mut coordinator = SessionCommitCoordinator::open(&database, &root).unwrap();
    let error = coordinator
        .prepare_pcm_writer_with_disk(&plan("low-disk", MAX_SESSION_PCM_BYTES), &FixedDisk(0))
        .err()
        .expect("capture must be rejected");
    assert_eq!(admission(error), AdmissionRejection::InsufficientDisk);
    drop(coordinator);
    let connection = connection(&database);
    let sessions: i64 = connection
        .query_row("SELECT COUNT(*) FROM dictation_session", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(sessions, 0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn documented_pcm_budget_constant_remains_256_mib() {
    assert_eq!(MAX_PENDING_PCM_BYTES, 256 * 1024 * 1024);
}

#[test]
fn a_terminally_failed_attempt_releases_its_session_into_recovery() {
    let root = fixture_root("terminal-failure");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_queued(&connection, "stuck", 100, 32_000);
    }
    let mut dispatcher = AsrDispatcher::open(&database).unwrap();

    // Generation 1 is the transient GPU attempt: it is requeued, and the session keeps working.
    let first = dispatcher.lease_next("worker-a", 1_000).unwrap().unwrap();
    assert!(
        dispatcher
            .release_failure(&first.key, "worker_exited", true, 1_001)
            .unwrap()
    );
    let connection = connection(&database);
    assert_eq!(session_state(&connection, "session-stuck"), "processing");

    // Generation 2 is the last one. Its failure is terminal, so the session must stop being
    // "processing": that state made the entry undeletable and unretryable in the recovery UI.
    let second = dispatcher.lease_next("worker-a", 1_002).unwrap().unwrap();
    assert_eq!(second.key.generation, 2);
    assert!(
        !dispatcher
            .release_failure(&second.key, "worker_exited", true, 1_003)
            .unwrap()
    );
    assert_eq!(session_state(&connection, "session-stuck"), "recovery");
    let (outcome, error_code, version): (String, String, i64) = connection
        .query_row(
            "SELECT outcome,last_error_code,state_version FROM dictation_session WHERE id=?1",
            ["session-stuck"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(outcome, "uncertain");
    assert_eq!(error_code, "worker_exited");
    assert_eq!(version, 4, "exactly one state transition");
    let events: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM session_event WHERE session_id=?1 AND event_type='asr_failed'",
            ["session-stuck"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);

    // The startup repair is idempotent and does not touch an already released session.
    assert_eq!(dispatcher.reconcile_failed_attempts(1_004).unwrap(), 0);
    assert_eq!(
        session_version(&connection, "session-stuck"),
        4,
        "repair must not advance a released session"
    );
    drop(connection);
    drop(dispatcher);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_repair_releases_sessions_stranded_by_an_older_build() {
    let root = fixture_root("stranded-repair");
    let database = migrated_database(&root);
    {
        let connection = connection(&database);
        insert_runtime(&connection);
        insert_queued(&connection, "old", 100, 32_000);
        // Exactly the rows an older build left behind: attempt failed, session still processing.
        connection
            .execute(
                "UPDATE asr_attempt SET status='failed',started_at=queued_at,
                        completed_at=queued_at+1,error_code='worker_exited'
                 WHERE session_id='session-old'",
                [],
            )
            .unwrap();
    }
    let mut dispatcher = AsrDispatcher::open(&database).unwrap();
    let connection = connection(&database);
    assert_eq!(session_state(&connection, "session-old"), "processing");
    assert_eq!(dispatcher.reconcile_failed_attempts(2_000).unwrap(), 1);
    assert_eq!(session_state(&connection, "session-old"), "recovery");
    // Idempotent: a second startup releases nothing and leaves the version alone.
    let version = session_version(&connection, "session-old");
    assert_eq!(dispatcher.reconcile_failed_attempts(2_001).unwrap(), 0);
    assert_eq!(session_version(&connection, "session-old"), version);
    drop(connection);
    drop(dispatcher);
    fs::remove_dir_all(root).unwrap();
}

fn session_state(connection: &Connection, session_id: &str) -> String {
    connection
        .query_row(
            "SELECT pipeline_state FROM dictation_session WHERE id=?1",
            [session_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn session_version(connection: &Connection, session_id: &str) -> i64 {
    connection
        .query_row(
            "SELECT state_version FROM dictation_session WHERE id=?1",
            [session_id],
            |row| row.get(0),
        )
        .unwrap()
}
