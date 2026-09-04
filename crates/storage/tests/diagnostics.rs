#![allow(linker_messages)]
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use wigigadict_storage::{
    DIAGNOSTIC_EXPORT_CONFIRMATION, DiagnosticComponent, DiagnosticError, DiagnosticEventInput,
    DiagnosticEventName, DiagnosticLimits, DiagnosticLogStore, DiagnosticOutcome, DiagnosticStage,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "wigigadict-diagnostics-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn event(now: i64, error: Option<&str>) -> DiagnosticEventInput {
    let mut input = DiagnosticEventInput::new(
        DiagnosticComponent::Storage,
        DiagnosticEventName::StorageCommit,
        DiagnosticStage::Commit,
        if error.is_some() {
            DiagnosticOutcome::Failed
        } else {
            DiagnosticOutcome::Succeeded
        },
        now,
    );
    input.session_id = Some("session-1".into());
    input.correlation_id = Some("commit-1".into());
    input.error_code = error.map(str::to_owned);
    input
}
fn tiny_limits() -> DiagnosticLimits {
    DiagnosticLimits {
        retention_ms: 100,
        max_total_bytes: 2_400,
        max_file_bytes: 700,
        max_files: 3,
        max_event_bytes: 600,
        max_bundle_bytes: 8_000,
    }
}

#[test]
fn marker_secret_is_rejected_and_bundle_excludes_content_fields() {
    let fixture = Fixture::new("content");
    let mut store = DiagnosticLogStore::open(&fixture.0, "process-1", 100).unwrap();
    let mut unsafe_event = event(100, None);
    unsafe_event.error_code = Some("marker_SECRET_value".into());
    assert!(matches!(
        store.append(unsafe_event),
        Err(DiagnosticError::InvalidInput(_))
    ));
    store.append(event(101, Some("commit_failed"))).unwrap();
    let bundle = store.prepare_bundle("0.0.1", "dev", 102).unwrap();
    let destination = fixture.0.join("content.wigigadiag.json");
    bundle
        .export(
            bundle.preview().preview_id.as_str(),
            &destination,
            DIAGNOSTIC_EXPORT_CONFIRMATION,
        )
        .unwrap();
    let serialized = fs::read_to_string(destination).unwrap();
    assert!(!serialized.contains("SECRET"));
    assert!(!serialized.contains("\"transcriptContent\":"));
    assert!(!serialized.contains("\"clipboardContent\":"));
    assert!(!serialized.contains("\"windowTitle\":"));
}

