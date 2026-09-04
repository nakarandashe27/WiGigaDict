use crate::{Database, StorageError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::{Display, Formatter};
use std::io;
use std::path::Path;

pub const MAX_PENDING_SESSIONS: u64 = 20;
pub const MAX_PENDING_PCM_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_SESSION_PCM_BYTES: u64 = 32 * 1024 * 1024;
pub const MIN_DISK_HEADROOM_BYTES: u64 = 1024 * 1024 * 1024;
pub const ASR_LEASE_MS: i64 = 30_000;
const MAX_TRANSCRIPT_BYTES: usize = 1024 * 1024;
const MAX_METRICS_BYTES: usize = 16 * 1024;
const MAX_LEASE_GENERATIONS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionRejection {
    SessionTooLarge,
    PendingSessionLimit,
    PendingPcmLimit,
    InsufficientDisk,
}

impl AdmissionRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::SessionTooLarge => "session_pcm_limit",
            Self::PendingSessionLimit => "asr_pending_session_limit",
            Self::PendingPcmLimit => "asr_pending_pcm_limit",
            Self::InsufficientDisk => "capture_disk_headroom",
        }
    }
}

impl Display for AdmissionRejection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

pub trait CaptureDiskSpace: Send + Sync {
    fn available_bytes(&self, path: &Path) -> io::Result<u64>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemCaptureDiskSpace;

impl CaptureDiskSpace for SystemCaptureDiskSpace {
    fn available_bytes(&self, path: &Path) -> io::Result<u64> {
        available_disk_bytes(path)
    }
}

pub(crate) fn check_capture_disk(
    root: &Path,
    reservation: u64,
    disk: &dyn CaptureDiskSpace,
) -> Result<(), AdmissionRejection> {
    if reservation > MAX_SESSION_PCM_BYTES {
        return Err(AdmissionRejection::SessionTooLarge);
    }
    let required = reservation
        .checked_add(MIN_DISK_HEADROOM_BYTES)
        .ok_or(AdmissionRejection::InsufficientDisk)?;
    let available = disk
        .available_bytes(root)
        .map_err(|_| AdmissionRejection::InsufficientDisk)?;
    if available < required {
        return Err(AdmissionRejection::InsufficientDisk);
    }
    Ok(())
}

pub(crate) fn enforce_capture_admission(
    connection: &Connection,
    reservation: u64,
) -> Result<(), AdmissionRejection> {
    if reservation > MAX_SESSION_PCM_BYTES {
        return Err(AdmissionRejection::SessionTooLarge);
    }
    let (pending, bytes): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(DISTINCT a.session_id),
                    COALESCE(SUM(COALESCE(a.byte_size,a.reserved_byte_size)),0)
             FROM audio_artifact a
             JOIN dictation_session s ON s.id=a.session_id
             WHERE a.artifact_state IN ('writing','finalized')
               AND s.pipeline_state <> 'recovery'
               AND NOT EXISTS (
                   SELECT 1 FROM transcript_version t
                   WHERE t.session_id=a.session_id AND t.kind='raw'
               )",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| AdmissionRejection::PendingSessionLimit)?;
    if pending >= MAX_PENDING_SESSIONS as i64 {
        return Err(AdmissionRejection::PendingSessionLimit);
    }
    let next = u64::try_from(bytes)
        .ok()
        .and_then(|value| value.checked_add(reservation))
        .ok_or(AdmissionRejection::PendingPcmLimit)?;
    if next > MAX_PENDING_PCM_BYTES {
        return Err(AdmissionRejection::PendingPcmLimit);
    }
    Ok(())
}

#[derive(Debug)]
pub enum AsrDispatcherError {
    Storage(StorageError),
    Sqlite(rusqlite::Error),
    InvalidInput(String),
    Conflict(String),
}

impl Display for AsrDispatcherError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => Display::fmt(error, formatter),
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::InvalidInput(detail) => {
                write!(formatter, "invalid ASR dispatcher input: {detail}")
            }
            Self::Conflict(detail) => write!(formatter, "ASR dispatcher conflict: {detail}"),
        }
    }
}

impl std::error::Error for AsrDispatcherError {}

