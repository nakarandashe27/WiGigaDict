use crate::{Database, StorageError};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const DEFAULT_DELIVERED_RETENTION_MS: i64 = 15 * 24 * 60 * 60 * 1_000;
const MAX_HISTORY_ITEMS: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    Pending,
    Delivered,
    Uncertain,
    Copied,
    Resolved,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryTranscript {
    pub transcript_id: String,
    pub session_id: String,
    pub kind: String,
    pub content: String,
    pub content_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryAttempt {
    pub attempt_id: String,
    pub ordinal: u32,
    pub method: String,
    pub status: String,
    pub evidence_class: String,
    pub error_code: Option<String>,
    pub started_at: i64,
    pub completed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryOperation {
    pub operation_id: String,
    pub operation_no: u32,
    pub initiated_by: String,
    pub user_action_id: Option<String>,
    pub status: String,
    pub confirmation_level: String,
    pub final_error_code: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub attempts: Vec<RecoveryAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    pub session_id: String,
    pub pipeline_state: String,
    pub state_version: u32,
    pub status: RecoveryStatus,
    pub recovery_required: bool,
    pub raw: Option<RecoveryTranscript>,
    pub cleaned: Option<RecoveryTranscript>,
    pub selected: Option<RecoveryTranscript>,
    pub operations: Vec<RecoveryOperation>,
    pub started_at: i64,
    pub updated_at: i64,
    pub delivered_at: Option<i64>,
    pub resolved_at: Option<i64>,
    pub pinned: bool,
    pub retention_expires_at: Option<i64>,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryActionReceipt {
    pub session_id: String,
    pub state_version: u32,
    pub status: RecoveryStatus,
    pub pinned: bool,
    pub retention_expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReceipt {
    pub journal_id: String,
    pub session_id: String,
    pub deleted_files: u32,
}

#[derive(Debug)]
pub enum RecoveryRepositoryError {
    Storage(StorageError),
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidInput(String),
    Conflict(String),
    Invariant(String),
}

impl Display for RecoveryRepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::Io(error) => write!(formatter, "managed storage I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "deletion journal JSON failed: {error}"),
            Self::InvalidInput(message) => write!(formatter, "invalid recovery input: {message}"),
            Self::Conflict(message) => write!(formatter, "recovery state conflict: {message}"),
            Self::Invariant(message) => write!(formatter, "recovery invariant failed: {message}"),
        }
    }
}

impl Error for RecoveryRepositoryError {}

impl From<StorageError> for RecoveryRepositoryError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<rusqlite::Error> for RecoveryRepositoryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

impl From<std::io::Error> for RecoveryRepositoryError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for RecoveryRepositoryError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type RecoveryRepositoryResult<T> = Result<T, RecoveryRepositoryError>;

type DeleteSessionRow = (u32, String, Option<String>, Option<i64>, Option<i64>);

#[derive(Debug)]
struct SessionRow {
    session_id: String,
    pipeline_state: String,
    state_version: u32,
    outcome: Option<String>,
    started_at: i64,
    updated_at: i64,
    delivered_at: Option<i64>,
    resolved_at: Option<i64>,
    pinned_at: Option<i64>,
    retention_expires_at: Option<i64>,
    last_error_code: Option<String>,
    copied: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeletePlan {
    session_id: String,
    action_id: String,
    storage_keys: Vec<String>,
}

pub struct RecoveryRepository {
    database: Database,
    managed_root: PathBuf,
}

impl RecoveryRepository {
    pub fn open(
        database_path: impl AsRef<Path>,
        managed_root: impl AsRef<Path>,
    ) -> RecoveryRepositoryResult<Self> {
        fs::create_dir_all(managed_root.as_ref())?;
        Ok(Self {
            database: Database::open(database_path)?,
            managed_root: managed_root.as_ref().canonicalize()?,
        })
    }

    pub fn list(&self, limit: u32) -> RecoveryRepositoryResult<Vec<RecoveryEntry>> {
        if limit == 0 || limit > MAX_HISTORY_ITEMS {
            return Err(RecoveryRepositoryError::InvalidInput(format!(
                "history limit must be 1..={MAX_HISTORY_ITEMS}"
            )));
        }
        let mut statement = self
            .database
            .connection
            .prepare("SELECT id FROM dictation_session ORDER BY updated_at DESC,id ASC LIMIT ?1")?;
        let ids = statement
            .query_map([limit], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        ids.iter()
            .map(|session_id| load_entry(&self.database.connection, session_id))
            .collect()
    }

    /// Returns the managed audio key for a session so the desktop shell can mirror the file into
    /// an owner-selected local archive without exposing internal storage paths to the frontend.
    pub fn archive_audio_storage_key(
        &self,
        session_id: &str,
    ) -> RecoveryRepositoryResult<Option<String>> {
        if session_id.is_empty() || session_id.len() > 128 {
            return Err(RecoveryRepositoryError::InvalidInput(
                "session id must contain 1..128 characters".into(),
            ));
        }
        self.database
            .connection
            .query_row(
                "SELECT COALESCE(storage_key,staging_storage_key)
                 FROM audio_artifact
                 WHERE session_id=?1 AND artifact_state<>'deleted'
                 ORDER BY CASE artifact_state WHEN 'committed' THEN 0 ELSE 1 END, created_at
                 LIMIT 1",
                [session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn archive_started_at(&self, session_id: &str) -> RecoveryRepositoryResult<i64> {
        if session_id.is_empty() || session_id.len() > 128 {
            return Err(RecoveryRepositoryError::InvalidInput(
                "session id must contain 1..128 characters".into(),
            ));
        }
        self.database
            .connection
            .query_row(
                "SELECT started_at FROM dictation_session WHERE id=?1",
                [session_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn selected_transcript(
        &self,
        session_id: &str,
    ) -> RecoveryRepositoryResult<RecoveryTranscript> {
        validate_identifier(session_id, "session id")?;
        load_transcript(&self.database.connection, session_id, None)?.ok_or_else(|| {
            RecoveryRepositoryError::Invariant("session has no recoverable transcript".into())
        })
    }

    pub fn record_copy(
        &mut self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        now_ms: i64,
    ) -> RecoveryRepositoryResult<RecoveryActionReceipt> {
        self.user_action(
            session_id,
            expected_state_version,
            action_id,
            now_ms,
            "recovery_copied",
            |transaction| {
                let count: u32 = transaction.query_row(
                    "SELECT COUNT(*) FROM transcript_version WHERE session_id=?1",
                    [session_id],
                    |row| row.get(0),
                )?;
                if count == 0 {
                    return Err(RecoveryRepositoryError::Invariant(
                        "copy requires a recoverable transcript".into(),
                    ));
                }
                Ok(())
            },
        )
    }

    pub fn resolve(
        &mut self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        now_ms: i64,
    ) -> RecoveryRepositoryResult<RecoveryActionReceipt> {
        self.user_action(
            session_id,
            expected_state_version,
            action_id,
            now_ms,
            "recovery_resolved",
            |transaction| {
                let changed = transaction.execute(
                    "UPDATE dictation_session
                     SET outcome='resolved',resolved_at=?3,last_error_code=NULL,
                         state_version=state_version+1,updated_at=?3
                     WHERE id=?1 AND state_version=?2
                       AND (outcome IS NULL OR outcome IN ('uncertain','failed'))",
                    params![session_id, expected_state_version, now_ms],
                )?;
                if changed != 1 {
                    return Err(RecoveryRepositoryError::Conflict(
                        "session is already terminal or changed".into(),
                    ));
                }
                Ok(())
            },
        )
    }

    pub fn set_pinned(
        &mut self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        pinned: bool,
        now_ms: i64,
    ) -> RecoveryRepositoryResult<RecoveryActionReceipt> {
        let event_type = if pinned {
            "recovery_pinned"
        } else {
            "recovery_unpinned"
        };
        self.user_action(
            session_id,
            expected_state_version,
            action_id,
            now_ms,
            event_type,
            |transaction| {
                let changed = transaction.execute(
                    "UPDATE dictation_session
                     SET pinned_at=CASE WHEN ?3=1 THEN ?4 ELSE NULL END,
                         retention_expires_at=CASE
                           WHEN ?3=1 THEN NULL
                           WHEN outcome='delivered' THEN delivered_at+?5
                           ELSE NULL
                         END,
                         state_version=state_version+1,updated_at=?4
                     WHERE id=?1 AND state_version=?2",
                    params![
                        session_id,
                        expected_state_version,
                        i64::from(pinned),
                        now_ms,
                        DEFAULT_DELIVERED_RETENTION_MS
                    ],
                )?;
                if changed != 1 {
                    return Err(RecoveryRepositoryError::Conflict(
                        "session state version changed".into(),
                    ));
                }
                Ok(())
            },
        )
    }

    pub fn journal_delete(
        &mut self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        now_ms: i64,
    ) -> RecoveryRepositoryResult<String> {
        self.prepare_delete(session_id, expected_state_version, action_id, now_ms, false)
    }
    pub fn delete_session(
        &mut self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        now_ms: i64,
    ) -> RecoveryRepositoryResult<DeleteReceipt> {
        let journal_id =
            self.prepare_delete(session_id, expected_state_version, action_id, now_ms, false)?;
        self.execute_delete(&journal_id, now_ms)
    }

    pub fn resume_pending_deletions(
        &mut self,
        now_ms: i64,
    ) -> RecoveryRepositoryResult<Vec<DeleteReceipt>> {
        validate_timestamp(now_ms)?;
        let mut statement = self.database.connection.prepare(
            "SELECT id FROM maintenance_run
             WHERE run_type='session_delete' AND status='running'
             ORDER BY started_at,id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        ids.iter()
            .map(|journal_id| self.execute_delete(journal_id, now_ms))
            .collect()
    }

    pub fn sweep_retention(
        &mut self,
        cutoff_at: i64,
    ) -> RecoveryRepositoryResult<Vec<DeleteReceipt>> {
        validate_timestamp(cutoff_at)?;
        let mut statement = self.database.connection.prepare(
            "SELECT id,state_version FROM dictation_session s
             WHERE pipeline_state='done' AND outcome='delivered'
               AND pinned_at IS NULL AND retention_expires_at IS NOT NULL
               AND retention_expires_at<=?1
               AND NOT EXISTS(
                 SELECT 1 FROM asr_attempt a WHERE a.session_id=s.id
                   AND a.status IN ('queued','leased','running')
               )
               AND NOT EXISTS(
                 SELECT 1 FROM cleanup_attempt c WHERE c.session_id=s.id AND c.status='running'
               )
               AND NOT EXISTS(
                 SELECT 1 FROM delivery_operation d WHERE d.session_id=s.id AND d.status='pending'
               )
             ORDER BY retention_expires_at,id",
        )?;
        let candidates = statement
            .query_map([cutoff_at], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut receipts = Vec::new();
        for (session_id, state_version) in candidates {
            let action_id = format!("retention-{session_id}-{cutoff_at}");
            let journal_id =
                self.prepare_delete(&session_id, state_version, &action_id, cutoff_at, true)?;
            receipts.push(self.execute_delete(&journal_id, cutoff_at)?);
        }
        Ok(receipts)
    }

    fn user_action<F>(
        &mut self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        now_ms: i64,
        event_type: &str,
        mutate: F,
    ) -> RecoveryRepositoryResult<RecoveryActionReceipt>
    where
        F: FnOnce(&Transaction<'_>) -> RecoveryRepositoryResult<()>,
    {
        validate_identifier(session_id, "session id")?;
        validate_action_id(action_id)?;
        validate_timestamp(now_ms)?;
        let event_id = format!("{event_type}-{action_id}");
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_event WHERE id=?1 AND session_id=?2)",
            params![event_id, session_id],
            |row| row.get(0),
        )?;
        if exists {
            transaction.commit()?;
            return action_receipt(&self.database.connection, session_id);
        }
        let current: Option<u32> = transaction
            .query_row(
                "SELECT state_version FROM dictation_session WHERE id=?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        if current != Some(expected_state_version) {
            return Err(RecoveryRepositoryError::Conflict(
                "session state version changed".into(),
            ));
        }
        mutate(&transaction)?;
        if event_type == "recovery_copied" {
            let changed = transaction.execute(
                "UPDATE dictation_session
                 SET state_version=state_version+1,updated_at=?3
                 WHERE id=?1 AND state_version=?2",
                params![session_id, expected_state_version, now_ms],
            )?;
            if changed != 1 {
                return Err(RecoveryRepositoryError::Conflict(
                    "session state version changed".into(),
                ));
            }
        }
        append_user_event(
            &transaction,
            &event_id,
            session_id,
            event_type,
            action_id,
            now_ms,
        )?;
        transaction.commit()?;
        action_receipt(&self.database.connection, session_id)
    }

    fn prepare_delete(
        &mut self,
        session_id: &str,
        expected_state_version: u32,
        action_id: &str,
        now_ms: i64,
        retention: bool,
    ) -> RecoveryRepositoryResult<String> {
        validate_identifier(session_id, "session id")?;
        validate_action_id(action_id)?;
        validate_timestamp(now_ms)?;
        let journal_id = format!("delete-{action_id}");
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT cursor FROM maintenance_run
                 WHERE id=?1 AND run_type='session_delete'",
                [&journal_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(cursor) = existing {
            let plan: DeletePlan = serde_json::from_str(&cursor)?;
            if plan.session_id != session_id || plan.action_id != action_id {
                return Err(RecoveryRepositoryError::Conflict(
                    "deletion action belongs to another session".into(),
                ));
            }
            transaction.commit()?;
            return Ok(journal_id);
        }
        let session: Option<DeleteSessionRow> = transaction
            .query_row(
                "SELECT state_version,pipeline_state,outcome,pinned_at,retention_expires_at
                 FROM dictation_session WHERE id=?1",
                [session_id],
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
            .optional()?;
        let Some((state_version, pipeline_state, outcome, pinned_at, expires_at)) = session else {
            return Err(RecoveryRepositoryError::Conflict(
                "session is missing".into(),
            ));
        };
        if state_version != expected_state_version {
            return Err(RecoveryRepositoryError::Conflict(
                "session state version changed".into(),
            ));
        }
        let active: bool = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM asr_attempt WHERE session_id=?1
                 AND status IN ('queued','leased','running')
               UNION ALL
               SELECT 1 FROM cleanup_attempt WHERE session_id=?1 AND status='running'
               UNION ALL
               SELECT 1 FROM delivery_operation WHERE session_id=?1 AND status='pending'
             )",
            [session_id],
            |row| row.get(0),
        )?;
        if active
            || matches!(
                pipeline_state.as_str(),
                "recording" | "finalizing" | "processing" | "ready_to_deliver" | "delivering"
            )
        {
            return Err(RecoveryRepositoryError::Conflict(
                "active session cannot be deleted".into(),
            ));
        }
        if retention
            && (pipeline_state != "done"
                || outcome.as_deref() != Some("delivered")
                || pinned_at.is_some()
                || expires_at.is_none_or(|value| value > now_ms))
        {
            return Err(RecoveryRepositoryError::Invariant(
                "retention candidate no longer satisfies deletion policy".into(),
            ));
        }
        let mut keys = BTreeSet::new();
        let mut statement = transaction.prepare(
            "SELECT commit_id,staging_storage_key,storage_key
             FROM audio_artifact WHERE session_id=?1",
        )?;
        let artifacts = statement
            .query_map([session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (commit_id, staging_key, storage_key) in artifacts {
            keys.insert(staging_key);
            if let Some(storage_key) = storage_key {
                keys.insert(storage_key);
            }
            keys.insert(format!("quarantine/{commit_id}.staging.orphan"));
            keys.insert(format!("quarantine/{commit_id}.final.orphan"));
        }
        let plan = DeletePlan {
            session_id: session_id.to_owned(),
            action_id: action_id.to_owned(),
            storage_keys: keys.into_iter().collect(),
        };
        let cursor = serde_json::to_string(&plan)?;
        transaction.execute(
            "INSERT INTO maintenance_run(
                id,run_type,cutoff_at,cursor,status,started_at,completed_at,error_code
             ) VALUES(?1,'session_delete',?2,?3,'running',?2,NULL,NULL)",
            params![journal_id, now_ms, cursor],
        )?;
        transaction.commit()?;
        Ok(journal_id)
    }

    fn execute_delete(
        &mut self,
        journal_id: &str,
        now_ms: i64,
    ) -> RecoveryRepositoryResult<DeleteReceipt> {
        let row: Option<(String, String)> = self
            .database
            .connection
            .query_row(
                "SELECT status,cursor FROM maintenance_run
                 WHERE id=?1 AND run_type='session_delete'",
                [journal_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((status, cursor)) = row else {
            return Err(RecoveryRepositoryError::Invariant(
                "deletion journal is missing".into(),
            ));
        };
        let plan: DeletePlan = serde_json::from_str(&cursor)?;
        if status == "succeeded" {
            return Ok(DeleteReceipt {
                journal_id: journal_id.to_owned(),
                session_id: plan.session_id,
                deleted_files: plan.storage_keys.len() as u32,
            });
        }
        if status != "running" {
            return Err(RecoveryRepositoryError::Conflict(
                "deletion journal is not resumable".into(),
            ));
        }
        for key in &plan.storage_keys {
            let path = managed_path(&self.managed_root, key)?;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: bool = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM asr_attempt WHERE session_id=?1
                 AND status IN ('queued','leased','running')
               UNION ALL
               SELECT 1 FROM cleanup_attempt WHERE session_id=?1 AND status='running'
               UNION ALL
               SELECT 1 FROM delivery_operation WHERE session_id=?1 AND status='pending'
             )",
            [&plan.session_id],
            |row| row.get(0),
        )?;
        if active {
            return Err(RecoveryRepositoryError::Conflict(
                "session gained an active job after delete was journaled".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM dictation_session WHERE id=?1",
            [&plan.session_id],
        )?;
        transaction.commit()?;
        checkpoint_truncate(&self.database.connection)?;
        self.database.connection.execute_batch("VACUUM")?;
        self.database.connection.execute(
            "UPDATE maintenance_run
             SET status='succeeded',completed_at=?2,error_code=NULL
             WHERE id=?1 AND status='running'",
            params![journal_id, now_ms],
        )?;
        checkpoint_truncate(&self.database.connection)?;
        Ok(DeleteReceipt {
            journal_id: journal_id.to_owned(),
            session_id: plan.session_id,
            deleted_files: plan.storage_keys.len() as u32,
        })
    }
}

fn load_entry(
    connection: &Connection,
    session_id: &str,
) -> RecoveryRepositoryResult<RecoveryEntry> {
    let row = load_session_row(connection, session_id)?;
    let raw = load_transcript(connection, session_id, Some("raw"))?;
    let cleaned = load_transcript(connection, session_id, Some("cleaned"))?;
    let selected = cleaned.clone().or_else(|| raw.clone());
    let operations = load_operations(connection, session_id)?;
    let status = recovery_status(&row);
    Ok(RecoveryEntry {
        session_id: row.session_id,
        pipeline_state: row.pipeline_state,
        state_version: row.state_version,
        status,
        recovery_required: !matches!(
            row.outcome.as_deref(),
            Some("delivered" | "resolved" | "cancelled")
        ),
        raw,
        cleaned,
        selected,
        operations,
        started_at: row.started_at,
        updated_at: row.updated_at,
        delivered_at: row.delivered_at,
        resolved_at: row.resolved_at,
        pinned: row.pinned_at.is_some(),
        retention_expires_at: row.retention_expires_at,
        last_error_code: row.last_error_code,
    })
}

fn load_session_row(
    connection: &Connection,
    session_id: &str,
) -> RecoveryRepositoryResult<SessionRow> {
    connection
        .query_row(
            "SELECT id,pipeline_state,state_version,outcome,started_at,updated_at,
                    delivered_at,resolved_at,pinned_at,retention_expires_at,last_error_code,
                    EXISTS(
                      SELECT 1 FROM session_event e
                      WHERE e.session_id=dictation_session.id
                        AND e.event_type='recovery_copied'
                    )
             FROM dictation_session WHERE id=?1",
            [session_id],
            |row| {
                Ok(SessionRow {
                    session_id: row.get(0)?,
                    pipeline_state: row.get(1)?,
                    state_version: row.get(2)?,
                    outcome: row.get(3)?,
                    started_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    delivered_at: row.get(6)?,
                    resolved_at: row.get(7)?,
                    pinned_at: row.get(8)?,
                    retention_expires_at: row.get(9)?,
                    last_error_code: row.get(10)?,
                    copied: row.get(11)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| RecoveryRepositoryError::Conflict("session is missing".into()))
}

fn load_transcript(
    connection: &Connection,
    session_id: &str,
    kind: Option<&str>,
) -> RecoveryRepositoryResult<Option<RecoveryTranscript>> {
    let (predicate, kind_value) = match kind {
        Some(value) => ("AND kind=?2", value),
        None => ("", ""),
    };
    let sql = format!(
        "SELECT id,kind,content,content_hash,created_at FROM transcript_version
         WHERE session_id=?1 {predicate}
         ORDER BY CASE kind WHEN 'cleaned' THEN 0 ELSE 1 END,version_no DESC LIMIT 1"
    );
    let result = if kind.is_some() {
        connection
            .query_row(&sql, params![session_id, kind_value], |row| {
                Ok(RecoveryTranscript {
                    transcript_id: row.get(0)?,
                    session_id: session_id.to_owned(),
                    kind: row.get(1)?,
                    content: row.get(2)?,
                    content_hash: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .optional()?
    } else {
        connection
            .query_row(&sql, [session_id], |row| {
                Ok(RecoveryTranscript {
                    transcript_id: row.get(0)?,
                    session_id: session_id.to_owned(),
                    kind: row.get(1)?,
                    content: row.get(2)?,
                    content_hash: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .optional()?
    };
    Ok(result)
}

fn load_operations(
    connection: &Connection,
    session_id: &str,
) -> RecoveryRepositoryResult<Vec<RecoveryOperation>> {
    type OperationRow = (
        String,
        u32,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        i64,
        Option<i64>,
    );
    let mut statement = connection.prepare(
        "SELECT id,operation_no,initiated_by,user_action_id,status,confirmation_level,
                final_error_code,started_at,completed_at
         FROM delivery_operation WHERE session_id=?1 ORDER BY operation_no",
    )?;
    let rows = statement
        .query_map([session_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?
        .collect::<Result<Vec<OperationRow>, _>>()?;
    drop(statement);
    rows.into_iter()
        .map(
            |(
                operation_id,
                operation_no,
                initiated_by,
                user_action_id,
                status,
                confirmation_level,
                final_error_code,
                started_at,
                completed_at,
            )| {
                let mut statement = connection.prepare(
                    "SELECT id,ordinal,method,status,evidence_class,error_code,started_at,completed_at
                     FROM delivery_attempt WHERE delivery_operation_id=?1 ORDER BY ordinal",
                )?;
                let attempts = statement
                    .query_map([&operation_id], |row| {
                        Ok(RecoveryAttempt {
                            attempt_id: row.get(0)?,
                            ordinal: row.get(1)?,
                            method: row.get(2)?,
                            status: row.get(3)?,
                            evidence_class: row.get(4)?,
                            error_code: row.get(5)?,
                            started_at: row.get(6)?,
                            completed_at: row.get(7)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(RecoveryOperation {
                    operation_id,
                    operation_no,
                    initiated_by,
                    user_action_id,
                    status,
                    confirmation_level,
                    final_error_code,
                    started_at,
                    completed_at,
                    attempts,
                })
            },
        )
        .collect()
}

fn recovery_status(row: &SessionRow) -> RecoveryStatus {
    match row.outcome.as_deref() {
        Some("delivered") => RecoveryStatus::Delivered,
        Some("resolved") => RecoveryStatus::Resolved,
        Some("cancelled") => RecoveryStatus::Cancelled,
        _ if row.copied => RecoveryStatus::Copied,
        Some("uncertain" | "failed") => RecoveryStatus::Uncertain,
        _ if row.pipeline_state == "recovery" || row.pipeline_state == "failed" => {
            RecoveryStatus::Uncertain
        }
        _ => RecoveryStatus::Pending,
    }
}

fn action_receipt(
    connection: &Connection,
    session_id: &str,
) -> RecoveryRepositoryResult<RecoveryActionReceipt> {
    let row = load_session_row(connection, session_id)?;
    let status = recovery_status(&row);
    Ok(RecoveryActionReceipt {
        session_id: row.session_id,
        state_version: row.state_version,
        status,
        pinned: row.pinned_at.is_some(),
        retention_expires_at: row.retention_expires_at,
    })
}

fn append_user_event(
    transaction: &Transaction<'_>,
    event_id: &str,
    session_id: &str,
    event_type: &str,
    action_id: &str,
    now_ms: i64,
) -> RecoveryRepositoryResult<()> {
    let sequence_no: u32 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence_no),0)+1 FROM session_event WHERE session_id=?1",
        [session_id],
        |row| row.get(0),
    )?;
    let metadata = serde_json::to_string(&serde_json::json!({
        "user_action_id": action_id
    }))?;
    transaction.execute(
        "INSERT INTO session_event(
            id,session_id,sequence_no,event_type,from_state,to_state,source,reason_code,
            metadata,occurred_at
         ) VALUES(?1,?2,?3,?4,NULL,NULL,'user',NULL,?5,?6)",
        params![
            event_id,
            session_id,
            sequence_no,
            event_type,
            metadata,
            now_ms
        ],
    )?;
    Ok(())
}

fn managed_path(root: &Path, key: &str) -> RecoveryRepositoryResult<PathBuf> {
    let relative = Path::new(key);
    if key.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RecoveryRepositoryError::Invariant(
            "deletion journal contains a non-normalized storage key".into(),
        ));
    }
    Ok(root.join(relative))
}

fn checkpoint_truncate(connection: &Connection) -> RecoveryRepositoryResult<()> {
    let (busy, _, _): (i64, i64, i64) =
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 {
        return Err(RecoveryRepositoryError::Conflict(
            "SQLite WAL checkpoint is busy; deletion remains journaled".into(),
        ));
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> RecoveryRepositoryResult<()> {
    if value.is_empty() || value.len() > 256 {
        return Err(RecoveryRepositoryError::InvalidInput(format!(
            "{label} must contain 1..256 bytes"
        )));
    }
    Ok(())
}

fn validate_action_id(value: &str) -> RecoveryRepositoryResult<()> {
    validate_identifier(value, "action id")?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RecoveryRepositoryError::InvalidInput(
            "action id must be lowercase-safe ASCII".into(),
        ));
    }
    Ok(())
}

fn validate_timestamp(value: i64) -> RecoveryRepositoryResult<()> {
    if value < 0 {
        return Err(RecoveryRepositoryError::InvalidInput(
            "timestamp must be non-negative".into(),
        ));
    }
    Ok(())
}