#[test]
fn rotation_retention_and_crash_tail_are_bounded() {
    let fixture = Fixture::new("rotation");
    let mut store =
        DiagnosticLogStore::open_with_limits(&fixture.0, "process-1", 100, tiny_limits()).unwrap();
    for index in 0..12 {
        store
            .append(event(100 + index, Some("commit_failed")))
            .unwrap();
    }
    let status = store.status().unwrap();
    assert!(status.file_count <= tiny_limits().max_files);
    assert!(status.stored_bytes <= tiny_limits().max_total_bytes);
    drop(store);
    let active = fixture.0.join("trace-current.ndjson");
    OpenOptions::new()
        .append(true)
        .open(&active)
        .unwrap()
        .write_all(br#"{"schemaVersion":1"#)
        .unwrap();
    let recovered =
        DiagnosticLogStore::open_with_limits(&fixture.0, "process-2", 200, tiny_limits()).unwrap();
    assert!(recovered.status().unwrap().event_count > 0);
}
#[test]
fn stale_active_trace_is_removed_on_restart() {
    let fixture = Fixture::new("age");
    let mut store =
        DiagnosticLogStore::open_with_limits(&fixture.0, "process-1", 100, tiny_limits()).unwrap();
    store.append(event(100, None)).unwrap();
    drop(store);
    let reopened =
        DiagnosticLogStore::open_with_limits(&fixture.0, "process-2", 1_000, tiny_limits())
            .unwrap();
    assert_eq!(reopened.status().unwrap().event_count, 0);
}

#[test]
fn bundle_fails_closed_on_unknown_entry_and_future_schema() {
    let fixture = Fixture::new("closed");
    let mut store = DiagnosticLogStore::open(&fixture.0, "process-1", 100).unwrap();
    store.append(event(100, None)).unwrap();
    fs::write(fixture.0.join("surprise.txt"), b"unknown").unwrap();
    assert!(matches!(
        store.prepare_bundle("0.0.1", "dev", 101),
        Err(DiagnosticError::UnknownEntry(_))
    ));
    fs::remove_file(fixture.0.join("surprise.txt")).unwrap();
    let active = fixture.0.join("trace-current.ndjson");
    let value = fs::read_to_string(&active)
        .unwrap()
        .replace("\"schemaVersion\":1", "\"schemaVersion\":99");
    fs::write(active, value).unwrap();
    assert!(matches!(
        store.prepare_bundle("0.0.1", "dev", 101),
        Err(DiagnosticError::UnsupportedSchema { .. })
    ));
}

#[test]
fn export_requires_matching_preview_confirmation_and_no_overwrite() {
    let fixture = Fixture::new("export");
    let mut store = DiagnosticLogStore::open(&fixture.0, "process-1", 100).unwrap();
    store.append(event(100, None)).unwrap();
    let bundle = store.prepare_bundle("0.0.1", "dev", 101).unwrap();
    let destination = fixture.0.join("support.wigigadiag.json");
    assert!(
        bundle
            .export("wrong", &destination, DIAGNOSTIC_EXPORT_CONFIRMATION)
            .is_err()
    );
    assert!(
        bundle
            .export(bundle.preview().preview_id.as_str(), &destination, "yes")
            .is_err()
    );
    assert_eq!(
        bundle
            .export(
                bundle.preview().preview_id.as_str(),
                &destination,
                DIAGNOSTIC_EXPORT_CONFIRMATION,
            )
            .unwrap(),
        "support.wigigadiag.json"
    );
    assert!(
        bundle
            .export(
                bundle.preview().preview_id.as_str(),
                &destination,
                DIAGNOSTIC_EXPORT_CONFIRMATION,
            )
            .is_err()
    );
}

#[test]
fn support_trace_preserves_recovery_focus_commit_failure_order() {
    let fixture = Fixture::new("order");
    let mut store = DiagnosticLogStore::open(&fixture.0, "process-1", 100).unwrap();
    let cases = [
        (
            DiagnosticComponent::Storage,
            DiagnosticEventName::StorageReconciliation,
            DiagnosticStage::Recover,
            DiagnosticOutcome::Recovered,
            None,
        ),
        (
            DiagnosticComponent::Delivery,
            DiagnosticEventName::FocusCheck,
            DiagnosticStage::TargetCheck,
            DiagnosticOutcome::Rejected,
            Some("focus_changed"),
        ),
        (
            DiagnosticComponent::Storage,
            DiagnosticEventName::StorageCommit,
            DiagnosticStage::Commit,
            DiagnosticOutcome::Failed,
            Some("commit_failed"),
        ),
    ];
    for (offset, (component, name, stage, outcome, error)) in cases.into_iter().enumerate() {
        let mut input =
            DiagnosticEventInput::new(component, name, stage, outcome, 100 + offset as i64);
        input.session_id = Some("session-1".into());
        input.correlation_id = Some("flow-1".into());
        input.error_code = error.map(str::to_owned);
        store.append(input).unwrap();
    }
    let bundle = store.prepare_bundle("0.0.1", "dev", 200).unwrap();
    let destination = fixture.0.join("order.wigigadiag.json");
    bundle
        .export(
            bundle.preview().preview_id.as_str(),
            &destination,
            DIAGNOSTIC_EXPORT_CONFIRMATION,
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&fs::read(destination).unwrap()).unwrap();
    let events = value["events"].as_array().unwrap();
    assert_eq!(events[0]["eventName"], "storage_reconciliation");
    assert_eq!(events[1]["errorCode"], "focus_changed");
    assert_eq!(events[2]["errorCode"], "commit_failed");
}