impl From<StorageError> for AsrDispatcherError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<rusqlite::Error> for AsrDispatcherError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub type AsrDispatcherResult<T> = Result<T, AsrDispatcherError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrLeaseKey {
    pub attempt_id: String,
    pub owner: String,
    pub generation: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsrLease {
    pub key: AsrLeaseKey,
    pub session_id: String,
    pub audio_storage_key: String,
    pub audio_sha256: String,
    pub audio_byte_size: u64,
    pub duration_ms: u64,
    pub runtime_profile_id: String,
    pub adapter_type: String,
    pub adapter_version: String,
    pub device_kind: String,
    pub runtime_settings: Value,
    pub model_storage_key: String,
    pub lease_expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrCompletionMetrics {
    pub inference_ms: u64,
    pub worker_restarts: u32,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTranscriptReceipt {
    pub transcript_id: String,
    pub session_id: String,
    pub content_hash: String,
}

pub struct AsrDispatcher {
    database: Database,
}

impl AsrDispatcher {
    pub fn open(path: impl AsRef<Path>) -> AsrDispatcherResult<Self> {
        Ok(Self {
            database: Database::open(path)?,
        })
    }

    pub fn open_in_memory() -> AsrDispatcherResult<Self> {
        Ok(Self {
            database: Database::open_in_memory()?,
        })
    }

    pub fn lease_next(
        &mut self,
        owner: &str,
        now_ms: i64,
    ) -> AsrDispatcherResult<Option<AsrLease>> {
        validate_token("lease owner", owner)?;
        if now_ms < 0 {
            return Err(AsrDispatcherError::InvalidInput(
                "negative timestamp".into(),
            ));
        }
        let expires = now_ms
            .checked_add(ASR_LEASE_MS)
            .ok_or_else(|| AsrDispatcherError::InvalidInput("lease timestamp overflow".into()))?;
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        reclaim_expired(&tx, now_ms)?;
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM asr_attempt WHERE status IN ('leased','running'))",
            [],
            |row| row.get(0),
        )?;
        if active {
            tx.commit()?;
            return Ok(None);
        }
        let attempt_id: Option<String> = tx
            .query_row(
                "SELECT x.id FROM asr_attempt x
                 JOIN dictation_session s ON s.id=x.session_id
                 JOIN audio_artifact a ON a.id=x.audio_artifact_id
                 WHERE x.status='queued' AND a.artifact_state='finalized'
                 ORDER BY s.finalized_at,x.session_id,x.attempt_no LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(attempt_id) = attempt_id else {
            tx.commit()?;
            return Ok(None);
        };
        let changed = tx.execute(
            "UPDATE asr_attempt
             SET status='leased',lease_owner=?2,lease_expires_at=?3,heartbeat_at=?4,
                 lease_generation=lease_generation+1,error_code=NULL
             WHERE id=?1 AND status='queued'",
            params![attempt_id, owner, expires, now_ms],
        )?;
        if changed != 1 {
            return Err(AsrDispatcherError::Conflict(
                "lease compare-and-set failed".into(),
            ));
        }
        let lease = load_lease(&tx, &attempt_id)?;
        tx.commit()?;
        Ok(Some(lease))
    }

    pub fn mark_running(&mut self, key: &AsrLeaseKey, now_ms: i64) -> AsrDispatcherResult<()> {
        validate_key(key)?;
        let changed = self.database.connection.execute(
            "UPDATE asr_attempt SET status='running',started_at=COALESCE(started_at,?4),
                    heartbeat_at=?4
             WHERE id=?1 AND lease_owner=?2 AND lease_generation=?3 AND status='leased'
               AND lease_expires_at>=?4",
            params![key.attempt_id, key.owner, key.generation, now_ms],
        )?;
        expect_one(changed, "mark-running")
    }

    pub fn heartbeat(&mut self, key: &AsrLeaseKey, now_ms: i64) -> AsrDispatcherResult<i64> {
        validate_key(key)?;
        let expires = now_ms
            .checked_add(ASR_LEASE_MS)
            .ok_or_else(|| AsrDispatcherError::InvalidInput("lease timestamp overflow".into()))?;
        let changed = self.database.connection.execute(
            "UPDATE asr_attempt SET heartbeat_at=?4,lease_expires_at=?5
             WHERE id=?1 AND lease_owner=?2 AND lease_generation=?3
               AND status IN ('leased','running') AND lease_expires_at>=?4",
            params![key.attempt_id, key.owner, key.generation, now_ms, expires],
        )?;
        expect_one(changed, "heartbeat")?;
        Ok(expires)
    }

    pub fn complete_raw(
        &mut self,
        key: &AsrLeaseKey,
        transcript_id: &str,
        content: &str,
        metrics: &AsrCompletionMetrics,
        now_ms: i64,
    ) -> AsrDispatcherResult<RawTranscriptReceipt> {
        validate_key(key)?;
        validate_token("transcript id", transcript_id)?;
        if content.len() > MAX_TRANSCRIPT_BYTES {
            return Err(AsrDispatcherError::InvalidInput(
                "raw transcript is too large".into(),
            ));
        }
        let content_hash = format!("{:x}", Sha256::digest(content.as_bytes()));
        if let Some(existing) = self.existing_receipt(&key.attempt_id)? {
            if existing.content_hash == content_hash {
                return Ok(existing);
            }
            return Err(AsrDispatcherError::Conflict(
                "attempt already has a different immutable raw transcript".into(),
            ));
        }
        let metrics_json = serde_json::to_string(metrics)
            .map_err(|error| AsrDispatcherError::InvalidInput(error.to_string()))?;
        if metrics_json.len() > MAX_METRICS_BYTES {
            return Err(AsrDispatcherError::InvalidInput(
                "ASR metrics are too large".into(),
            ));
        }
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session_id: String = tx
            .query_row(
                "SELECT session_id FROM asr_attempt
                 WHERE id=?1 AND lease_owner=?2 AND lease_generation=?3
                   AND status IN ('leased','running')",
                params![key.attempt_id, key.owner, key.generation],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| AsrDispatcherError::Conflict("completion lease is stale".into()))?;
        tx.execute(
            "INSERT INTO transcript_version(
                id,session_id,kind,version_no,content,content_hash,source_asr_attempt_id,created_at)
             VALUES(?1,?2,'raw',
                (SELECT COALESCE(MAX(version_no),0)+1 FROM transcript_version
                 WHERE session_id=?2 AND kind='raw'),?3,?4,?5,?6)",
            params![
                transcript_id,
                session_id,
                content,
                content_hash,
                key.attempt_id,
                now_ms
            ],
        )?;
        let changed = tx.execute(
            "UPDATE asr_attempt SET status='succeeded',completed_at=?4,error_code=NULL,
                    metrics=?5,heartbeat_at=?4,lease_expires_at=NULL
             WHERE id=?1 AND lease_owner=?2 AND lease_generation=?3
               AND status IN ('leased','running')",
            params![
                key.attempt_id,
                key.owner,
                key.generation,
                now_ms,
                metrics_json
            ],
        )?;
        expect_one(changed, "complete")?;
        tx.commit()?;
        Ok(RawTranscriptReceipt {
            transcript_id: transcript_id.into(),
            session_id,
            content_hash,
        })
    }

    pub fn release_failure(
        &mut self,
        key: &AsrLeaseKey,
        error_code: &str,
        transient: bool,
        now_ms: i64,
    ) -> AsrDispatcherResult<bool> {
        validate_key(key)?;
        validate_error_code(error_code)?;
        let retry = transient && key.generation < MAX_LEASE_GENERATIONS;
        let (status, completed): (&str, Option<i64>) = if retry {
            ("queued", None)
        } else {
            ("failed", Some(now_ms))
        };
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE asr_attempt SET status=?4,started_at=COALESCE(started_at,queued_at),
                    completed_at=?5,error_code=?6,lease_owner=NULL,lease_expires_at=NULL,
                    heartbeat_at=NULL
             WHERE id=?1 AND lease_owner=?2 AND lease_generation=?3
               AND status IN ('leased','running')",
            params![
                key.attempt_id,
                key.owner,
                key.generation,
                status,
                completed,
                error_code
            ],
        )?;
        expect_one(changed, "release-failure")?;
        if !retry {
            fail_owning_session(&transaction, &key.attempt_id, error_code, now_ms)?;
        }
        transaction.commit()?;
        Ok(retry)
    }

