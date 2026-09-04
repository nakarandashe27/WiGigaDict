use crate::insertion::{
    CompatibilityRegistry, DeliveryRun, InsertionCoordinator, InsertionFailure, InsertionPlatform,
    TransportAttempt,
};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use wigigadict_storage::{
    AsrCompletionMetrics, AsrDispatcher, CaptureCommitPlan, CleanupRepository, Database,
    DeliveryMethod, DeliveryRepository, DeliveryStatus, IntegrityLevel, PcmFormat,
    ReconciliationDisposition, RecoveryRepository, SessionCommitCoordinator, TargetSnapshot,
    TargetSnapshotInput,
};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SESSION_COUNT: usize = 100;
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "wigigadict-step16-golden-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn connection(path: &Path) -> Connection {
    let connection = Connection::open(path).expect("golden database must open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys must be enabled");
    connection
}

fn seed_runtime(path: &Path) {
    let connection = connection(path);
    connection
        .execute(
            "INSERT INTO model_package(
                id,engine_family,model_name,model_version,source_uri,license_id,expected_size,
                checksum_algorithm,checksum,storage_key,install_state,installed_at,created_at,updated_at)
             VALUES('model-golden','whisper','large-v3-turbo-q5','1','managed:model-golden',
                'MIT',100,'sha256',?1,'installed/model-golden','installed',1,1,1)",
            [HASH],
        )
        .expect("selected model must seed");
    connection
        .execute(
            "INSERT INTO runtime_profile(
                id,profile_version,model_package_id,adapter_type,adapter_version,device_kind,
                settings,settings_hash,health_state,enabled,created_at,updated_at)
             VALUES('runtime-golden',1,'model-golden','transcribe-rs','0.3.11','vulkan',
                '{}',?1,'healthy',1,1,1)",
            [HASH],
        )
        .expect("selected runtime must seed");
}

fn plan(index: usize, timestamp: i64) -> CaptureCommitPlan {
    let label = format!("{index:03}");
    CaptureCommitPlan {
        session_id: format!("session-{label}"),
        artifact_id: format!("audio-{label}"),
        commit_id: format!("commit-{label}"),
        prepare_event_id: format!("event-prepare-{label}"),
        finalizing_event_id: format!("event-finalizing-{label}"),
        finalized_event_id: format!("event-finalized-{label}"),
        runtime_profile_id: "runtime-golden".into(),
        asr_attempt_id: format!("asr-{label}"),
        asr_idempotency_key: format!("asr-key-{label}"),
        started_at: timestamp,
        finalized_at: timestamp + 50,
        reserved_byte_size: 4_096,
        format: PcmFormat::MONO_16KHZ_S16,
    }
}

fn target(index: usize, timestamp: i64) -> TargetSnapshotInput {
    let codex = index.is_multiple_of(2);
    TargetSnapshotInput {
        snapshot_id: format!("target-{index:03}"),
        process_identity: if codex { "codex.exe" } else { "code.exe" }.into(),
        process_id: 10 + index as u32,
        thread_id: 1_000 + index as u32,
        window_handle: format!("0x{:016x}", index + 1),
        window_class: "Chrome_WidgetWin_1".into(),
        control_class: "WebViewHost".into(),
        process_version: "1.0.0".into(),
        integrity_level: IntegrityLevel::Medium,
        integrity_rid: 0x2000,
        os_build: 19_045,
        captured_at: timestamp,
    }
}

struct EvidencePlatform {
    acknowledge: bool,
}

impl InsertionPlatform for EvidencePlatform {
    fn revalidate(&mut self, target: &TargetSnapshot) -> Result<String, InsertionFailure> {
        Ok(target.window_handle.clone())
    }

    fn insert(
        &mut self,
        method: DeliveryMethod,
        text: &str,
        _target: &TargetSnapshot,
    ) -> TransportAttempt {
        assert_eq!(method, DeliveryMethod::Unicode);
        let units = u32::try_from(text.encode_utf16().count()).expect("fixture text is bounded");
        TransportAttempt {
            expected_units: units,
            accepted_units: units,
            target_acknowledged: self.acknowledge,
            keyboard_state_safe: Some(true),
            clipboard_set: None,
            clipboard_restored: None,
            failure: None,
        }
    }
}

