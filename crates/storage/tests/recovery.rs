#![allow(linker_messages)]

use rusqlite::{Connection, params};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use wigigadict_storage::{
    AttemptStatus, BeginDelivery, Database, DeliveryAttemptInput, DeliveryConclusion,
    DeliveryMethod, DeliveryRepository, DeliveryStatus, EvidenceClass, IntegrityLevel,
    RecoveryRepository, RecoveryStatus, TargetSnapshotInput,
};

const HASH: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Seed {
    session_id: String,
    transcript_id: String,
    commit_id: String,
}

fn fixture_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wigigadict-step13-{label}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn migrated_database(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join("staging")).unwrap();
    fs::create_dir_all(root.join("audio")).unwrap();
    fs::create_dir_all(root.join("quarantine")).unwrap();
    let path = root.join("wigigadict.sqlite3");
    drop(Database::open(&path).unwrap());
    seed_runtime(&path);
    path
}

fn connection(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .pragma_update(None, "secure_delete", true)
        .unwrap();
    connection
}

fn seed_runtime(path: &Path) {
    let connection = connection(path);
    connection
        .execute(
            "INSERT OR IGNORE INTO model_package(
                id,engine_family,model_name,model_version,source_uri,license_id,expected_size,
                checksum_algorithm,checksum,storage_key,install_state,installed_at,created_at,updated_at)
             VALUES('model-recovery','whisper','fixture','1','managed:model','MIT',100,
                'sha256',?1,'installed/model','installed',1,1,1)",
            [HASH],
        )
        .unwrap();
    connection
        .execute(
            "INSERT OR IGNORE INTO runtime_profile(
                id,profile_version,model_package_id,adapter_type,adapter_version,device_kind,
                settings,settings_hash,health_state,enabled,created_at,updated_at)
             VALUES('runtime-recovery',1,'model-recovery','fixture','1','cpu',
                '{}',?1,'healthy',1,1,1)",
            [HASH],
        )
        .unwrap();
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fixture spells out retention policy state"
)]
fn seed_session(
    path: &Path,
    root: &Path,
    label: &str,
    pipeline_state: &str,
    outcome: Option<&str>,
    delivered_at: Option<i64>,
    pinned_at: Option<i64>,
    retention_expires_at: Option<i64>,
    content: &str,
) -> Seed {
    let session_id = format!("session-{label}");
    let artifact_id = format!("audio-{label}");
    let asr_id = format!("asr-{label}");
    let transcript_id = format!("raw-{label}");
    let commit_id = format!("commit-{label}");
    let staging_key = format!("staging/{commit_id}.wav.part");
    let storage_key = format!("audio/{commit_id}.wav");
    let connection = connection(path);
    connection
        .execute(
            "INSERT INTO dictation_session(
                id,pipeline_state,state_version,outcome,started_at,finalized_at,delivered_at,
                pinned_at,retention_expires_at,last_error_code,created_at,updated_at)
             VALUES(?1,?2,3,?3,1,2,?4,?5,?6,
                CASE WHEN ?3='uncertain' THEN 'delivery_uncertain' ELSE NULL END,1,10)",
            params![
                session_id,
                pipeline_state,
                outcome,
                delivered_at,
                pinned_at,
                retention_expires_at
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO audio_artifact(
                id,session_id,commit_id,staging_storage_key,storage_key,format,duration_ms,
                reserved_byte_size,byte_size,content_hash,artifact_state,created_at)
             VALUES(?1,?2,?3,?4,?5,'wav_pcm_s16le_16000hz_1ch',1,64,64,?6,'finalized',1)",
            params![
                artifact_id,
                session_id,
                commit_id,
                staging_key,
                storage_key,
                HASH
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO asr_attempt(
                id,session_id,audio_artifact_id,runtime_profile_id,attempt_no,idempotency_key,
                status,queued_at,started_at,completed_at,metrics)
             VALUES(?1,?2,?3,'runtime-recovery',1,?4,'succeeded',2,2,3,'{}')",
            params![asr_id, session_id, artifact_id, format!("asr-key-{label}")],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO transcript_version(
                id,session_id,kind,version_no,content,content_hash,source_asr_attempt_id,created_at)
             VALUES(?1,?2,'raw',1,?3,?4,?5,3)",
            params![transcript_id, session_id, content, HASH, asr_id],
        )
        .unwrap();
    for key in [
        format!("staging/{commit_id}.wav.part"),
        format!("audio/{commit_id}.wav"),
        format!("quarantine/{commit_id}.staging.orphan"),
        format!("quarantine/{commit_id}.final.orphan"),
    ] {
        fs::write(root.join(key), content.as_bytes()).unwrap();
    }
    Seed {
        session_id,
        transcript_id,
        commit_id,
    }
}

fn seed_uncertain_delivery(path: &Path, seed: &Seed) {
    let connection = connection(path);
    connection
        .execute(
            "INSERT INTO target_snapshot(
                id,session_id,purpose,process_identity,process_id,window_handle,window_class,
                integrity_level,captured_at,thread_id,control_class,process_version,integrity_rid,
                os_build)
             VALUES(?1,?2,'initial','fixture.exe',10,'0x1','Fixture','medium',4,11,'Edit',
                '1.0.0',8192,19045)",
            params![format!("initial-{}", seed.commit_id), seed.session_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO delivery_operation(
                id,session_id,transcript_version_id,target_snapshot_id,operation_no,initiated_by,
                status,confirmation_level,started_at,completed_at,final_error_code)
             VALUES(?1,?2,?3,?4,1,'system','uncertain','transport_only',5,6,
                'delivery_uncertain')",
            params![
                format!("old-operation-{}", seed.commit_id),
                seed.session_id,
                seed.transcript_id,
                format!("initial-{}", seed.commit_id)
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO delivery_attempt(
                id,delivery_operation_id,ordinal,method,status,evidence_class,
                expected_input_units,accepted_input_units,foreground_before,foreground_after,
                target_revalidated,keyboard_state_safe,started_at,completed_at,error_code)
             VALUES(?1,?2,1,'unicode','uncertain','transport_only',4,4,'0x1','0x1',1,1,5,6,
                'delivery_uncertain')",
            params![
                format!("old-attempt-{}", seed.commit_id),
                format!("old-operation-{}", seed.commit_id)
            ],
        )
        .unwrap();
}

fn retry_target(label: &str) -> TargetSnapshotInput {
    TargetSnapshotInput {
        snapshot_id: format!("retry-target-{label}"),
        process_identity: "fixture.exe".into(),
        process_id: 20,
        thread_id: 21,
        window_handle: "0x2".into(),
        window_class: "Fixture".into(),
        control_class: "Edit".into(),
        process_version: "1.0.0".into(),
        integrity_level: IntegrityLevel::Medium,
        integrity_rid: 0x2000,
        os_build: 19_045,
        captured_at: 20,
    }
}

#[test]
fn restart_projection_uses_session_aggregate_and_keeps_attempts_immutable() {
    let root = fixture_root("projection");
    let path = migrated_database(&root);
    let seed = seed_session(
        &path,
        &root,
        "projection",
        "recovery",
        Some("uncertain"),
        None,
        None,
        None,
        "recoverable projection text",
    );
    seed_uncertain_delivery(&path, &seed);
    let repository = RecoveryRepository::open(&path, &root).unwrap();
    let entries = repository.list(20).unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry.session_id == seed.session_id)
        .unwrap();
    assert_eq!(entry.status, RecoveryStatus::Uncertain);
    assert!(entry.recovery_required);
    assert_eq!(
        entry.selected.as_ref().unwrap().content,
        "recoverable projection text"
    );
    assert_eq!(entry.operations.len(), 1);
    assert_eq!(entry.operations[0].attempts.len(), 1);
    assert_eq!(entry.operations[0].attempts[0].method, "unicode");
    let connection = connection(&path);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM transcript_version WHERE session_id=?1",
                [&seed.session_id],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        1
    );
    assert!(
        connection
            .execute(
                "UPDATE delivery_attempt SET status='failed' WHERE id=?1",
                [format!("old-attempt-{}", seed.commit_id)]
            )
            .is_err()
    );
    drop(connection);
    drop(repository);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_pin_and_resolve_are_idempotent_and_version_checked() {
    let root = fixture_root("actions");
    let path = migrated_database(&root);
    let seed = seed_session(
        &path,
        &root,
        "actions",
        "recovery",
        Some("uncertain"),
        None,
        None,
        None,
        "action text",
    );
    let mut repository = RecoveryRepository::open(&path, &root).unwrap();
    let copied = repository
        .record_copy(&seed.session_id, 3, "copy-action", 20)
        .unwrap();
    assert_eq!(copied.state_version, 4);
    assert_eq!(copied.status, RecoveryStatus::Copied);
    let repeated = repository
        .record_copy(&seed.session_id, 3, "copy-action", 21)
        .unwrap();
    assert_eq!(repeated, copied);
    let pinned = repository
        .set_pinned(&seed.session_id, 4, "pin-action", true, 22)
        .unwrap();
    assert!(pinned.pinned);
    assert_eq!(pinned.state_version, 5);
    assert!(
        repository
            .resolve(&seed.session_id, 4, "stale-resolve", 23)
            .is_err()
    );
    let resolved = repository
        .resolve(&seed.session_id, 5, "resolve-action", 24)
        .unwrap();
    assert_eq!(resolved.status, RecoveryStatus::Resolved);
    assert_eq!(resolved.state_version, 6);
    drop(repository);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_retry_uses_a_new_target_and_user_action_without_replay() {
    let root = fixture_root("retry");
    let path = migrated_database(&root);
    let seed = seed_session(
        &path,
        &root,
        "retry",
        "recovery",
        Some("uncertain"),
        None,
        None,
        None,
        "retry text",
    );
    seed_uncertain_delivery(&path, &seed);
    let target = retry_target("retry");
    let mut repository = DeliveryRepository::open(&path).unwrap();
    let operation = match repository
        .begin_retry_delivery(
            "retry-operation",
            &seed.session_id,
            &seed.transcript_id,
            3,
            "retry-action",
            &target,
            20,
        )
        .unwrap()
    {
        BeginDelivery::Ready(operation) => operation,
        BeginDelivery::Existing(_) => panic!("first explicit retry must be new"),
    };
    assert_eq!(operation.operation_no, 2);
    repository
        .append_attempt(
            &operation.operation_id,
            &DeliveryAttemptInput {
                attempt_id: "retry-attempt".into(),
                method: DeliveryMethod::Unicode,
                status: AttemptStatus::Uncertain,
                evidence_class: EvidenceClass::TransportOnly,
                expected_input_units: Some(4),
                accepted_input_units: Some(4),
                foreground_before: "0x2".into(),
                foreground_after: Some("0x2".into()),
                target_revalidated: true,
                keyboard_state_safe: Some(true),
                clipboard_set: None,
                clipboard_restored: None,
                started_at: 21,
                completed_at: 22,
                error_code: None,
            },
        )
        .unwrap();
    let receipt = repository
        .finalize(
            &operation.operation_id,
            &DeliveryConclusion {
                evidence_class: EvidenceClass::TransportOnly,
                compatibility_rule: None,
                error_code: None,
                completed_at: 23,
            },
        )
        .unwrap();
    assert_eq!(receipt.status, DeliveryStatus::Uncertain);
    let repeated = repository
        .begin_retry_delivery(
            "must-not-run",
            &seed.session_id,
            &seed.transcript_id,
            3,
            "retry-action",
            &target,
            24,
        )
        .unwrap();
    assert!(matches!(repeated, BeginDelivery::Existing(existing)
        if existing.operation_id == "retry-operation"
            && existing.status == DeliveryStatus::Uncertain));
    let connection = connection(&path);
    let counts: (u32, u32, u32) = connection
        .query_row(
            "SELECT
            (SELECT COUNT(*) FROM delivery_operation WHERE session_id=?1),
            (SELECT COUNT(*) FROM delivery_attempt a
                JOIN delivery_operation o ON o.id=a.delivery_operation_id WHERE o.session_id=?1),
            (SELECT COUNT(*) FROM target_snapshot WHERE session_id=?1 AND purpose='retry')",
            [&seed.session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(counts, (2, 2, 1));
    drop(connection);
    drop(repository);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retention_sweep_deletes_only_expired_delivered_unpinned_sessions() {
    let root = fixture_root("retention");
    let path = migrated_database(&root);
    let expired = seed_session(
        &path,
        &root,
        "expired",
        "done",
        Some("delivered"),
        Some(10),
        None,
        Some(20),
        "expired owned text",
    );
    let pinned = seed_session(
        &path,
        &root,
        "pinned",
        "done",
        Some("delivered"),
        Some(10),
        Some(11),
        None,
        "pinned owned text",
    );
    let unresolved = seed_session(
        &path,
        &root,
        "unresolved",
        "recovery",
        Some("uncertain"),
        None,
        None,
        None,
        "unresolved owned text",
    );
    let fresh = seed_session(
        &path,
        &root,
        "fresh",
        "done",
        Some("delivered"),
        Some(10),
        None,
        Some(200),
        "fresh owned text",
    );
    let mut repository = RecoveryRepository::open(&path, &root).unwrap();
    let receipts = repository.sweep_retention(100).unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].session_id, expired.session_id);
    let connection = connection(&path);
    for session_id in [pinned.session_id, unresolved.session_id, fresh.session_id] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM dictation_session WHERE id=?1",
                    [session_id],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            1
        );
    }
    drop(connection);
    drop(repository);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deletion_journal_resumes_after_restart_and_removes_every_owned_copy() {
    let root = fixture_root("delete");
    let path = migrated_database(&root);
    let secret = "owned-delete-secret-7f7f19";
    let seed = seed_session(
        &path,
        &root,
        "delete",
        "recovery",
        Some("uncertain"),
        None,
        None,
        None,
        secret,
    );
    let mut repository = RecoveryRepository::open(&path, &root).unwrap();
    let journal_id = repository
        .journal_delete(&seed.session_id, 3, "delete-action", 20)
        .unwrap();
    drop(repository);
    assert_eq!(
        connection(&path)
            .query_row(
                "SELECT COUNT(*) FROM dictation_session WHERE id=?1",
                [&seed.session_id],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        1
    );
    assert!(root.join(format!("audio/{}.wav", seed.commit_id)).exists());
    let mut restarted = RecoveryRepository::open(&path, &root).unwrap();
    let receipts = restarted.resume_pending_deletions(21).unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].journal_id, journal_id);
    drop(restarted);
    let connection = connection(&path);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM dictation_session WHERE id=?1",
                [&seed.session_id],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM maintenance_run WHERE id=?1",
                [&journal_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "succeeded"
    );
    drop(connection);
    for relative in [
        format!("staging/{}.wav.part", seed.commit_id),
        format!("audio/{}.wav", seed.commit_id),
        format!("quarantine/{}.staging.orphan", seed.commit_id),
        format!("quarantine/{}.final.orphan", seed.commit_id),
    ] {
        assert!(!root.join(relative).exists());
    }
    let needle = secret.as_bytes();
    for physical in [
        path.clone(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if physical.exists() {
            let bytes = fs::read(&physical).unwrap();
            assert!(
                !bytes.windows(needle.len()).any(|window| window == needle),
                "owned transcript remained in {}",
                physical.display()
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}