    /// Startup repair for sessions stranded in `processing` by a terminally failed attempt.
    ///
    /// Rows written before `release_failure` moved the session itself are still out there, and a
    /// crash between the attempt update and the session update could produce one again. Returns
    /// how many sessions were released; running it twice releases nothing the second time.
    pub fn reconcile_failed_attempts(&mut self, now_ms: i64) -> AsrDispatcherResult<u32> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stranded = {
            let mut statement = transaction.prepare(
                "SELECT a.id FROM asr_attempt a
                 JOIN dictation_session s ON s.id=a.session_id
                 WHERE a.status='failed' AND s.pipeline_state='processing'
                   AND NOT EXISTS (SELECT 1 FROM asr_attempt b WHERE b.session_id=s.id
                                     AND b.status IN ('queued','leased','running','succeeded'))
                 ORDER BY a.completed_at,a.id",
            )?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut released = 0_u32;
        for attempt_id in &stranded {
            let error_code: String = transaction.query_row(
                "SELECT COALESCE(error_code,'asr_failed') FROM asr_attempt WHERE id=?1",
                [attempt_id.as_str()],
                |row| row.get(0),
            )?;
            fail_owning_session(&transaction, attempt_id, &error_code, now_ms)?;
            released += 1;
        }
        transaction.commit()?;
        Ok(released)
    }