#[test]
fn one_hundred_completed_sessions_have_zero_irrecoverable_results() {
    let root = fixture_root();
    fs::create_dir_all(&root).expect("fixture root must exist");
    let database_path = root.join("wigigadict.sqlite3");
    let audio_root = root.join("managed");
    drop(Database::open(&database_path).expect("schema must migrate"));
    seed_runtime(&database_path);

    for index in 0..SESSION_COUNT {
        let timestamp = 10_000 + index as i64 * 1_000;
        let plan = plan(index, timestamp);
        let receipt = {
            let mut commit = SessionCommitCoordinator::open(&database_path, &audio_root)
                .expect("commit coordinator must open");
            commit
                .commit_pcm(&plan, &[index as i16; 160])
                .expect("accepted capture must become durable")
        };
        assert!(audio_root.join(&receipt.storage_key).is_file());

        let target = target(index, timestamp);
        {
            let mut repository =
                DeliveryRepository::open(&database_path).expect("delivery repository must open");
            repository
                .capture_initial_target(&plan.session_id, &target)
                .expect("key-down target must persist");
        }

        let raw = {
            let mut dispatcher = AsrDispatcher::open(&database_path).expect("dispatcher must open");
            let mut lease = dispatcher
                .lease_next("golden-worker", timestamp + 60)
                .expect("lease query must work")
                .expect("just-committed capture must be queued");
            dispatcher
                .mark_running(&lease.key, timestamp + 61)
                .expect("lease must start");
            if index.is_multiple_of(10) {
                assert!(
                    dispatcher
                        .release_failure(&lease.key, "fixture_worker_crash", true, timestamp + 62)
                        .expect("transient crash must release")
                );
                lease = dispatcher
                    .lease_next("golden-worker-restarted", timestamp + 63)
                    .expect("restarted lease query must work")
                    .expect("same attempt must return after crash");
                assert_eq!(lease.key.generation, 2);
                dispatcher
                    .mark_running(&lease.key, timestamp + 64)
                    .expect("restarted lease must start");
            }
            dispatcher
                .complete_raw(
                    &lease.key,
                    &format!("raw-{index:03}"),
                    "ну ну сохрани API и не меняй intent",
                    &AsrCompletionMetrics {
                        inference_ms: 700,
                        worker_restarts: lease.key.generation - 1,
                        profile: "gpu".into(),
                    },
                    timestamp + 65,
                )
                .expect("raw transcript must commit")
        };

        let selection = CleanupRepository::open(&database_path)
            .expect("cleanup repository must open")
            .cleanup_raw(&raw.transcript_id, timestamp + 66)
            .expect("deterministic cleanup must select a recoverable transcript");
        assert_eq!(selection.raw.content, "ну ну сохрани API и не меняй intent");

        let acknowledge = !index.is_multiple_of(5);
        let run = InsertionCoordinator::new(
            DeliveryRepository::open(&database_path).expect("delivery repository must reopen"),
            EvidencePlatform { acknowledge },
            CompatibilityRegistry::builtin().expect("built-in registry must verify"),
        )
        .deliver_initial(&selection.selected)
        .expect("delivery policy must finish with durable evidence");
        let DeliveryRun::Completed(delivery) = run else {
            panic!("a fresh session cannot reuse an operation");
        };
        assert_eq!(
            delivery.status,
            if acknowledge {
                DeliveryStatus::Delivered
            } else {
                DeliveryStatus::Uncertain
            }
        );

        if !acknowledge {
            let recovery = RecoveryRepository::open(&database_path, &audio_root)
                .expect("recovery repository must open");
            let recovered = recovery
                .selected_transcript(&plan.session_id)
                .expect("uncertain delivery must retain selected text");
            assert_eq!(recovered.content, selection.selected.content);
        }

        if (index + 1).is_multiple_of(25) {
            let mut restarted = SessionCommitCoordinator::open(&database_path, &audio_root)
                .expect("restart coordinator must open");
            let records = restarted
                .reconcile_startup(timestamp + 100)
                .expect("startup reconciliation must complete");
            assert!(
                records
                    .iter()
                    .all(|record| { record.disposition != ReconciliationDisposition::Corrupt })
            );
        }
    }

    let connection = connection(&database_path);
    let counts: (i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM dictation_session),
                (SELECT COUNT(*) FROM audio_artifact WHERE artifact_state='finalized'),
                (SELECT COUNT(*) FROM asr_attempt),
                (SELECT COUNT(*) FROM transcript_version WHERE kind='raw'),
                (SELECT COUNT(*) FROM delivery_operation WHERE status='delivered'),
                (SELECT COUNT(*) FROM delivery_operation WHERE status='uncertain')",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .expect("aggregate evidence must load");
    assert_eq!(counts, (100, 100, 100, 100, 80, 20));
    let irrecoverable: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM dictation_session session
             WHERE NOT EXISTS(
                    SELECT 1 FROM audio_artifact audio
                    WHERE audio.session_id=session.id AND audio.artifact_state='finalized')
               AND NOT EXISTS(
                    SELECT 1 FROM transcript_version transcript
                    WHERE transcript.session_id=session.id)",
            [],
            |row| row.get(0),
        )
        .expect("zero-loss query must execute");
    assert_eq!(irrecoverable, 0);
    let crash_retries: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM asr_attempt WHERE lease_generation=2",
            [],
            |row| row.get(0),
        )
        .expect("crash retry count must load");
    assert_eq!(crash_retries, 10);
    drop(connection);

    let finalized_files = fs::read_dir(audio_root.join("audio"))
        .expect("audio directory must exist")
        .count();
    assert_eq!(finalized_files, SESSION_COUNT);
    fs::remove_dir_all(root).expect("fixture must clean up");
}
