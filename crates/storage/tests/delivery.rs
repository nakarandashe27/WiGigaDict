#![allow(linker_messages)]

use rusqlite::{Connection, params};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use wigigadict_storage::{
    AttemptStatus, BeginDelivery, Database, DeliveryAttemptInput, DeliveryConclusion,
    DeliveryMethod, DeliveryRepository, DeliveryStatus, EvidenceClass, IntegrityLevel,
    TargetSnapshotInput,
};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wigigadict-step12-{label}-{}-{}",
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

fn seed_session(path: &Path, label: &str) -> (String, String) {
    let session = format!("session-{label}");
    let audio = format!("audio-{label}");
    let attempt = format!("asr-{label}");
    let transcript = format!("raw-{label}");
    let connection = connection(path);
    connection
        .execute(
            "INSERT OR IGNORE INTO model_package(
                id,engine_family,model_name,model_version,source_uri,license_id,expected_size,
                checksum_algorithm,checksum,storage_key,install_state,installed_at,created_at,updated_at)
             VALUES('model-1','whisper','fixture','1','managed:model-1','MIT',100,
                'sha256',?1,'installed/model-1','installed',1,1,1)",
            [HASH],
        )
        .unwrap();
    connection
        .execute(
            "INSERT OR IGNORE INTO runtime_profile(
                id,profile_version,model_package_id,adapter_type,adapter_version,device_kind,
                settings,settings_hash,health_state,enabled,created_at,updated_at)
             VALUES('runtime-1',1,'model-1','transcribe-rs','0.3.11','cpu',
                '{}',?1,'healthy',1,1,1)",
            [HASH],
        )
        .unwrap();
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
             VALUES(?1,?2,'raw',1,'fixture transcript',?3,?4,3)",
            params![transcript, session, HASH, attempt],
        )
        .unwrap();
    (session, transcript)
}

fn target(label: &str) -> TargetSnapshotInput {
    TargetSnapshotInput {
        snapshot_id: format!("target-{label}"),
        process_identity: "fixture.exe".into(),
        process_id: 10,
        thread_id: 11,
        window_handle: "0x0000000000000001".into(),
        window_class: "FixtureWindow".into(),
        control_class: "Edit".into(),
        process_version: "1.2.3".into(),
        integrity_level: IntegrityLevel::Medium,
        integrity_rid: 0x2000,
        os_build: 19_045,
        captured_at: 1,
    }
}

fn attempt(id: &str, evidence_class: EvidenceClass, status: AttemptStatus) -> DeliveryAttemptInput {
    DeliveryAttemptInput {
        attempt_id: id.into(),
        method: DeliveryMethod::Unicode,
        status,
        evidence_class,
        expected_input_units: Some(4),
        accepted_input_units: Some(4),
        foreground_before: "0x0000000000000001".into(),
        foreground_after: Some("0x0000000000000001".into()),
        target_revalidated: true,
        keyboard_state_safe: Some(true),
        clipboard_set: None,
        clipboard_restored: None,
        started_at: 10,
        completed_at: 11,
        error_code: None,
    }
}