    pub fn cancel(&mut self, key: &AsrLeaseKey, now_ms: i64) -> AsrDispatcherResult<()> {
        validate_key(key)?;
        let changed = self.database.connection.execute(
            "UPDATE asr_attempt SET status='cancelled',started_at=COALESCE(started_at,queued_at),
                    completed_at=?4,error_code='cancelled',lease_owner=NULL,lease_expires_at=NULL,
                    heartbeat_at=NULL
             WHERE id=?1 AND lease_owner=?2 AND lease_generation=?3
               AND status IN ('leased','running')",
            params![key.attempt_id, key.owner, key.generation, now_ms],
        )?;
        expect_one(changed, "cancel")
    }

    fn existing_receipt(
        &self,
        attempt_id: &str,
    ) -> AsrDispatcherResult<Option<RawTranscriptReceipt>> {
        Ok(self
            .database
            .connection
            .query_row(
                "SELECT id,session_id,content_hash FROM transcript_version
                 WHERE source_asr_attempt_id=?1 AND kind='raw'",
                [attempt_id],
                |row| {
                    Ok(RawTranscriptReceipt {
                        transcript_id: row.get(0)?,
                        session_id: row.get(1)?,
                        content_hash: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }
}

fn reclaim_expired(connection: &Connection, now_ms: i64) -> AsrDispatcherResult<()> {
    connection.execute(
        "UPDATE asr_attempt SET status='failed',started_at=COALESCE(started_at,queued_at),
                completed_at=?1,error_code='lease_expired',lease_owner=NULL,
                lease_expires_at=NULL,heartbeat_at=NULL
         WHERE status IN ('leased','running') AND lease_expires_at<?1
           AND lease_generation>=?2",
        params![now_ms, MAX_LEASE_GENERATIONS],
    )?;
    connection.execute(
        "UPDATE asr_attempt SET status='queued',error_code='lease_expired',lease_owner=NULL,
                lease_expires_at=NULL,heartbeat_at=NULL
         WHERE status IN ('leased','running') AND lease_expires_at<?1
           AND lease_generation<?2",
        params![now_ms, MAX_LEASE_GENERATIONS],
    )?;
    Ok(())
}

fn load_lease(connection: &Connection, attempt_id: &str) -> AsrDispatcherResult<AsrLease> {
    let row = connection.query_row(
        "SELECT x.session_id,a.storage_key,a.content_hash,a.byte_size,a.duration_ms,
                x.runtime_profile_id,p.adapter_type,p.adapter_version,p.device_kind,p.settings,
                m.storage_key,x.lease_owner,x.lease_generation,x.lease_expires_at
         FROM asr_attempt x JOIN audio_artifact a ON a.id=x.audio_artifact_id
         JOIN runtime_profile p ON p.id=x.runtime_profile_id
         JOIN model_package m ON m.id=p.model_package_id
         WHERE x.id=?1 AND x.status='leased'",
        [attempt_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
            ))
        },
    )?;
    let settings = serde_json::from_str(&row.9)
        .map_err(|error| AsrDispatcherError::InvalidInput(error.to_string()))?;
    Ok(AsrLease {
        key: AsrLeaseKey {
            attempt_id: attempt_id.into(),
            owner: row.11,
            generation: u32::try_from(row.12)
                .map_err(|_| AsrDispatcherError::InvalidInput("invalid lease generation".into()))?,
        },
        session_id: row.0,
        audio_storage_key: row.1,
        audio_sha256: row.2,
        audio_byte_size: u64::try_from(row.3)
            .map_err(|_| AsrDispatcherError::InvalidInput("invalid audio size".into()))?,
        duration_ms: u64::try_from(row.4)
            .map_err(|_| AsrDispatcherError::InvalidInput("invalid audio duration".into()))?,
        runtime_profile_id: row.5,
        adapter_type: row.6,
        adapter_version: row.7,
        device_kind: row.8,
        runtime_settings: settings,
        model_storage_key: row.10,
        lease_expires_at: row.13,
    })
}

fn validate_key(key: &AsrLeaseKey) -> AsrDispatcherResult<()> {
    validate_token("attempt id", &key.attempt_id)?;
    validate_token("lease owner", &key.owner)?;
    if key.generation == 0 {
        return Err(AsrDispatcherError::InvalidInput(
            "zero lease generation".into(),
        ));
    }
    Ok(())
}

fn validate_token(name: &str, value: &str) -> AsrDispatcherResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(AsrDispatcherError::InvalidInput(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_error_code(value: &str) -> AsrDispatcherResult<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(AsrDispatcherError::InvalidInput(
            "invalid error code".into(),
        ));
    }
    Ok(())
}

/// Moves the owning session out of `processing` when its last attempt failed for good.
///
/// Without this the attempt was marked `failed` while the session stayed `processing`: the owner
/// saw a dictation that was permanently "being processed", could not retry it (there is no
/// transcript) and could not delete it (`processing` counts as an active pipeline state).
fn fail_owning_session(
    transaction: &rusqlite::Transaction<'_>,
    attempt_id: &str,
    error_code: &str,
    now_ms: i64,
) -> AsrDispatcherResult<()> {
    let session_id: String = transaction.query_row(
        "SELECT session_id FROM asr_attempt WHERE id=?1",
        [attempt_id],
        |row| row.get(0),
    )?;
    let changed = transaction.execute(
        "UPDATE dictation_session SET pipeline_state='recovery',state_version=state_version+1,
                outcome='uncertain',last_error_code=?2,updated_at=?3
         WHERE id=?1 AND pipeline_state='processing'",
        params![session_id, error_code, now_ms],
    )?;
    if changed == 0 {
        return Ok(());
    }
    let sequence_no: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence_no),0)+1 FROM session_event WHERE session_id=?1",
        [session_id.as_str()],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO session_event(id,session_id,sequence_no,event_type,from_state,
                to_state,source,reason_code,occurred_at)
         VALUES(?1,?2,?3,'asr_failed','processing','recovery','system',?4,?5)",
        params![
            format!("asr-failed-{attempt_id}"),
            session_id,
            sequence_no,
            error_code,
            now_ms
        ],
    )?;
    Ok(())
}

fn expect_one(changed: usize, operation: &str) -> AsrDispatcherResult<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(AsrDispatcherError::Conflict(format!(
            "{operation} lease is stale"
        )))
    }
}

#[cfg(windows)]
fn available_disk_bytes(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    // SAFETY: wide is NUL-terminated and available is a valid output pointer.
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

#[cfg(not(windows))]
fn available_disk_bytes(_path: &Path) -> io::Result<u64> {
    Ok(u64::MAX)
}
