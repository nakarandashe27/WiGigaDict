use crate::asr_dispatcher::{
    AdmissionRejection, CaptureDiskSpace, SystemCaptureDiskSpace, check_capture_disk,
    enforce_capture_admission,
};
use crate::{Database, StorageError};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

const WAV_HEADER_BYTES: u64 = 44;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl PcmFormat {
    pub const MONO_16KHZ_S16: Self = Self {
        sample_rate_hz: 16_000,
        channels: 1,
        bits_per_sample: 16,
    };

    fn validate(self) -> CommitResult<()> {
        if !(8_000..=192_000).contains(&self.sample_rate_hz) {
            return Err(SessionCommitError::InvalidInput(
                "sample_rate_hz must be between 8000 and 192000".into(),
            ));
        }
        if !(1..=8).contains(&self.channels) {
            return Err(SessionCommitError::InvalidInput(
                "channels must be between 1 and 8".into(),
            ));
        }
        if self.bits_per_sample != 16 {
            return Err(SessionCommitError::InvalidInput(
                "Step 6 writer accepts only PCM signed 16-bit samples".into(),
            ));
        }
        Ok(())
    }

    fn block_align(self) -> u16 {
        self.channels * (self.bits_per_sample / 8)
    }

    fn storage_label(self) -> String {
        format!(
            "wav_pcm_s16le_{}hz_{}ch",
            self.sample_rate_hz, self.channels
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureCommitPlan {
    pub session_id: String,
    pub artifact_id: String,
    pub commit_id: String,
    pub prepare_event_id: String,
    pub finalizing_event_id: String,
    pub finalized_event_id: String,
    pub runtime_profile_id: String,
    pub asr_attempt_id: String,
    pub asr_idempotency_key: String,
    pub started_at: i64,
    pub finalized_at: i64,
    pub reserved_byte_size: u64,
    pub format: PcmFormat,
}

impl CaptureCommitPlan {
    fn validate(&self) -> CommitResult<()> {
        for (name, value) in [
            ("session_id", self.session_id.as_str()),
            ("artifact_id", self.artifact_id.as_str()),
            ("prepare_event_id", self.prepare_event_id.as_str()),
            ("finalizing_event_id", self.finalizing_event_id.as_str()),
            ("finalized_event_id", self.finalized_event_id.as_str()),
            ("runtime_profile_id", self.runtime_profile_id.as_str()),
            ("asr_attempt_id", self.asr_attempt_id.as_str()),
            ("asr_idempotency_key", self.asr_idempotency_key.as_str()),
        ] {
            if value.is_empty() {
                return Err(SessionCommitError::InvalidInput(format!(
                    "{name} must not be empty"
                )));
            }
        }
        validate_commit_id(&self.commit_id)?;
        self.format.validate()?;
        if self.started_at < 0 || self.finalized_at < self.started_at {
            return Err(SessionCommitError::InvalidInput(
                "timestamps must be non-negative and finalized_at >= started_at".into(),
            ));
        }
        if self.reserved_byte_size <= WAV_HEADER_BYTES || self.reserved_byte_size > i64::MAX as u64
        {
            return Err(SessionCommitError::InvalidInput(
                "reserved_byte_size is outside the supported SQLite/WAV range".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitCheckpoint {
    AfterPrepareCommit,
    AfterPartWrite,
    AfterArtifactFlush,
    AfterAtomicRename,
    DuringCheckpointCommit,
    AfterCheckpointCommit,
}

#[derive(Debug)]
pub enum SessionCommitError {
    Storage(StorageError),
    Io(std::io::Error),
    InvalidInput(String),
    Conflict(String),
    Admission(AdmissionRejection),
    Injected(CommitCheckpoint),
}

impl Display for SessionCommitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "storage error: {e}"),
            Self::Io(e) => write!(f, "filesystem error: {e}"),
            Self::InvalidInput(d) => write!(f, "invalid commit input: {d}"),
            Self::Conflict(d) => write!(f, "commit conflict: {d}"),
            Self::Admission(reason) => write!(f, "capture admission rejected: {reason}"),
            Self::Injected(p) => write!(f, "fault injected at {p:?}"),
        }
    }
}
impl std::error::Error for SessionCommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
impl From<StorageError> for SessionCommitError {
    fn from(v: StorageError) -> Self {
        Self::Storage(v)
    }
}
impl From<rusqlite::Error> for SessionCommitError {
    fn from(v: rusqlite::Error) -> Self {
        Self::Storage(StorageError::Sqlite(v))
    }
}
impl From<std::io::Error> for SessionCommitError {
    fn from(v: std::io::Error) -> Self {
        Self::Io(v)
    }
}
impl From<AdmissionRejection> for SessionCommitError {
    fn from(value: AdmissionRejection) -> Self {
        Self::Admission(value)
    }
}
pub type CommitResult<T> = std::result::Result<T, SessionCommitError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReceipt {
    pub commit_id: String,
    pub session_id: String,
    pub artifact_id: String,
    pub asr_attempt_id: String,
    pub storage_key: String,
    pub byte_size: u64,
    pub content_hash: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReceipt {
    pub commit_id: String,
    pub session_id: String,
    pub staging_storage_key: String,
    pub byte_size: u64,
    pub content_hash: Option<String>,
    pub duration_ms: Option<u64>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationDisposition {
    Continue,
    Recovery,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationRecord {
    pub commit_id: String,
    pub disposition: ReconciliationDisposition,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ManagedAudioStore {
    root: PathBuf,
    staging: PathBuf,
    finalized: PathBuf,
    quarantine: PathBuf,
}

impl ManagedAudioStore {
    pub fn open(root: impl AsRef<Path>) -> CommitResult<Self> {
        fs::create_dir_all(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        let staging = root.join("staging");
        let finalized = root.join("audio");
        let quarantine = root.join("quarantine");
        fs::create_dir_all(&staging)?;
        fs::create_dir_all(&finalized)?;
        fs::create_dir_all(&quarantine)?;
        Ok(Self {
            root,
            staging,
            finalized,
            quarantine,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    fn staging_key(commit_id: &str) -> String {
        format!("staging/{commit_id}.wav.part")
    }
    fn final_key(commit_id: &str) -> String {
        format!("audio/{commit_id}.wav")
    }

    fn path_for_key(&self, key: &str) -> CommitResult<PathBuf> {
        let relative = Path::new(key);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(SessionCommitError::InvalidInput(format!(
                "managed storage key is not normalized: {key}"
            )));
        }
        Ok(self.root.join(relative))
    }

    pub fn create_writer(
        &self,
        commit_id: &str,
        reserved: u64,
        format: PcmFormat,
    ) -> CommitResult<PcmPartWriter> {
        validate_commit_id(commit_id)?;
        format.validate()?;
        PcmPartWriter::create(
            self.path_for_key(&Self::staging_key(commit_id))?,
            reserved,
            format,
        )
    }

    fn promote(&self, artifact: &FlushedArtifact) -> CommitResult<PathBuf> {
        let final_path = self.path_for_key(&Self::final_key(&artifact.commit_id))?;
        write_through_move_no_replace(&artifact.staging_path, &final_path)?;
        let actual = inspect_wav(&final_path, artifact.format)?;
        if actual.byte_size != artifact.byte_size
            || actual.content_hash != artifact.content_hash
            || actual.duration_ms != artifact.duration_ms
        {
            return Err(SessionCommitError::Conflict(
                "promoted WAV identity changed".into(),
            ));
        }
        Ok(final_path)
    }
}

pub struct PcmPartWriter {
    commit_id: String,
    path: PathBuf,
    file: Option<File>,
    format: PcmFormat,
    reserved_byte_size: u64,
    data_bytes: u64,
}

impl PcmPartWriter {
    fn create(path: PathBuf, reserved_byte_size: u64, format: PcmFormat) -> CommitResult<Self> {
        if reserved_byte_size <= WAV_HEADER_BYTES {
            return Err(SessionCommitError::InvalidInput(
                "PCM reservation is too small".into(),
            ));
        }
        let commit_id = commit_id_from_path(&path, ".wav.part")?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
        file.write_all(&wav_header(format, 0)?)?;
        Ok(Self {
            commit_id,
            path,
            file: Some(file),
            format,
            reserved_byte_size,
            data_bytes: 0,
        })
    }

    pub fn write_samples(&mut self, samples: &[i16]) -> CommitResult<()> {
        let incoming = u64::try_from(samples.len())
            .ok()
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| SessionCommitError::InvalidInput("PCM block size overflow".into()))?;
        let next = self
            .data_bytes
            .checked_add(incoming)
            .ok_or_else(|| SessionCommitError::InvalidInput("PCM artifact size overflow".into()))?;
        if WAV_HEADER_BYTES
            .checked_add(next)
            .is_none_or(|size| size > self.reserved_byte_size)
        {
            return Err(SessionCommitError::Conflict(
                "PCM reservation exceeded".into(),
            ));
        }
        if next > u32::MAX as u64 {
            return Err(SessionCommitError::Conflict(
                "RIFF u32 size exceeded".into(),
            ));
        }
        let mut encoded = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            encoded.extend_from_slice(&sample.to_le_bytes());
        }
        self.file_mut()?.write_all(&encoded)?;
        self.data_bytes = next;
        Ok(())
    }

    pub fn written_file_bytes(&self) -> u64 {
        WAV_HEADER_BYTES + self.data_bytes
    }

    fn finish(mut self) -> CommitResult<FlushedArtifact> {
        if self.data_bytes == 0 {
            return Err(SessionCommitError::InvalidInput(
                "cannot finalize empty PCM".into(),
            ));
        }
        let data_bytes = u32::try_from(self.data_bytes)
            .map_err(|_| SessionCommitError::InvalidInput("WAV data exceeds u32".into()))?;
        let header = wav_header(self.format, data_bytes)?;
        let format = self.format;
        {
            let file = self.file_mut()?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&header)?;
            file.flush()?;
            flush_file_buffers(file)?;
        }
        self.file.take();
        let actual = inspect_wav(&self.path, format)?;
        Ok(FlushedArtifact {
            commit_id: self.commit_id,
            staging_path: self.path,
            format,
            byte_size: actual.byte_size,
            content_hash: actual.content_hash,
            duration_ms: actual.duration_ms,
        })
    }

    fn checkpoint(mut self) -> CommitResult<Option<FlushedArtifact>> {
        if self.data_bytes == 0 {
            {
                let file = self.file_mut()?;
                file.flush()?;
                flush_file_buffers(file)?;
            }
            self.file.take();
            return Ok(None);
        }
        self.finish().map(Some)
    }

    fn file_mut(&mut self) -> CommitResult<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| SessionCommitError::Conflict("PCM writer already finalized".into()))
    }
}

#[derive(Debug, Clone)]
struct FlushedArtifact {
    commit_id: String,
    staging_path: PathBuf,
    format: PcmFormat,
    byte_size: u64,
    content_hash: String,
    duration_ms: u64,
}
#[derive(Debug, Clone)]
struct InspectedWav {
    byte_size: u64,
    content_hash: String,
    duration_ms: u64,
}
#[derive(Debug, Clone)]
struct CommitIntent {
    plan: CaptureCommitPlan,
    artifact_state: String,
    storage_key: Option<String>,
    byte_size: Option<u64>,
    content_hash: Option<String>,
    checkpoint_state: String,
}
trait CommitFaultInjector {
    fn hit(&mut self, point: CommitCheckpoint) -> CommitResult<()>;
}
struct NoCommitFaults;
impl CommitFaultInjector for NoCommitFaults {
    fn hit(&mut self, _: CommitCheckpoint) -> CommitResult<()> {
        Ok(())
    }
}
pub struct SessionCommitCoordinator {
    database: Database,
    audio: ManagedAudioStore,
}

impl SessionCommitCoordinator {
    pub fn open(
        database_path: impl AsRef<Path>,
        audio_root: impl AsRef<Path>,
    ) -> CommitResult<Self> {
        Ok(Self {
            database: Database::open(database_path)?,
            audio: ManagedAudioStore::open(audio_root)?,
        })
    }

    pub fn prepare_pcm_writer(&mut self, plan: &CaptureCommitPlan) -> CommitResult<PcmPartWriter> {
        self.prepare_pcm_writer_with_disk(plan, &SystemCaptureDiskSpace)
    }

    pub fn prepare_pcm_writer_with_disk(
        &mut self,
        plan: &CaptureCommitPlan,
        disk: &dyn CaptureDiskSpace,
    ) -> CommitResult<PcmPartWriter> {
        plan.validate()?;
        if self.existing_receipt(&plan.commit_id)?.is_some() {
            return Err(SessionCommitError::Conflict(
                "commit is already finalized".into(),
            ));
        }
        check_capture_disk(self.audio.root(), plan.reserved_byte_size, disk)?;
        self.prepare_capture(plan)?;
        self.audio
            .create_writer(&plan.commit_id, plan.reserved_byte_size, plan.format)
    }

    pub fn active_runtime_profile_id(&self) -> CommitResult<Option<String>> {
        let selected: Option<Option<String>> = self
            .database
            .connection
            .query_row(
                "SELECT active_runtime_profile_id FROM app_configuration WHERE is_active=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(selected.flatten())
    }

    pub fn finalize_pcm_writer(
        &mut self,
        plan: &CaptureCommitPlan,
        writer: PcmPartWriter,
    ) -> CommitResult<CommitReceipt> {
        plan.validate()?;
        if writer.commit_id != plan.commit_id || writer.format != plan.format {
            return Err(SessionCommitError::Conflict(
                "PCM writer does not belong to the immutable commit plan".into(),
            ));
        }
        self.mark_finalizing(plan)?;
        let flushed = writer.finish()?;
        self.audio.promote(&flushed)?;
        self.database.connection.execute(
            "UPDATE audio_commit_intent SET checkpoint_state='file_promoted',updated_at=?2
             WHERE commit_id=?1 AND checkpoint_state<>'committed'",
            params![plan.commit_id, plan.finalized_at],
        )?;
        self.commit_final_marker(plan, &flushed, &mut NoCommitFaults)
    }

    pub fn recover_pcm_writer(
        &mut self,
        plan: &CaptureCommitPlan,
        writer: PcmPartWriter,
        observed_at: i64,
        reason: &str,
    ) -> CommitResult<RecoveryReceipt> {
        plan.validate()?;
        if observed_at < plan.started_at {
            return Err(SessionCommitError::InvalidInput(
                "recovery timestamp must be >= capture start".into(),
            ));
        }
        if reason.is_empty()
            || reason.len() > 64
            || !reason
                .bytes()
                .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'_')
        {
            return Err(SessionCommitError::InvalidInput(
                "recovery reason must be 1..64 lowercase ASCII letters, digits or underscores"
                    .into(),
            ));
        }
        if writer.commit_id != plan.commit_id || writer.format != plan.format {
            return Err(SessionCommitError::Conflict(
                "PCM writer does not belong to the immutable commit plan".into(),
            ));
        }

        let artifact = writer.checkpoint()?;
        let previous_state: String = self.database.connection.query_row(
            "SELECT pipeline_state FROM dictation_session WHERE id=?1",
            [plan.session_id.as_str()],
            |row| row.get(0),
        )?;
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let checkpoint_byte_size = artifact
            .as_ref()
            .map_or(WAV_HEADER_BYTES, |value| value.byte_size);
        let checkpoint_reserved = i64::try_from(checkpoint_byte_size).map_err(|_| {
            SessionCommitError::InvalidInput("checkpoint byte size exceeds SQLite range".into())
        })?;
        tx.execute(
            "UPDATE audio_artifact SET reserved_byte_size=?2
             WHERE id=?1 AND artifact_state='writing'",
            params![plan.artifact_id, checkpoint_reserved],
        )?;
        let sequence_no: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence_no),0)+1 FROM session_event WHERE session_id=?1",
            [plan.session_id.as_str()],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE dictation_session SET pipeline_state='recovery',state_version=state_version+1,
                    outcome='uncertain',last_error_code=?2,finalized_at=?3,updated_at=?3
             WHERE id=?1 AND pipeline_state IN ('recording','finalizing','recovery')",
            params![plan.session_id, reason, observed_at],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO session_event(
                id,session_id,sequence_no,event_type,from_state,to_state,source,reason_code,metadata,occurred_at
             ) VALUES(?1,?2,?3,'capture_recovery',?4,'recovery','system',?5,?6,?7)",
            params![
                format!("event-recovery-{}", plan.commit_id),
                plan.session_id,
                sequence_no,
                previous_state,
                reason,
                commit_metadata(&plan.commit_id),
                observed_at
            ],
        )?;
        tx.execute(
            "UPDATE audio_commit_intent SET checkpoint_state='recovery',updated_at=?2
             WHERE commit_id=?1 AND checkpoint_state<>'committed'",
            params![plan.commit_id, observed_at],
        )?;
        tx.commit()?;

        Ok(RecoveryReceipt {
            commit_id: plan.commit_id.clone(),
            session_id: plan.session_id.clone(),
            staging_storage_key: ManagedAudioStore::staging_key(&plan.commit_id),
            byte_size: artifact
                .as_ref()
                .map_or(WAV_HEADER_BYTES, |value| value.byte_size),
            content_hash: artifact.as_ref().map(|value| value.content_hash.clone()),
            duration_ms: artifact.as_ref().map(|value| value.duration_ms),
            reason: reason.to_owned(),
        })
    }

    pub fn cancel_pcm_writer(
        &mut self,
        plan: &CaptureCommitPlan,
        writer: PcmPartWriter,
        observed_at: i64,
    ) -> CommitResult<()> {
        plan.validate()?;
        if observed_at < plan.started_at {
            return Err(SessionCommitError::InvalidInput(
                "cancellation timestamp must be >= capture start".into(),
            ));
        }
        if writer.commit_id != plan.commit_id || writer.format != plan.format {
            return Err(SessionCommitError::Conflict(
                "PCM writer does not belong to the immutable commit plan".into(),
            ));
        }

        let previous_state: String = self.database.connection.query_row(
            "SELECT pipeline_state FROM dictation_session WHERE id=?1",
            [plan.session_id.as_str()],
            |row| row.get(0),
        )?;
        let artifact = writer.checkpoint()?;
        let staging_path = artifact.map_or_else(
            || {
                self.audio
                    .path_for_key(&ManagedAudioStore::staging_key(&plan.commit_id))
            },
            |value| Ok(value.staging_path),
        )?;
        if let Err(error) = fs::remove_file(&staging_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.into());
        }

        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence_no: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence_no),0)+1 FROM session_event WHERE session_id=?1",
            [plan.session_id.as_str()],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE dictation_session SET pipeline_state='cancelled',
                    state_version=state_version+1,outcome='cancelled',last_error_code='cancelled',
                    finalized_at=?2,updated_at=?2
             WHERE id=?1 AND pipeline_state IN ('recording','finalizing','recovery')",
            params![plan.session_id, observed_at],
        )?;
        tx.execute(
            "UPDATE audio_artifact SET artifact_state='deleted',reserved_byte_size=0,
                    storage_key=NULL,byte_size=NULL,content_hash=NULL
             WHERE id=?1 AND artifact_state='writing'",
            [plan.artifact_id.as_str()],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO session_event(
                id,session_id,sequence_no,event_type,from_state,to_state,source,reason_code,metadata,occurred_at
             ) VALUES(?1,?2,?3,'capture_cancelled',?4,'cancelled','user','cancelled',?5,?6)",
            params![
                format!("event-cancel-{}", plan.commit_id),
                plan.session_id,
                sequence_no,
                previous_state,
                commit_metadata(&plan.commit_id),
                observed_at
            ],
        )?;
        tx.execute(
            "UPDATE audio_commit_intent SET checkpoint_state='recovery',updated_at=?2
             WHERE commit_id=?1 AND checkpoint_state<>'committed'",
            params![plan.commit_id, observed_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn commit_pcm(
        &mut self,
        plan: &CaptureCommitPlan,
        samples: &[i16],
    ) -> CommitResult<CommitReceipt> {
        self.commit_pcm_with_faults(plan, samples, &mut NoCommitFaults)
    }

    pub fn reconcile_startup(
        &mut self,
        observed_at: i64,
    ) -> CommitResult<Vec<ReconciliationRecord>> {
        if observed_at < 0 {
            return Err(SessionCommitError::InvalidInput(
                "reconciliation timestamp must be non-negative".into(),
            ));
        }
        let intents = self.load_intents()?;
        let known = intents
            .iter()
            .map(|i| i.plan.commit_id.clone())
            .collect::<BTreeSet<_>>();
        let mut records = Vec::new();
        for intent in intents {
            records.push(self.reconcile_intent(intent, observed_at)?);
        }
        records.extend(self.quarantine_orphans(&known)?);
        Ok(records)
    }

    fn commit_pcm_with_faults(
        &mut self,
        plan: &CaptureCommitPlan,
        samples: &[i16],
        injector: &mut impl CommitFaultInjector,
    ) -> CommitResult<CommitReceipt> {
        plan.validate()?;
        if let Some(receipt) = self.existing_receipt(&plan.commit_id)? {
            return Ok(receipt);
        }
        check_capture_disk(
            self.audio.root(),
            plan.reserved_byte_size,
            &SystemCaptureDiskSpace,
        )?;
        self.prepare_capture(plan)?;
        injector.hit(CommitCheckpoint::AfterPrepareCommit)?;
        let mut writer =
            self.audio
                .create_writer(&plan.commit_id, plan.reserved_byte_size, plan.format)?;
        writer.write_samples(samples)?;
        injector.hit(CommitCheckpoint::AfterPartWrite)?;
        self.mark_finalizing(plan)?;
        let flushed = writer.finish()?;
        injector.hit(CommitCheckpoint::AfterArtifactFlush)?;
        self.audio.promote(&flushed)?;
        self.database.connection.execute(
            "UPDATE audio_commit_intent SET checkpoint_state='file_promoted', updated_at=?2
             WHERE commit_id=?1 AND checkpoint_state<>'committed'",
            params![plan.commit_id, plan.finalized_at],
        )?;
        injector.hit(CommitCheckpoint::AfterAtomicRename)?;
        let receipt = self.commit_final_marker(plan, &flushed, injector)?;
        injector.hit(CommitCheckpoint::AfterCheckpointCommit)?;
        Ok(receipt)
    }

    fn prepare_capture(&mut self, plan: &CaptureCommitPlan) -> CommitResult<()> {
        if let Some(existing) = self.load_intent(&plan.commit_id)? {
            if existing.plan != *plan {
                return Err(SessionCommitError::Conflict(
                    "commit_id belongs to a different immutable plan".into(),
                ));
            }
            return Ok(());
        }
        let staging_key = ManagedAudioStore::staging_key(&plan.commit_id);
        let final_key = ManagedAudioStore::final_key(&plan.commit_id);
        let reserved = i64::try_from(plan.reserved_byte_size).map_err(|_| {
            SessionCommitError::InvalidInput("reservation exceeds SQLite INTEGER".into())
        })?;
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        enforce_capture_admission(&tx, plan.reserved_byte_size)?;
        tx.execute(
            "INSERT INTO dictation_session(id,pipeline_state,state_version,started_at,created_at,updated_at)
             VALUES(?1,'recording',1,?2,?2,?2)",
            params![plan.session_id, plan.started_at],
        )?;
        tx.execute(
            "INSERT INTO audio_artifact(id,session_id,commit_id,staging_storage_key,format,reserved_byte_size,artifact_state,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,'writing',?7)",
            params![plan.artifact_id, plan.session_id, plan.commit_id, staging_key, plan.format.storage_label(), reserved, plan.started_at],
        )?;
        tx.execute(
            "INSERT INTO audio_commit_intent(
                commit_id,session_id,artifact_id,final_storage_key,runtime_profile_id,
                asr_attempt_id,asr_idempotency_key,prepare_event_id,finalizing_event_id,
                finalized_event_id,expected_finalizing_state_version,sample_rate_hz,
                channels,bits_per_sample,checkpoint_state,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,2,?11,?12,?13,'prepared',?14,?14)",
            params![
                plan.commit_id,
                plan.session_id,
                plan.artifact_id,
                final_key,
                plan.runtime_profile_id,
                plan.asr_attempt_id,
                plan.asr_idempotency_key,
                plan.prepare_event_id,
                plan.finalizing_event_id,
                plan.finalized_event_id,
                plan.format.sample_rate_hz,
                plan.format.channels,
                plan.format.bits_per_sample,
                plan.started_at
            ],
        )?;
        tx.execute(
            "INSERT INTO session_event(id,session_id,sequence_no,event_type,to_state,source,metadata,occurred_at)
             VALUES(?1,?2,1,'capture_prepared','recording','system',?3,?4)",
            params![plan.prepare_event_id, plan.session_id, commit_metadata(&plan.commit_id), plan.started_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn mark_finalizing(&mut self, plan: &CaptureCommitPlan) -> CommitResult<()> {
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: (String, i64) = tx.query_row(
            "SELECT pipeline_state,state_version FROM dictation_session WHERE id=?1",
            [plan.session_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        match current.0.as_str() {
            "recording" if current.1 == 1 => {
                if tx.execute(
                    "UPDATE dictation_session SET pipeline_state='finalizing',state_version=2,updated_at=?2
                     WHERE id=?1 AND pipeline_state='recording' AND state_version=1",
                    params![plan.session_id, plan.finalized_at],
                )? != 1 { return Err(SessionCommitError::Conflict("recording->finalizing CAS failed".into())); }
                tx.execute(
                    "INSERT INTO session_event(id,session_id,sequence_no,event_type,from_state,to_state,source,metadata,occurred_at)
                     VALUES(?1,?2,2,'capture_stopped','recording','finalizing','system',?3,?4)",
                    params![plan.finalizing_event_id, plan.session_id, commit_metadata(&plan.commit_id), plan.finalized_at],
                )?;
                tx.execute(
                    "UPDATE audio_commit_intent SET checkpoint_state='finalizing',updated_at=?2 WHERE commit_id=?1",
                    params![plan.commit_id, plan.finalized_at],
                )?;
            }
            "finalizing" | "processing" => {}
            state => {
                return Err(SessionCommitError::Conflict(format!(
                    "session cannot finalize from {state}/{}",
                    current.1
                )));
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn commit_final_marker(
        &mut self,
        plan: &CaptureCommitPlan,
        artifact: &FlushedArtifact,
        injector: &mut impl CommitFaultInjector,
    ) -> CommitResult<CommitReceipt> {
        if let Some(receipt) = self.existing_receipt(&plan.commit_id)? {
            return Ok(receipt);
        }
        let storage_key = ManagedAudioStore::final_key(&plan.commit_id);
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let duration = i64::try_from(artifact.duration_ms)
            .map_err(|_| SessionCommitError::InvalidInput("duration exceeds i64".into()))?;
        let bytes = i64::try_from(artifact.byte_size)
            .map_err(|_| SessionCommitError::InvalidInput("size exceeds i64".into()))?;
        if tx.execute(
            "UPDATE audio_artifact SET storage_key=?2,duration_ms=?3,byte_size=?4,content_hash=?5,
                    artifact_state='finalized',last_verified_at=?6
             WHERE id=?1 AND commit_id=?7 AND artifact_state='writing'",
            params![
                plan.artifact_id,
                storage_key,
                duration,
                bytes,
                artifact.content_hash,
                plan.finalized_at,
                plan.commit_id
            ],
        )? != 1
        {
            return Err(SessionCommitError::Conflict(
                "audio final-marker CAS failed".into(),
            ));
        }
        if tx.execute(
            "UPDATE dictation_session SET pipeline_state='processing',state_version=3,finalized_at=?2,updated_at=?2
             WHERE id=?1 AND pipeline_state='finalizing' AND state_version=2",
            params![plan.session_id, plan.finalized_at],
        )? != 1 { return Err(SessionCommitError::Conflict("session finalizing->processing CAS failed".into())); }
        tx.execute(
            "INSERT INTO session_event(id,session_id,sequence_no,event_type,from_state,to_state,source,metadata,occurred_at)
             VALUES(?1,?2,3,'audio_finalized','finalizing','processing','system',?3,?4)",
            params![plan.finalized_event_id, plan.session_id, commit_metadata(&plan.commit_id), plan.finalized_at],
        )?;
        tx.execute(
            "INSERT INTO asr_attempt(id,session_id,audio_artifact_id,runtime_profile_id,attempt_no,idempotency_key,status,queued_at)
             VALUES(?1,?2,?3,?4,1,?5,'queued',?6)",
            params![plan.asr_attempt_id, plan.session_id, plan.artifact_id, plan.runtime_profile_id, plan.asr_idempotency_key, plan.finalized_at],
        )?;
        tx.execute(
            "UPDATE audio_commit_intent SET checkpoint_state='committed',updated_at=?2 WHERE commit_id=?1",
            params![plan.commit_id, plan.finalized_at],
        )?;
        injector.hit(CommitCheckpoint::DuringCheckpointCommit)?;
        tx.commit()?;
        Ok(CommitReceipt {
            commit_id: plan.commit_id.clone(),
            session_id: plan.session_id.clone(),
            artifact_id: plan.artifact_id.clone(),
            asr_attempt_id: plan.asr_attempt_id.clone(),
            storage_key,
            byte_size: artifact.byte_size,
            content_hash: artifact.content_hash.clone(),
            duration_ms: artifact.duration_ms,
        })
    }

    fn existing_receipt(&self, commit_id: &str) -> CommitResult<Option<CommitReceipt>> {
        Ok(self.database.connection.query_row(
            "SELECT i.commit_id,i.session_id,i.artifact_id,i.asr_attempt_id,a.storage_key,a.byte_size,a.content_hash,a.duration_ms
             FROM audio_commit_intent i JOIN audio_artifact a ON a.id=i.artifact_id
             JOIN asr_attempt x ON x.id=i.asr_attempt_id
             WHERE i.commit_id=?1 AND i.checkpoint_state='committed' AND a.artifact_state='finalized'",
            [commit_id],
            |row| {
                let byte_size: i64 = row.get(5)?;
                let duration_ms: i64 = row.get(7)?;
                Ok(CommitReceipt {
                    commit_id: row.get(0)?,
                    session_id: row.get(1)?,
                    artifact_id: row.get(2)?,
                    asr_attempt_id: row.get(3)?,
                    storage_key: row.get(4)?,
                    byte_size: byte_size as u64,
                    content_hash: row.get(6)?,
                    duration_ms: duration_ms as u64,
                })
            },
        ).optional()?)
    }

    fn load_intents(&self) -> CommitResult<Vec<CommitIntent>> {
        let mut statement = self.database.connection.prepare(
            "SELECT i.session_id,i.artifact_id,i.commit_id,i.prepare_event_id,i.finalizing_event_id,
                    i.finalized_event_id,i.runtime_profile_id,i.asr_attempt_id,i.asr_idempotency_key,
                    s.started_at,COALESCE(s.finalized_at,i.updated_at),a.reserved_byte_size,
                    i.sample_rate_hz,i.channels,i.bits_per_sample,a.artifact_state,
                    a.storage_key,a.byte_size,a.content_hash,i.checkpoint_state
             FROM audio_commit_intent i
             JOIN audio_artifact a ON a.id=i.artifact_id AND a.session_id=i.session_id
             JOIN dictation_session s ON s.id=i.session_id
             ORDER BY i.created_at,i.commit_id",
        )?;
        Ok(statement
            .query_map([], |row| {
                let reserved: i64 = row.get(11)?;
                Ok(CommitIntent {
                    plan: CaptureCommitPlan {
                        session_id: row.get(0)?,
                        artifact_id: row.get(1)?,
                        commit_id: row.get(2)?,
                        prepare_event_id: row.get(3)?,
                        finalizing_event_id: row.get(4)?,
                        finalized_event_id: row.get(5)?,
                        runtime_profile_id: row.get(6)?,
                        asr_attempt_id: row.get(7)?,
                        asr_idempotency_key: row.get(8)?,
                        started_at: row.get(9)?,
                        finalized_at: row.get(10)?,
                        reserved_byte_size: reserved as u64,
                        format: PcmFormat {
                            sample_rate_hz: row.get(12)?,
                            channels: row.get(13)?,
                            bits_per_sample: row.get(14)?,
                        },
                    },
                    artifact_state: row.get(15)?,
                    storage_key: row.get(16)?,
                    byte_size: row.get::<_, Option<i64>>(17)?.map(|value| value as u64),
                    content_hash: row.get(18)?,
                    checkpoint_state: row.get(19)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn load_intent(&self, commit_id: &str) -> CommitResult<Option<CommitIntent>> {
        Ok(self
            .load_intents()?
            .into_iter()
            .find(|i| i.plan.commit_id == commit_id))
    }

    fn reconcile_intent(
        &mut self,
        intent: CommitIntent,
        observed_at: i64,
    ) -> CommitResult<ReconciliationRecord> {
        let plan = intent.plan;
        let expected_key = ManagedAudioStore::final_key(&plan.commit_id);
        if intent.checkpoint_state == "recovery" {
            self.set_recovery(&plan.commit_id, observed_at)?;
            return Ok(record(
                &plan.commit_id,
                ReconciliationDisposition::Recovery,
                "explicit capture recovery preserved",
            ));
        }
        let final_path = self.audio.path_for_key(&expected_key)?;
        let staging_path = self
            .audio
            .path_for_key(&ManagedAudioStore::staging_key(&plan.commit_id))?;
        if intent.artifact_state == "corrupt" {
            return Ok(record(
                &plan.commit_id,
                ReconciliationDisposition::Corrupt,
                "artifact already marked corrupt",
            ));
        }
        if intent.artifact_state == "finalized" {
            return match inspect_wav(&final_path, plan.format) {
                Ok(actual)
                    if Some(actual.byte_size) == intent.byte_size
                        && Some(actual.content_hash.as_str()) == intent.content_hash.as_deref()
                        && intent.storage_key.as_deref() == Some(expected_key.as_str())
                        && self.asr_attempt_count(&plan.commit_id)? == 1 =>
                {
                    Ok(record(
                        &plan.commit_id,
                        ReconciliationDisposition::Continue,
                        "final file, DB marker and one ASR attempt agree",
                    ))
                }
                _ => {
                    self.mark_corrupt(&plan, observed_at, "finalized_artifact_mismatch")?;
                    Ok(record(
                        &plan.commit_id,
                        ReconciliationDisposition::Corrupt,
                        "final artifact missing/changed or ASR identity invalid",
                    ))
                }
            };
        }
        if final_path.is_file() {
            return match inspect_wav(&final_path, plan.format) {
                Ok(actual) => {
                    self.mark_finalizing(&plan)?;
                    let flushed = FlushedArtifact {
                        commit_id: plan.commit_id.clone(),
                        staging_path,
                        format: plan.format,
                        byte_size: actual.byte_size,
                        content_hash: actual.content_hash,
                        duration_ms: actual.duration_ms,
                    };
                    self.commit_final_marker(&plan, &flushed, &mut NoCommitFaults)?;
                    Ok(record(
                        &plan.commit_id,
                        ReconciliationDisposition::Continue,
                        "durable final file completed DB marker",
                    ))
                }
                Err(_) => {
                    self.mark_corrupt(&plan, observed_at, "promoted_artifact_invalid")?;
                    Ok(record(
                        &plan.commit_id,
                        ReconciliationDisposition::Corrupt,
                        "promoted final file is invalid",
                    ))
                }
            };
        }
        if staging_path.is_file() {
            return match inspect_wav(&staging_path, plan.format) {
                Ok(actual) => {
                    flush_path(&staging_path)?;
                    self.mark_finalizing(&plan)?;
                    let flushed = FlushedArtifact {
                        commit_id: plan.commit_id.clone(),
                        staging_path,
                        format: plan.format,
                        byte_size: actual.byte_size,
                        content_hash: actual.content_hash,
                        duration_ms: actual.duration_ms,
                    };
                    self.audio.promote(&flushed)?;
                    self.database.connection.execute(
                        "UPDATE audio_commit_intent SET checkpoint_state='file_promoted',updated_at=?2 WHERE commit_id=?1",
                        params![plan.commit_id, observed_at],
                    )?;
                    self.commit_final_marker(&plan, &flushed, &mut NoCommitFaults)?;
                    Ok(record(
                        &plan.commit_id,
                        ReconciliationDisposition::Continue,
                        "complete staging WAV promoted and committed",
                    ))
                }
                Err(_) => {
                    self.set_recovery(&plan.commit_id, observed_at)?;
                    Ok(record(
                        &plan.commit_id,
                        ReconciliationDisposition::Recovery,
                        "incomplete staging file preserved",
                    ))
                }
            };
        }
        self.set_recovery(&plan.commit_id, observed_at)?;
        Ok(record(
            &plan.commit_id,
            ReconciliationDisposition::Recovery,
            "prepared intent has no file and was preserved",
        ))
    }

    fn set_recovery(&mut self, commit_id: &str, observed_at: i64) -> CommitResult<()> {
        let staging_path = self
            .audio
            .path_for_key(&ManagedAudioStore::staging_key(commit_id))?;
        let checkpoint_bytes = fs::metadata(&staging_path)
            .map(|metadata| metadata.len().max(WAV_HEADER_BYTES))
            .unwrap_or(WAV_HEADER_BYTES);
        let checkpoint_reserved = i64::try_from(checkpoint_bytes).map_err(|_| {
            SessionCommitError::InvalidInput("checkpoint byte size exceeds SQLite range".into())
        })?;
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (session_id, artifact_id, previous_state): (String, String, String) = tx.query_row(
            "SELECT i.session_id,i.artifact_id,s.pipeline_state
             FROM audio_commit_intent i JOIN dictation_session s ON s.id=i.session_id
             WHERE i.commit_id=?1",
            [commit_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let sequence_no: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence_no),0)+1 FROM session_event WHERE session_id=?1",
            [session_id.as_str()],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE audio_artifact SET reserved_byte_size=?2
             WHERE id=?1 AND artifact_state='writing'",
            params![artifact_id, checkpoint_reserved],
        )?;
        tx.execute(
            "UPDATE dictation_session SET pipeline_state='recovery',
                    state_version=state_version+CASE WHEN pipeline_state='recovery' THEN 0 ELSE 1 END,
                    outcome=COALESCE(outcome,'uncertain'),
                    last_error_code=COALESCE(last_error_code,'startup_reconciliation'),
                    finalized_at=COALESCE(finalized_at,?2),updated_at=?2
             WHERE id=?1 AND pipeline_state IN ('recording','finalizing','recovery')",
            params![session_id, observed_at],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO session_event(
                id,session_id,sequence_no,event_type,from_state,to_state,source,reason_code,metadata,occurred_at
             ) VALUES(?1,?2,?3,'capture_recovery',?4,'recovery','system',
                      'startup_reconciliation',?5,?6)",
            params![
                format!("event-recovery-{commit_id}"),
                session_id,
                sequence_no,
                previous_state,
                commit_metadata(commit_id),
                observed_at
            ],
        )?;
        tx.execute(
            "UPDATE audio_commit_intent SET checkpoint_state='recovery',updated_at=?2
             WHERE commit_id=?1 AND checkpoint_state<>'committed'",
            params![commit_id, observed_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn mark_corrupt(
        &mut self,
        plan: &CaptureCommitPlan,
        observed_at: i64,
        reason: &str,
    ) -> CommitResult<()> {
        let tx = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE audio_artifact SET artifact_state='corrupt',last_verified_at=?2 WHERE id=?1",
            params![plan.artifact_id, observed_at],
        )?;
        let previous_version: i64 = tx.query_row(
            "SELECT state_version FROM dictation_session WHERE id=?1",
            [plan.session_id.as_str()],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE dictation_session SET pipeline_state='recovery',state_version=state_version+1,
                    outcome='uncertain',last_error_code=?2,updated_at=?3 WHERE id=?1",
            params![plan.session_id, reason, observed_at],
        )?;
        let sequence_no: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence_no),0)+1 FROM session_event WHERE session_id=?1",
            [plan.session_id.as_str()],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO session_event(id,session_id,sequence_no,event_type,to_state,source,reason_code,metadata,occurred_at)
             VALUES(?1,?2,?3,'artifact_corrupt','recovery','system',?4,?5,?6)",
            params![format!("reconcile-corrupt-{}", plan.commit_id), plan.session_id, sequence_no, reason, commit_metadata(&plan.commit_id), observed_at],
        )?;
        tx.execute(
            "UPDATE asr_attempt SET status='cancelled',started_at=COALESCE(started_at,?2),completed_at=?2,error_code='artifact_corrupt'
             WHERE id=?1 AND status='queued'",
            params![plan.asr_attempt_id, observed_at],
        )?;
        tx.execute("UPDATE audio_commit_intent SET checkpoint_state='corrupt',updated_at=?2 WHERE commit_id=?1", params![plan.commit_id, observed_at])?;
        let new_version: i64 = tx.query_row(
            "SELECT state_version FROM dictation_session WHERE id=?1",
            [plan.session_id.as_str()],
            |row| row.get(0),
        )?;
        if new_version != previous_version + 1 {
            return Err(SessionCommitError::Conflict(
                "corrupt transition did not advance state_version once".into(),
            ));
        }
        tx.commit()?;
        Ok(())
    }

    fn asr_attempt_count(&self, commit_id: &str) -> CommitResult<u32> {
        Ok(self.database.connection.query_row(
            "SELECT COUNT(*) FROM asr_attempt a JOIN audio_commit_intent i ON i.asr_attempt_id=a.id WHERE i.commit_id=?1",
            [commit_id], |row| row.get(0),
        )?)
    }

    fn quarantine_orphans(
        &self,
        known: &BTreeSet<String>,
    ) -> CommitResult<Vec<ReconciliationRecord>> {
        let mut records = Vec::new();
        for (directory, suffix, kind) in [
            (&self.audio.staging, ".wav.part", "staging"),
            (&self.audio.finalized, ".wav", "final"),
        ] {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(commit_id) = name.strip_suffix(suffix) else {
                    continue;
                };
                if validate_commit_id(commit_id).is_err() || known.contains(commit_id) {
                    continue;
                }
                let destination = self
                    .audio
                    .quarantine
                    .join(format!("{commit_id}.{kind}.orphan"));
                write_through_move_no_replace(&entry.path(), &destination)?;
                records.push(record(
                    commit_id,
                    ReconciliationDisposition::Recovery,
                    "orphan moved to quarantine without deletion",
                ));
            }
        }
        Ok(records)
    }
}

fn record(
    commit_id: &str,
    disposition: ReconciliationDisposition,
    detail: &str,
) -> ReconciliationRecord {
    ReconciliationRecord {
        commit_id: commit_id.into(),
        disposition,
        detail: detail.into(),
    }
}

fn validate_commit_id(commit_id: &str) -> CommitResult<()> {
    if commit_id.is_empty()
        || commit_id.len() > 128
        || !commit_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(SessionCommitError::InvalidInput(
            "commit_id must be 1..=128 safe ASCII bytes".into(),
        ));
    }
    Ok(())
}

fn commit_id_from_path(path: &Path, suffix: &str) -> CommitResult<String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| SessionCommitError::InvalidInput("non-UTF-8 artifact name".into()))?;
    let commit_id = name.strip_suffix(suffix).ok_or_else(|| {
        SessionCommitError::InvalidInput(format!("artifact name must end with {suffix}"))
    })?;
    validate_commit_id(commit_id)?;
    Ok(commit_id.into())
}

fn commit_metadata(commit_id: &str) -> String {
    format!(r#"{{"commit_id":"{commit_id}"}}"#)
}

fn wav_header(format: PcmFormat, data_bytes: u32) -> CommitResult<[u8; WAV_HEADER_BYTES as usize]> {
    format.validate()?;
    let riff_size = 36_u32
        .checked_add(data_bytes)
        .ok_or_else(|| SessionCommitError::InvalidInput("RIFF size overflow".into()))?;
    let block_align = format.block_align();
    let byte_rate = format
        .sample_rate_hz
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| SessionCommitError::InvalidInput("WAV byte-rate overflow".into()))?;
    let mut header = [0_u8; WAV_HEADER_BYTES as usize];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&format.channels.to_le_bytes());
    header[24..28].copy_from_slice(&format.sample_rate_hz.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&format.bits_per_sample.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    Ok(header)
}

fn inspect_wav(path: &Path, expected: PcmFormat) -> CommitResult<InspectedWav> {
    expected.validate()?;
    let mut file = File::open(path)?;
    let byte_size = file.metadata()?.len();
    if byte_size < WAV_HEADER_BYTES {
        return Err(SessionCommitError::Conflict(
            "WAV shorter than header".into(),
        ));
    }
    let mut riff = [0_u8; 12];
    file.read_exact(&mut riff)?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(SessionCommitError::Conflict("not RIFF/WAVE".into()));
    }
    let declared = u64::from(u32::from_le_bytes(riff[4..8].try_into().unwrap())) + 8;
    if declared != byte_size {
        return Err(SessionCommitError::Conflict("RIFF length mismatch".into()));
    }
    let mut position = 12_u64;
    let mut found_format = None;
    let mut data_bytes = None;
    while position + 8 <= byte_size {
        file.seek(SeekFrom::Start(position))?;
        let mut chunk = [0_u8; 8];
        file.read_exact(&mut chunk)?;
        let chunk_size = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
        let payload_start = position + 8;
        let payload_end = payload_start
            .checked_add(chunk_size)
            .ok_or_else(|| SessionCommitError::Conflict("RIFF chunk overflow".into()))?;
        if payload_end > byte_size {
            return Err(SessionCommitError::Conflict(
                "RIFF chunk past file end".into(),
            ));
        }
        match &chunk[0..4] {
            b"fmt " => {
                if chunk_size < 16 {
                    return Err(SessionCommitError::Conflict("short fmt chunk".into()));
                }
                let mut bytes = [0_u8; 16];
                file.read_exact(&mut bytes)?;
                if u16::from_le_bytes(bytes[0..2].try_into().unwrap()) != 1 {
                    return Err(SessionCommitError::Conflict(
                        "WAV is not integer PCM".into(),
                    ));
                }
                found_format = Some(PcmFormat {
                    channels: u16::from_le_bytes(bytes[2..4].try_into().unwrap()),
                    sample_rate_hz: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                    bits_per_sample: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
                });
            }
            b"data" if data_bytes.is_some() => {
                return Err(SessionCommitError::Conflict(
                    "multiple WAV data chunks".into(),
                ));
            }
            b"data" => data_bytes = Some(chunk_size),
            _ => {}
        }
        position = payload_end + (chunk_size & 1);
    }
    if position != byte_size || found_format != Some(expected) {
        return Err(SessionCommitError::Conflict(
            "WAV layout/format mismatch".into(),
        ));
    }
    let data_bytes =
        data_bytes.ok_or_else(|| SessionCommitError::Conflict("missing WAV data chunk".into()))?;
    if data_bytes == 0 || !data_bytes.is_multiple_of(u64::from(expected.block_align())) {
        return Err(SessionCommitError::Conflict(
            "empty or unaligned WAV data".into(),
        ));
    }
    let frames = data_bytes / u64::from(expected.block_align());
    let duration_ms = frames
        .checked_mul(1000)
        .and_then(|v| v.checked_div(u64::from(expected.sample_rate_hz)))
        .ok_or_else(|| SessionCommitError::Conflict("WAV duration overflow".into()))?;
    Ok(InspectedWav {
        byte_size,
        content_hash: hash_file(path)?,
        duration_ms,
    })
}

fn hash_file(path: &Path) -> CommitResult<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn flush_path(path: &Path) -> CommitResult<()> {
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    flush_file_buffers(&file)
}

#[cfg(windows)]
fn flush_file_buffers(file: &File) -> CommitResult<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
    // SAFETY: File owns a valid HANDLE for the duration of this call.
    if unsafe { FlushFileBuffers(file.as_raw_handle().cast()) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn flush_file_buffers(file: &File) -> CommitResult<()> {
    file.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn write_through_move_no_replace(source: &Path, destination: &Path) -> CommitResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: NUL-terminated buffers live for the call. REPLACE_EXISTING is intentionally absent.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn write_through_move_no_replace(source: &Path, destination: &Path) -> CommitResult<()> {
    if destination.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "destination collision",
        )
        .into());
    }
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wigigadict_test_support::{FaultInjector as _, FaultPoint, ScriptedFaults};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct TestEnvironment {
        root: PathBuf,
        database: PathBuf,
        audio: PathBuf,
    }

    impl TestEnvironment {
        fn new(label: &str) -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "wigigadict-step6-{label}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            Self {
                database: root.join("state.sqlite3"),
                audio: root.join("managed"),
                root,
            }
        }

        fn open(&self) -> SessionCommitCoordinator {
            SessionCommitCoordinator::open(&self.database, &self.audio).unwrap()
        }
    }

    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            if self.root.exists() {
                fs::remove_dir_all(&self.root)
                    .expect("generated Step 6 temp tree must be removable");
            }
        }
    }

    struct TestFaults {
        inner: ScriptedFaults,
    }

    impl TestFaults {
        fn once(point: CommitCheckpoint) -> Self {
            Self {
                inner: ScriptedFaults::once([map_fault(point)]),
            }
        }
    }

    impl CommitFaultInjector for TestFaults {
        fn hit(&mut self, point: CommitCheckpoint) -> CommitResult<()> {
            self.inner
                .hit(map_fault(point))
                .map_err(|_| SessionCommitError::Injected(point))
        }
    }

    fn map_fault(point: CommitCheckpoint) -> FaultPoint {
        match point {
            CommitCheckpoint::AfterPrepareCommit => FaultPoint::AfterPrepareCommit,
            CommitCheckpoint::AfterPartWrite => FaultPoint::AfterPartWrite,
            CommitCheckpoint::AfterArtifactFlush => FaultPoint::AfterArtifactFlush,
            CommitCheckpoint::AfterAtomicRename => FaultPoint::AfterAtomicRename,
            CommitCheckpoint::DuringCheckpointCommit => FaultPoint::DuringCheckpointCommit,
            CommitCheckpoint::AfterCheckpointCommit => FaultPoint::AfterCheckpointCommit,
        }
    }

    fn seed_runtime(coordinator: &SessionCommitCoordinator) {
        coordinator.database.connection.execute(
            "INSERT INTO model_package(id,engine_family,model_name,model_version,source_uri,license_id,
                    expected_size,checksum_algorithm,checksum,storage_key,install_state,installed_at,created_at,updated_at)
             VALUES('model-1','whisper','large-v3-turbo-q5','1','managed:model','MIT',100,'sha256',?1,
                    'models/model.bin','installed',1,1,1)", [HASH],
        ).unwrap();
        coordinator.database.connection.execute(
            "INSERT INTO runtime_profile(id,profile_version,model_package_id,adapter_type,adapter_version,
                    device_kind,settings,settings_hash,health_state,enabled,created_at,updated_at)
             VALUES('runtime-1',1,'model-1','transcribe-rs','0.3.11','cpu','{}',?1,'healthy',1,1,1)", [HASH],
        ).unwrap();
    }

    fn plan(label: &str) -> CaptureCommitPlan {
        CaptureCommitPlan {
            session_id: format!("session-{label}"),
            artifact_id: format!("artifact-{label}"),
            commit_id: format!("commit-{label}"),
            prepare_event_id: format!("event-prepare-{label}"),
            finalizing_event_id: format!("event-finalizing-{label}"),
            finalized_event_id: format!("event-finalized-{label}"),
            runtime_profile_id: "runtime-1".into(),
            asr_attempt_id: format!("asr-{label}"),
            asr_idempotency_key: format!("asr-key-{label}"),
            started_at: 100,
            finalized_at: 200,
            reserved_byte_size: 4096,
            format: PcmFormat::MONO_16KHZ_S16,
        }
    }

    fn samples() -> Vec<i16> {
        (0..1600).map(|index| (index % 200) as i16 - 100).collect()
    }

    fn count(coordinator: &SessionCommitCoordinator, table: &str) -> u32 {
        coordinator
            .database
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn every_durable_checkpoint_reconciles_without_loss_or_duplicate_asr() {
        let cases = [
            (
                CommitCheckpoint::AfterPrepareCommit,
                ReconciliationDisposition::Recovery,
            ),
            (
                CommitCheckpoint::AfterPartWrite,
                ReconciliationDisposition::Recovery,
            ),
            (
                CommitCheckpoint::AfterArtifactFlush,
                ReconciliationDisposition::Continue,
            ),
            (
                CommitCheckpoint::AfterAtomicRename,
                ReconciliationDisposition::Continue,
            ),
            (
                CommitCheckpoint::DuringCheckpointCommit,
                ReconciliationDisposition::Continue,
            ),
            (
                CommitCheckpoint::AfterCheckpointCommit,
                ReconciliationDisposition::Continue,
            ),
        ];
        for (index, (checkpoint, expected)) in cases.into_iter().enumerate() {
            let label = format!("fault-{index}");
            let environment = TestEnvironment::new(&label);
            let mut coordinator = environment.open();
            seed_runtime(&coordinator);
            let plan = plan(&label);
            let error = coordinator
                .commit_pcm_with_faults(&plan, &samples(), &mut TestFaults::once(checkpoint))
                .unwrap_err();
            assert!(matches!(error, SessionCommitError::Injected(point) if point == checkpoint));
            drop(coordinator);

            let mut restarted = environment.open();
            let records = restarted.reconcile_startup(300).unwrap();
            let record = records
                .iter()
                .find(|record| record.commit_id == plan.commit_id)
                .unwrap();
            assert_eq!(record.disposition, expected, "checkpoint={checkpoint:?}");
            assert_eq!(count(&restarted, "dictation_session"), 1);
            assert_eq!(count(&restarted, "audio_artifact"), 1);
            assert_eq!(count(&restarted, "audio_commit_intent"), 1);
            let asr_count = count(&restarted, "asr_attempt");
            assert!(asr_count <= 1, "checkpoint={checkpoint:?}");
            if expected == ReconciliationDisposition::Continue {
                assert_eq!(asr_count, 1);
            } else {
                assert_eq!(asr_count, 0);
                let recovery: (String, i64, String) = restarted
                    .database
                    .connection
                    .query_row(
                        "SELECT pipeline_state,state_version,last_error_code
                         FROM dictation_session WHERE id=?1",
                        [plan.session_id.as_str()],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .unwrap();
                assert_eq!(
                    recovery,
                    ("recovery".into(), 2, "startup_reconciliation".into())
                );
            }
            let recoverable = restarted
                .audio
                .path_for_key(&ManagedAudioStore::staging_key(&plan.commit_id))
                .unwrap()
                .exists()
                || restarted
                    .audio
                    .path_for_key(&ManagedAudioStore::final_key(&plan.commit_id))
                    .unwrap()
                    .exists()
                || restarted
                    .database
                    .connection
                    .query_row(
                        "SELECT artifact_state='finalized' FROM audio_artifact WHERE commit_id=?1",
                        [plan.commit_id.as_str()],
                        |row| row.get::<_, bool>(0),
                    )
                    .unwrap();
            assert!(recoverable || checkpoint == CommitCheckpoint::AfterPrepareCommit);

            let before = asr_count;
            let state_version_before: i64 = restarted
                .database
                .connection
                .query_row(
                    "SELECT state_version FROM dictation_session WHERE id=?1",
                    [plan.session_id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            restarted.reconcile_startup(301).unwrap();
            assert_eq!(
                count(&restarted, "asr_attempt"),
                before,
                "second restart duplicated ASR at {checkpoint:?}"
            );
            let state_version_after: i64 = restarted
                .database
                .connection
                .query_row(
                    "SELECT state_version FROM dictation_session WHERE id=?1",
                    [plan.session_id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                state_version_after, state_version_before,
                "second restart mutated session state at {checkpoint:?}"
            );
        }
    }

    #[test]
    fn public_two_phase_writer_accepts_bounded_blocks_before_finalize() {
        let environment = TestEnvironment::new("two-phase");
        let mut coordinator = environment.open();
        seed_runtime(&coordinator);
        let plan = plan("two-phase");
        let all_samples = samples();
        let mut writer = coordinator.prepare_pcm_writer(&plan).unwrap();
        for block in all_samples.chunks(160) {
            writer.write_samples(block).unwrap();
        }
        assert_eq!(count(&coordinator, "asr_attempt"), 0);
        let receipt = coordinator.finalize_pcm_writer(&plan, writer).unwrap();
        assert_eq!(receipt.duration_ms, 100);
        assert_eq!(count(&coordinator, "asr_attempt"), 1);
    }

    #[test]
    fn successful_commit_is_idempotent_and_hash_bound() {
        let environment = TestEnvironment::new("success");
        let mut coordinator = environment.open();
        seed_runtime(&coordinator);
        let plan = plan("success");
        let first = coordinator.commit_pcm(&plan, &samples()).unwrap();
        let second = coordinator.commit_pcm(&plan, &samples()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.byte_size, WAV_HEADER_BYTES + 3200);
        assert_eq!(first.duration_ms, 100);
        assert_eq!(first.content_hash.len(), 64);
        assert_eq!(count(&coordinator, "asr_attempt"), 1);
        assert_eq!(count(&coordinator, "session_event"), 3);
        assert_eq!(
            coordinator.reconcile_startup(300).unwrap()[0].disposition,
            ReconciliationDisposition::Continue
        );
    }

    #[test]
    fn bounded_writer_rejects_overflow_without_deleting_part() {
        let environment = TestEnvironment::new("bounded");
        let store = ManagedAudioStore::open(&environment.audio).unwrap();
        let mut writer = store
            .create_writer(
                "commit-bounded",
                WAV_HEADER_BYTES + 2,
                PcmFormat::MONO_16KHZ_S16,
            )
            .unwrap();
        writer.write_samples(&[1]).unwrap();
        assert!(matches!(
            writer.write_samples(&[2]),
            Err(SessionCommitError::Conflict(_))
        ));
        assert_eq!(writer.written_file_bytes(), WAV_HEADER_BYTES + 2);
        assert!(
            store
                .path_for_key(&ManagedAudioStore::staging_key("commit-bounded"))
                .unwrap()
                .exists()
        );
    }

    #[test]
    fn explicit_recovery_flushes_fragment_and_never_queues_asr() {
        let environment = TestEnvironment::new("explicit-recovery");
        let mut coordinator = environment.open();
        seed_runtime(&coordinator);
        let plan = plan("explicit-recovery");
        let mut writer = coordinator.prepare_pcm_writer(&plan).unwrap();
        writer.write_samples(&samples()).unwrap();

        let receipt = coordinator
            .recover_pcm_writer(&plan, writer, 150, "audio_device_lost")
            .unwrap();
        assert_eq!(receipt.byte_size, WAV_HEADER_BYTES + 3200);
        assert_eq!(receipt.duration_ms, Some(100));
        assert_eq!(receipt.content_hash.as_ref().unwrap().len(), 64);
        assert_eq!(count(&coordinator, "asr_attempt"), 0);
        let reserved: i64 = coordinator
            .database
            .connection
            .query_row(
                "SELECT reserved_byte_size FROM audio_artifact WHERE id=?1",
                [plan.artifact_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reserved as u64, receipt.byte_size);
        let state: (String, String, String) = coordinator
            .database
            .connection
            .query_row(
                "SELECT s.pipeline_state,s.last_error_code,i.checkpoint_state
                 FROM dictation_session s JOIN audio_commit_intent i ON i.session_id=s.id
                 WHERE s.id=?1",
                [plan.session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            state,
            (
                "recovery".into(),
                "audio_device_lost".into(),
                "recovery".into()
            )
        );
        drop(coordinator);

        let mut restarted = environment.open();
        let records = restarted.reconcile_startup(300).unwrap();
        assert_eq!(records[0].disposition, ReconciliationDisposition::Recovery);
        assert_eq!(count(&restarted, "asr_attempt"), 0);
        let part = restarted
            .audio
            .path_for_key(&ManagedAudioStore::staging_key(&plan.commit_id))
            .unwrap();
        assert_eq!(
            inspect_wav(&part, PcmFormat::MONO_16KHZ_S16)
                .unwrap()
                .duration_ms,
            100
        );
    }

    #[test]
    fn empty_capture_is_durable_recovery_not_a_false_finalization() {
        let environment = TestEnvironment::new("empty-recovery");
        let mut coordinator = environment.open();
        seed_runtime(&coordinator);
        let plan = plan("empty-recovery");
        let writer = coordinator.prepare_pcm_writer(&plan).unwrap();

        let receipt = coordinator
            .recover_pcm_writer(&plan, writer, 101, "empty_capture")
            .unwrap();
        assert_eq!(receipt.byte_size, WAV_HEADER_BYTES);
        assert_eq!(receipt.duration_ms, None);
        assert_eq!(receipt.content_hash, None);
        assert_eq!(count(&coordinator, "asr_attempt"), 0);
        let reserved: i64 = coordinator
            .database
            .connection
            .query_row(
                "SELECT reserved_byte_size FROM audio_artifact WHERE id=?1",
                [plan.artifact_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reserved as u64, WAV_HEADER_BYTES);
        assert_eq!(
            coordinator.reconcile_startup(300).unwrap()[0].disposition,
            ReconciliationDisposition::Recovery
        );
    }

    #[test]
    fn cancellation_is_terminal_deletes_pcm_and_never_queues_asr() {
        let environment = TestEnvironment::new("cancelled-capture");
        let mut coordinator = environment.open();
        seed_runtime(&coordinator);
        let plan = plan("cancelled-capture");
        let mut writer = coordinator.prepare_pcm_writer(&plan).unwrap();
        writer.write_samples(&samples()).unwrap();
        let staging = coordinator
            .audio
            .path_for_key(&ManagedAudioStore::staging_key(&plan.commit_id))
            .unwrap();

        coordinator.cancel_pcm_writer(&plan, writer, 150).unwrap();

        assert!(!staging.exists());
        assert_eq!(count(&coordinator, "asr_attempt"), 0);
        let state: (String, String, String, i64, String) = coordinator
            .database
            .connection
            .query_row(
                "SELECT s.pipeline_state,s.outcome,a.artifact_state,a.reserved_byte_size,
                        i.checkpoint_state
                 FROM dictation_session s
                 JOIN audio_artifact a ON a.session_id=s.id
                 JOIN audio_commit_intent i ON i.session_id=s.id
                 WHERE s.id=?1",
                [plan.session_id.as_str()],
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
        assert_eq!(
            state,
            (
                "cancelled".into(),
                "cancelled".into(),
                "deleted".into(),
                0,
                "recovery".into()
            )
        );
    }

    #[test]
    fn write_through_promotion_never_overwrites_collision() {
        let environment = TestEnvironment::new("collision");
        let mut coordinator = environment.open();
        seed_runtime(&coordinator);
        let plan = plan("collision");
        coordinator.prepare_capture(&plan).unwrap();
        let mut writer = coordinator
            .audio
            .create_writer(&plan.commit_id, plan.reserved_byte_size, plan.format)
            .unwrap();
        writer.write_samples(&samples()).unwrap();
        coordinator.mark_finalizing(&plan).unwrap();
        let flushed = writer.finish().unwrap();
        let final_path = coordinator
            .audio
            .path_for_key(&ManagedAudioStore::final_key(&plan.commit_id))
            .unwrap();
        fs::write(&final_path, b"sentinel").unwrap();
        assert!(coordinator.audio.promote(&flushed).is_err());
        assert_eq!(fs::read(&final_path).unwrap(), b"sentinel");
        assert!(flushed.staging_path.exists());
    }

    #[test]
    fn changed_final_file_becomes_corrupt_and_blocks_queued_attempt() {
        let environment = TestEnvironment::new("corrupt");
        let mut coordinator = environment.open();
        seed_runtime(&coordinator);
        let plan = plan("corrupt");
        coordinator.commit_pcm(&plan, &samples()).unwrap();
        let final_path = coordinator
            .audio
            .path_for_key(&ManagedAudioStore::final_key(&plan.commit_id))
            .unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&final_path)
            .unwrap();
        file.seek(SeekFrom::End(-1)).unwrap();
        file.write_all(&[0x7f]).unwrap();
        file.flush().unwrap();
        drop(file);
        let records = coordinator.reconcile_startup(400).unwrap();
        assert_eq!(records[0].disposition, ReconciliationDisposition::Corrupt);
        let state: (String, String, String) = coordinator.database.connection.query_row(
            "SELECT a.artifact_state,s.pipeline_state,x.status FROM audio_artifact a
             JOIN dictation_session s ON s.id=a.session_id JOIN asr_attempt x ON x.audio_artifact_id=a.id
             WHERE a.commit_id=?1", [plan.commit_id.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(
            state,
            ("corrupt".into(), "recovery".into(), "cancelled".into())
        );
        assert_eq!(
            coordinator.reconcile_startup(401).unwrap()[0].disposition,
            ReconciliationDisposition::Corrupt
        );
        assert_eq!(count(&coordinator, "asr_attempt"), 1);
    }

    #[test]
    fn orphan_is_quarantined_and_never_deleted() {
        let environment = TestEnvironment::new("orphan");
        let mut coordinator = environment.open();
        let orphan = coordinator.audio.finalized.join("orphan-1.wav");
        fs::write(&orphan, b"recoverable orphan bytes").unwrap();
        let records = coordinator.reconcile_startup(500).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].commit_id, "orphan-1");
        assert_eq!(records[0].disposition, ReconciliationDisposition::Recovery);
        assert!(!orphan.exists());
        assert_eq!(
            fs::read(coordinator.audio.quarantine.join("orphan-1.final.orphan")).unwrap(),
            b"recoverable orphan bytes"
        );
    }

    #[test]
    fn commit_id_cannot_escape_managed_root() {
        let environment = TestEnvironment::new("containment");
        let store = ManagedAudioStore::open(&environment.audio).unwrap();
        assert!(matches!(
            store.create_writer("../escape", 100, PcmFormat::MONO_16KHZ_S16),
            Err(SessionCommitError::InvalidInput(_))
        ));
        assert!(!environment.root.join("escape.wav.part").exists());
    }
}
