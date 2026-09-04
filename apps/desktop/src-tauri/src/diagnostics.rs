use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;
use uuid::Uuid;
use wigigadict_storage::{
    DIAGNOSTIC_EXPORT_CONFIRMATION, DiagnosticActor, DiagnosticBundlePreview, DiagnosticComponent,
    DiagnosticEventInput, DiagnosticEventName, DiagnosticLogStore, DiagnosticOutcome,
    DiagnosticStage, DiagnosticStatus, PreparedDiagnosticBundle,
};

use crate::shell_lifecycle;
use crate::version::{APP_VERSION, BUILD_COMMIT};

const MAX_EXPORT_PATH_BYTES: usize = 1_024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticView {
    pub expanded_events_enabled: bool,
    #[serde(flatten)]
    pub status: DiagnosticStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DiagnosticExportRequest {
    pub preview_id: String,
    pub destination_path: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticExportReceipt {
    pub file_name: String,
    pub byte_count: u64,
    pub event_count: usize,
}

pub struct DiagnosticService {
    store: Mutex<DiagnosticLogStore>,
    prepared: Mutex<Option<PreparedDiagnosticBundle>>,
    expanded_events_enabled: AtomicBool,
}

impl DiagnosticService {
    pub fn new(managed_root: &Path, expanded_events_enabled: bool) -> Result<Self, String> {
        let root = managed_root.join("logs").join("diagnostics");
        let generation = format!("desktop-{}-{}", std::process::id(), Uuid::new_v4());
        let mut store = DiagnosticLogStore::open(root, &generation, now_ms())
            .map_err(|error| error.to_string())?;
        store
            .append(DiagnosticEventInput::new(
                DiagnosticComponent::Shell,
                DiagnosticEventName::ShellLifecycle,
                DiagnosticStage::Startup,
                DiagnosticOutcome::Succeeded,
                now_ms(),
            ))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            store: Mutex::new(store),
            prepared: Mutex::new(None),
            expanded_events_enabled: AtomicBool::new(expanded_events_enabled),
        })
    }

    pub fn set_expanded_events_enabled(&self, enabled: bool) {
        self.expanded_events_enabled
            .store(enabled, Ordering::Release);
    }

    pub fn record_essential(&self, event: DiagnosticEventInput) {
        if let Ok(mut store) = self.store.lock() {
            let _ = store.append(event);
        }
    }

    pub fn record_extended(&self, event: DiagnosticEventInput) {
        if self.expanded_events_enabled.load(Ordering::Acquire) {
            self.record_essential(event);
        }
    }

    fn view(&self) -> Result<DiagnosticView, String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "diagnostic store mutex was poisoned".to_owned())?;
        Ok(DiagnosticView {
            expanded_events_enabled: self.expanded_events_enabled.load(Ordering::Acquire),
            status: store.status().map_err(|error| error.to_string())?,
        })
    }

    fn prepare(&self) -> Result<DiagnosticBundlePreview, String> {
        self.record_essential(owner_event(
            DiagnosticEventName::BundlePreview,
            DiagnosticStage::Preview,
            DiagnosticOutcome::Started,
        ));
        let bundle = self
            .store
            .lock()
            .map_err(|_| "diagnostic store mutex was poisoned".to_owned())?
            .prepare_bundle(APP_VERSION, safe_build_commit(), now_ms())
            .map_err(|error| error.to_string())?;
        let preview = bundle.preview().clone();
        *self
            .prepared
            .lock()
            .map_err(|_| "diagnostic preview mutex was poisoned".to_owned())? = Some(bundle);
        Ok(preview)
    }

    fn export(&self, request: DiagnosticExportRequest) -> Result<DiagnosticExportReceipt, String> {
        validate_export_request(&request)?;
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "diagnostic preview mutex was poisoned".to_owned())?;
        let bundle = prepared
            .as_ref()
            .ok_or_else(|| "diagnostic preview must be created before export".to_owned())?;
        let preview = bundle.preview().clone();
        let file_name = bundle
            .export(
                &request.preview_id,
                &PathBuf::from(&request.destination_path),
                &request.confirmation,
            )
            .map_err(|error| error.to_string())?;
        *prepared = None;
        drop(prepared);
        self.record_essential(owner_event(
            DiagnosticEventName::BundleExport,
            DiagnosticStage::Export,
            DiagnosticOutcome::Succeeded,
        ));
        Ok(DiagnosticExportReceipt {
            file_name,
            byte_count: preview.byte_count,
            event_count: preview.event_count,
        })
    }
}