#[test]
fn target_ack_is_the_only_unregistered_path_to_delivered() {
    let root = fixture_root("target-ack");
    let path = migrated_database(&root);
    let (session, transcript) = seed_session(&path, "target-ack");
    let mut repository = DeliveryRepository::open(&path).unwrap();
    repository
        .capture_initial_target(&session, &target("target-ack"))
        .unwrap();
    let operation = match repository
        .begin_initial_delivery("operation-target-ack", &session, &transcript, 10)
        .unwrap()
    {
        BeginDelivery::Ready(value) => value,
        BeginDelivery::Existing(_) => panic!("first operation cannot exist"),
    };
    repository
        .append_attempt(
            &operation.operation_id,
            &attempt(
                "attempt-target-ack",
                EvidenceClass::TargetAck,
                AttemptStatus::Delivered,
            ),
        )
        .unwrap();
    let receipt = repository
        .finalize(
            &operation.operation_id,
            &DeliveryConclusion {
                evidence_class: EvidenceClass::TargetAck,
                compatibility_rule: None,
                error_code: None,
                completed_at: 12,
            },
        )
        .unwrap();
    assert_eq!(receipt.status, DeliveryStatus::Delivered);

    let connection = connection(&path);
    let state: (String, String, Option<i64>, Option<i64>) = connection
        .query_row(
            "SELECT pipeline_state,outcome,delivered_at,retention_expires_at
             FROM dictation_session WHERE id=?1",
            [&session],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "done".into(),
            "delivered".into(),
            Some(12),
            Some(12 + wigigadict_storage::DEFAULT_DELIVERED_RETENTION_MS),
        )
    );
    assert!(
        connection
            .execute(
                "UPDATE delivery_attempt SET accepted_input_units=0 WHERE id='attempt-target-ack'",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE delivery_operation SET status='uncertain'
                 WHERE id='operation-target-ack'",
                [],
            )
            .is_err()
    );
    drop(connection);
    drop(repository);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_only_is_uncertain_and_cannot_auto_retry() {
    let root = fixture_root("transport");
    let path = migrated_database(&root);
    let (session, transcript) = seed_session(&path, "transport");
    let mut repository = DeliveryRepository::open(&path).unwrap();
    repository
        .capture_initial_target(&session, &target("transport"))
        .unwrap();
    let operation = match repository
        .begin_initial_delivery("operation-transport", &session, &transcript, 10)
        .unwrap()
    {
        BeginDelivery::Ready(value) => value,
        BeginDelivery::Existing(_) => panic!("first operation cannot exist"),
    };
    repository
        .append_attempt(
            &operation.operation_id,
            &attempt(
                "attempt-transport",
                EvidenceClass::TransportOnly,
                AttemptStatus::Uncertain,
            ),
        )
        .unwrap();
    assert!(
        repository
            .finalize(
                &operation.operation_id,
                &DeliveryConclusion {
                    evidence_class: EvidenceClass::TargetAck,
                    compatibility_rule: None,
                    error_code: None,
                    completed_at: 12,
                },
            )
            .is_err()
    );
    let receipt = repository
        .finalize(
            &operation.operation_id,
            &DeliveryConclusion {
                evidence_class: EvidenceClass::TransportOnly,
                compatibility_rule: None,
                error_code: None,
                completed_at: 12,
            },
        )
        .unwrap();
    assert_eq!(receipt.status, DeliveryStatus::Uncertain);
    let repeated = repository
        .begin_initial_delivery("operation-duplicate", &session, &transcript, 13)
        .unwrap();
    assert!(matches!(
        repeated,
        BeginDelivery::Existing(operation) if operation.status == DeliveryStatus::Uncertain
    ));

    let connection = connection(&path);
    let state: (String, String, Option<i64>, i64) = connection
        .query_row(
            "SELECT pipeline_state,outcome,delivered_at,
                (SELECT COUNT(*) FROM delivery_operation WHERE session_id=?1)
             FROM dictation_session WHERE id=?1",
            [&session],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(state, ("recovery".into(), "uncertain".into(), None, 1));
    drop(connection);
    drop(repository);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_pending_operation_becomes_uncertain_without_replay() {
    let root = fixture_root("interrupted");
    let path = migrated_database(&root);
    let (session, transcript) = seed_session(&path, "interrupted");
    let mut repository = DeliveryRepository::open(&path).unwrap();
    repository
        .capture_initial_target(&session, &target("interrupted"))
        .unwrap();
    repository
        .begin_initial_delivery("operation-interrupted", &session, &transcript, 10)
        .unwrap();

    let repeated = repository
        .begin_initial_delivery("operation-must-not-run", &session, &transcript, 20)
        .unwrap();
    assert!(matches!(
        repeated,
        BeginDelivery::Existing(operation) if operation.status == DeliveryStatus::Uncertain
    ));
    let connection = connection(&path);
    let row: (String, String, i64) = connection
        .query_row(
            "SELECT status,final_error_code,
                (SELECT COUNT(*) FROM delivery_operation WHERE session_id=?2)
             FROM delivery_operation WHERE id=?1",
            params!["operation-interrupted", session],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("uncertain".into(), "delivery_interrupted".into(), 1));
    drop(connection);
    drop(repository);
    fs::remove_dir_all(root).unwrap();
}
