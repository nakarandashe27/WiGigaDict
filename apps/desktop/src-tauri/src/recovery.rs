use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;
use uuid::Uuid;
use wigigadict_storage::{
    AsrDispatcher, DeleteReceipt, DeliveryStatus, RecoveryActionReceipt, RecoveryEntry,
    RecoveryRepository, RecoveryRepositoryError, TranscriptSnapshot,
};

use crate::insertion::{DeliveryRun, WindowsInsertionCoordinator, capture_foreground_target};
use crate::shell_lifecycle;

#[derive(Clone)]
pub struct RecoveryService {
    database_path: PathBuf,
    managed_root: PathBuf,
}

impl RecoveryService {
    pub fn new(database_path: impl AsRef<Path>, managed_root: impl AsRef<Path>) -> Self {
        Self {
            database_path: database_path.as_ref().to_owned(),
            managed_root: managed_root.as_ref().to_owned(),
        }
    }

    pub fn startup_maintenance(&self) -> Result<(), RecoveryRepositoryError> {
        // A session whose last ASR attempt failed for good must become actionable, not stay
        // "В обработке" forever. This runs before the dispatcher so a degraded runtime repairs
        // the same rows.
        if let Ok(mut dispatcher) = AsrDispatcher::open(&self.database_path) {
            let _ = dispatcher.reconcile_failed_attempts(now_ms());
        }
        let mut repository = self.repository()?;
        repository.resume_pending_deletions(now_ms())?;
        repository.sweep_retention(now_ms())?;
        Ok(())
    }

    fn repository(&self) -> Result<RecoveryRepository, RecoveryRepositoryError> {
        RecoveryRepository::open(&self.database_path, &self.managed_root)
    }

    fn list(&self) -> Result<Vec<RecoveryEntry>, String> {
        self.repository()
            .and_then(|repository| repository.list(100))
            .map_err(|error| error.to_string())
    }

    fn record_copy(
        &self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
    ) -> Result<RecoveryActionReceipt, String> {
        self.repository()
            .and_then(|mut repository| {
                repository.record_copy(session_id, expected_state_version, action_id, now_ms())
            })
            .map_err(|error| error.to_string())
    }

    fn resolve(
        &self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
    ) -> Result<RecoveryActionReceipt, String> {
        self.repository()
            .and_then(|mut repository| {
                repository.resolve(session_id, expected_state_version, action_id, now_ms())
            })
            .map_err(|error| error.to_string())
    }

    fn set_pinned(
        &self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        pinned: bool,
    ) -> Result<RecoveryActionReceipt, String> {
        self.repository()
            .and_then(|mut repository| {
                repository.set_pinned(
                    session_id,
                    expected_state_version,
                    action_id,
                    pinned,
                    now_ms(),
                )
            })
            .map_err(|error| error.to_string())
    }

    fn delete(
        &self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
    ) -> Result<DeleteReceipt, String> {
        self.repository()
            .and_then(|mut repository| {
                repository.delete_session(session_id, expected_state_version, action_id, now_ms())
            })
            .map_err(|error| error.to_string())
    }

    fn retry(
        &self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
    ) -> Result<RetryReceipt, String> {
        let transcript = self
            .repository()
            .and_then(|repository| repository.selected_transcript(session_id))
            .map_err(|error| error.to_string())?;
        let snapshot = TranscriptSnapshot {
            transcript_id: transcript.transcript_id,
            session_id: transcript.session_id,
            content: transcript.content,
            content_hash: transcript.content_hash,
        };
        let captured_at = now_ms();
        let target =
            capture_foreground_target(format!("retry-target-{}", Uuid::new_v4()), captured_at)
                .map_err(|failure| failure.code().to_owned())?;
        let mut coordinator = WindowsInsertionCoordinator::open(&self.database_path)
            .map_err(|error| error.to_string())?;
        match coordinator
            .deliver_retry(&snapshot, &target, expected_state_version, action_id)
            .map_err(|error| error.to_string())?
        {
            DeliveryRun::Completed(receipt) => Ok(RetryReceipt {
                session_id: receipt.session_id,
                operation_id: Some(receipt.operation_id),
                status: delivery_status(receipt.status).into(),
            }),
            DeliveryRun::Existing(status) => Ok(RetryReceipt {
                session_id: session_id.to_owned(),
                operation_id: None,
                status: delivery_status(status).into(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryReceipt {
    session_id: String,
    operation_id: Option<String>,
    status: String,
}

#[tauri::command]
pub fn recovery_list(
    window: tauri::WebviewWindow,
    recovery: State<'_, RecoveryService>,
) -> Result<Vec<RecoveryEntry>, String> {
    authorize(&window)?;
    recovery.list()
}

#[tauri::command]
pub fn recovery_retry(
    window: tauri::WebviewWindow,
    recovery: State<'_, RecoveryService>,
    session_id: String,
    expected_state_version: u32,
    action_id: String,
) -> Result<RetryReceipt, String> {
    authorize(&window)?;
    recovery.retry(&session_id, expected_state_version, &action_id)
}

#[tauri::command]
pub fn recovery_record_copy(
    window: tauri::WebviewWindow,
    recovery: State<'_, RecoveryService>,
    session_id: String,
    expected_state_version: u32,
    action_id: String,
) -> Result<RecoveryActionReceipt, String> {
    authorize(&window)?;
    recovery.record_copy(&session_id, expected_state_version, &action_id)
}

#[tauri::command]
pub fn recovery_resolve(
    window: tauri::WebviewWindow,
    recovery: State<'_, RecoveryService>,
    session_id: String,
    expected_state_version: u32,
    action_id: String,
) -> Result<RecoveryActionReceipt, String> {
    authorize(&window)?;
    recovery.resolve(&session_id, expected_state_version, &action_id)
}

#[tauri::command]
pub fn recovery_set_pinned(
    window: tauri::WebviewWindow,
    recovery: State<'_, RecoveryService>,
    session_id: String,
    expected_state_version: u32,
    action_id: String,
    pinned: bool,
) -> Result<RecoveryActionReceipt, String> {
    authorize(&window)?;
    recovery.set_pinned(&session_id, expected_state_version, &action_id, pinned)
}

#[tauri::command]
pub fn recovery_delete(
    window: tauri::WebviewWindow,
    recovery: State<'_, RecoveryService>,
    session_id: String,
    expected_state_version: u32,
    action_id: String,
) -> Result<DeleteReceipt, String> {
    authorize(&window)?;
    recovery.delete(&session_id, expected_state_version, &action_id)
}

fn authorize(window: &tauri::WebviewWindow) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(window).map_err(|error| error.to_string())
}

fn delivery_status(status: DeliveryStatus) -> &'static str {
    match status {
        DeliveryStatus::Pending => "pending",
        DeliveryStatus::Delivered => "delivered",
        DeliveryStatus::Uncertain => "uncertain",
        DeliveryStatus::Failed => "failed",
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