#[tauri::command]
pub fn diagnostic_status(
    window: tauri::WebviewWindow,
    service: State<'_, DiagnosticService>,
) -> Result<DiagnosticView, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    service.view()
}

#[tauri::command]
pub fn diagnostic_prepare(
    window: tauri::WebviewWindow,
    service: State<'_, DiagnosticService>,
) -> Result<DiagnosticBundlePreview, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    service.prepare()
}

#[tauri::command]
pub fn diagnostic_export(
    window: tauri::WebviewWindow,
    service: State<'_, DiagnosticService>,
    request: DiagnosticExportRequest,
) -> Result<DiagnosticExportReceipt, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    service.export(request)
}

pub fn capture_event(
    session_id: Option<String>,
    stage: DiagnosticStage,
    outcome: DiagnosticOutcome,
    error_code: Option<&str>,
    byte_count: u64,
) -> DiagnosticEventInput {
    let mut event = DiagnosticEventInput::new(
        if stage == DiagnosticStage::Commit {
            DiagnosticComponent::Storage
        } else {
            DiagnosticComponent::Capture
        },
        if stage == DiagnosticStage::Commit {
            DiagnosticEventName::StorageCommit
        } else {
            DiagnosticEventName::CaptureState
        },
        stage,
        outcome,
        now_ms(),
    );
    event.correlation_id = session_id.clone();
    event.session_id = session_id;
    event.error_code = error_code.map(str::to_owned);
    event.metadata.byte_count = Some(byte_count);
    event
}

pub fn pipeline_event(
    component: DiagnosticComponent,
    event_name: DiagnosticEventName,
    stage: DiagnosticStage,
    outcome: DiagnosticOutcome,
    session_id: &str,
    error_code: Option<&str>,
) -> DiagnosticEventInput {
    let mut event = DiagnosticEventInput::new(component, event_name, stage, outcome, now_ms());
    event.session_id = Some(session_id.to_owned());
    event.correlation_id = Some(session_id.to_owned());
    event.error_code = error_code.map(str::to_owned);
    event
}

fn owner_event(
    event_name: DiagnosticEventName,
    stage: DiagnosticStage,
    outcome: DiagnosticOutcome,
) -> DiagnosticEventInput {
    let mut event = DiagnosticEventInput::new(
        DiagnosticComponent::Diagnostics,
        event_name,
        stage,
        outcome,
        now_ms(),
    );
    event.actor = DiagnosticActor::Owner;
    event
}

fn validate_export_request(request: &DiagnosticExportRequest) -> Result<(), String> {
    if request.preview_id.len() != 64
        || !request
            .preview_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("diagnostic preview id is invalid".into());
    }
    if request.destination_path.is_empty()
        || request.destination_path.len() > MAX_EXPORT_PATH_BYTES
        || request.destination_path.contains('\0')
    {
        return Err("diagnostic export path is invalid".into());
    }
    if request.confirmation != DIAGNOSTIC_EXPORT_CONFIRMATION {
        return Err("explicit diagnostic export confirmation is missing".into());
    }
    Ok(())
}

fn safe_build_commit() -> &'static str {
    let valid = !BUILD_COMMIT.is_empty()
        && BUILD_COMMIT.len() <= 128
        && BUILD_COMMIT.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':')
        });
    if valid { BUILD_COMMIT } else { "unavailable" }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_export_requests_fail_closed() {
        assert!(
            serde_json::from_str::<DiagnosticExportRequest>(
                r#"{"previewId":"a","destinationPath":"x","confirmation":"yes","future":true}"#
            )
            .is_err()
        );
        let request = DiagnosticExportRequest {
            preview_id: "a".repeat(64),
            destination_path: "x".repeat(MAX_EXPORT_PATH_BYTES + 1),
            confirmation: DIAGNOSTIC_EXPORT_CONFIRMATION.into(),
        };
        assert!(validate_export_request(&request).is_err());
    }

    #[test]
    fn build_fingerprint_falls_back_to_content_free_token() {
        assert!(!safe_build_commit().is_empty());
        assert!(safe_build_commit().bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':')
        }));
    }
}
