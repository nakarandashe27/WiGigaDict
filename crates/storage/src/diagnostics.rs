//! Bounded content-free diagnostics and deterministic support bundles.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const DIAGNOSTIC_TRACE_SCHEMA_VERSION: u32 = 1;
pub const DIAGNOSTIC_BUNDLE_SCHEMA_VERSION: u32 = 1;
pub const DIAGNOSTIC_RETENTION_DAYS: i64 = 30;
pub const DIAGNOSTIC_MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
pub const DIAGNOSTIC_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
pub const DIAGNOSTIC_MAX_FILES: usize = 25;
pub const DIAGNOSTIC_MAX_EVENT_BYTES: usize = 16 * 1024;
pub const DIAGNOSTIC_BUNDLE_MAX_BYTES: usize = 100 * 1024 * 1024;
pub const DIAGNOSTIC_EXPORT_CONFIRMATION: &str = "export_content_free_diagnostics";
const ACTIVE_FILE: &str = "trace-current.ndjson";

#[derive(Debug)]
pub enum DiagnosticError {
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidInput(String),
    CorruptTrace(String),
    UnknownEntry(String),
    UnsupportedSchema { found: u32, supported: u32 },
    LimitExceeded(String),
}
impl Display for DiagnosticError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "diagnostic I/O failed: {error}"),
            Self::Json(error) => write!(f, "diagnostic JSON failed: {error}"),
            Self::InvalidInput(detail) => write!(f, "invalid diagnostic input: {detail}"),
            Self::CorruptTrace(detail) => write!(f, "diagnostic trace is corrupt: {detail}"),
            Self::UnknownEntry(entry) => {
                write!(f, "diagnostic bundle rejected unknown entry: {entry}")
            }
            Self::UnsupportedSchema { found, supported } => {
                write!(
                    f,
                    "diagnostic schema {found} is newer than supported schema {supported}"
                )
            }
            Self::LimitExceeded(detail) => write!(f, "diagnostic limit exceeded: {detail}"),
        }
    }
}
impl std::error::Error for DiagnosticError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}
impl From<std::io::Error> for DiagnosticError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for DiagnosticError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
pub type DiagnosticResult<T> = Result<T, DiagnosticError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticComponent {
    Shell,
    Capture,
    Storage,
    Asr,
    Cleanup,
    Delivery,
    Security,
    Diagnostics,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticEventName {
    ShellLifecycle,
    CaptureState,
    StorageCommit,
    StorageReconciliation,
    AsrState,
    CleanupState,
    FocusCheck,
    DeliveryState,
    SecurityRejection,
    BundlePreview,
    BundleExport,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStage {
    Startup,
    Prepare,
    Record,
    Finalize,
    Recover,
    Lease,
    Transcribe,
    Cleanup,
    TargetCheck,
    Commit,
    Retention,
    Preview,
    Export,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticOutcome {
    Started,
    Succeeded,
    Failed,
    Uncertain,
    Rejected,
    Recovered,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticActor {
    Owner,
    System,
    Windows,
    Sidecar,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DiagnosticMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_no: Option<u32>,
}
impl DiagnosticMetadata {
    fn validate(&self) -> DiagnosticResult<()> {
        for (name, value) in [
            ("from_state", self.from_state.as_deref()),
            ("to_state", self.to_state.as_deref()),
            ("missing_source", self.missing_source.as_deref()),
            ("delivery_method", self.delivery_method.as_deref()),
            ("evidence_class", self.evidence_class.as_deref()),
        ] {
            if let Some(value) = value {
                validate_token(value, name, 64)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEventInput {
    pub component: DiagnosticComponent,
    pub event_name: DiagnosticEventName,
    pub stage: DiagnosticStage,
    pub outcome: DiagnosticOutcome,
    pub actor: DiagnosticActor,
    pub occurred_at: i64,
    pub session_id: Option<String>,
    pub correlation_id: Option<String>,
    pub error_code: Option<String>,
    pub duration_ms: Option<u64>,
    pub metadata: DiagnosticMetadata,
}
impl DiagnosticEventInput {
    pub fn new(
        component: DiagnosticComponent,
        event_name: DiagnosticEventName,
        stage: DiagnosticStage,
        outcome: DiagnosticOutcome,
        occurred_at: i64,
    ) -> Self {
        Self {
            component,
            event_name,
            stage,
            outcome,
            occurred_at,
            actor: DiagnosticActor::System,
            session_id: None,
            correlation_id: None,
            error_code: None,
            duration_ms: None,
            metadata: DiagnosticMetadata::default(),
        }
    }
    fn validate(&self) -> DiagnosticResult<()> {
        if self.occurred_at < 0 {
            return Err(DiagnosticError::InvalidInput(
                "occurred_at must be non-negative".into(),
            ));
        }
        for (name, value, maximum) in [
            ("session_id", self.session_id.as_deref(), 128),
            ("correlation_id", self.correlation_id.as_deref(), 128),
            ("error_code", self.error_code.as_deref(), 64),
        ] {
            if let Some(value) = value {
                validate_token(value, name, maximum)?;
            }
        }
        self.metadata.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DiagnosticEvent {
    pub schema_version: u32,
    pub sequence: u64,
    pub process_generation: String,
    pub component: DiagnosticComponent,
    pub event_name: DiagnosticEventName,
    pub stage: DiagnosticStage,
    pub outcome: DiagnosticOutcome,
    pub actor: DiagnosticActor,
    pub occurred_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub metadata: DiagnosticMetadata,
}
impl DiagnosticEvent {
    fn validate(&self) -> DiagnosticResult<()> {
        if self.schema_version != DIAGNOSTIC_TRACE_SCHEMA_VERSION {
            return Err(DiagnosticError::UnsupportedSchema {
                found: self.schema_version,
                supported: DIAGNOSTIC_TRACE_SCHEMA_VERSION,
            });
        }
        if self.sequence == 0 {
            return Err(DiagnosticError::CorruptTrace(
                "sequence must be positive".into(),
            ));
        }
        validate_token(&self.process_generation, "process_generation", 128)?;
        DiagnosticEventInput {
            component: self.component,
            event_name: self.event_name,
            stage: self.stage,
            outcome: self.outcome,
            actor: self.actor,
            occurred_at: self.occurred_at,
            session_id: self.session_id.clone(),
            correlation_id: self.correlation_id.clone(),
            error_code: self.error_code.clone(),
            duration_ms: self.duration_ms,
            metadata: self.metadata.clone(),
        }
        .validate()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticLimits {
    pub retention_ms: i64,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    pub max_files: usize,
    pub max_event_bytes: usize,
    pub max_bundle_bytes: usize,
}
impl Default for DiagnosticLimits {
    fn default() -> Self {
        Self {
            retention_ms: DIAGNOSTIC_RETENTION_DAYS * 24 * 60 * 60 * 1000,
            max_total_bytes: DIAGNOSTIC_MAX_TOTAL_BYTES,
            max_file_bytes: DIAGNOSTIC_MAX_FILE_BYTES,
            max_files: DIAGNOSTIC_MAX_FILES,
            max_event_bytes: DIAGNOSTIC_MAX_EVENT_BYTES,
            max_bundle_bytes: DIAGNOSTIC_BUNDLE_MAX_BYTES,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticStatus {
    pub trace_schema_version: u32,
    pub retention_days: i64,
    pub maximum_bytes: u64,
    pub file_count: usize,
    pub stored_bytes: u64,
    pub event_count: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticBundlePreview {
    pub preview_id: String,
    pub bundle_schema_version: u32,
    pub event_count: usize,
    pub source_file_count: usize,
    pub byte_count: u64,
    pub first_occurred_at: Option<i64>,
    pub last_occurred_at: Option<i64>,
    pub redaction_count: u32,
    pub excluded_by_default: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceFile {
    name: String,
    byte_count: u64,
    event_count: usize,
    sha256: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    bundle_schema_version: u32,
    trace_schema_version: u32,
    created_at: i64,
    app_version: String,
    build_commit: String,
    event_count: usize,
    source_files: Vec<SourceFile>,
    excluded_by_default: Vec<String>,
    redaction_policy: String,
}
#[derive(Debug, Serialize)]
struct Bundle<'a> {
    manifest: Manifest,
    events: &'a [DiagnosticEvent],
}

pub struct PreparedDiagnosticBundle {
    preview: DiagnosticBundlePreview,
    bytes: Vec<u8>,
}
impl PreparedDiagnosticBundle {
    pub fn preview(&self) -> &DiagnosticBundlePreview {
        &self.preview
    }
    pub fn export(
        &self,
        preview_id: &str,
        destination: &Path,
        confirmation: &str,
    ) -> DiagnosticResult<String> {
        if preview_id != self.preview.preview_id {
            return Err(DiagnosticError::InvalidInput(
                "preview id does not match".into(),
            ));
        }
        if confirmation != DIAGNOSTIC_EXPORT_CONFIRMATION {
            return Err(DiagnosticError::InvalidInput(
                "explicit confirmation is missing".into(),
            ));
        }
        validate_export_destination(destination)?;
        let name = destination
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| DiagnosticError::InvalidInput("export filename is invalid".into()))?
            .to_owned();
        let part = destination.with_file_name(format!("{name}.part"));
        if part.exists() {
            return Err(DiagnosticError::InvalidInput(
                "export staging file exists".into(),
            ));
        }
        let result = (|| -> DiagnosticResult<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part)?;
            file.write_all(&self.bytes)?;
            file.flush()?;
            file.sync_all()?;
            fs::rename(&part, destination)?;
            Ok(())
        })();
        if result.is_err() && part.is_file() {
            let _ = fs::remove_file(&part);
        }
        result?;
        Ok(name)
    }
}

pub struct DiagnosticLogStore {
    root: PathBuf,
    process_generation: String,
    next_sequence: u64,
    active_first: Option<u64>,
    active_last: Option<u64>,
    active_bytes: u64,
    limits: DiagnosticLimits,
}
impl DiagnosticLogStore {
    pub fn open(root: impl AsRef<Path>, generation: &str, now: i64) -> DiagnosticResult<Self> {
        Self::open_with_limits(root, generation, now, DiagnosticLimits::default())
    }
    pub fn open_with_limits(
        root: impl AsRef<Path>,
        generation: &str,
        now: i64,
        limits: DiagnosticLimits,
    ) -> DiagnosticResult<Self> {
        validate_token(generation, "process_generation", 128)?;
        validate_limits(limits)?;
        if now < 0 {
            return Err(DiagnosticError::InvalidInput("timestamp is invalid".into()));
        }
        let root = root.as_ref().to_owned();
        reject_symlink(&root)?;
        fs::create_dir_all(&root)?;
        reject_symlink(&root)?;
        let mut store = Self {
            root,
            process_generation: generation.to_owned(),
            next_sequence: 1,
            active_first: None,
            active_last: None,
            active_bytes: 0,
            limits,
        };
        store.recover_and_index()?;
        store.enforce_retention(now)?;
        Ok(store)
    }
    pub fn append(&mut self, input: DiagnosticEventInput) -> DiagnosticResult<DiagnosticEvent> {
        input.validate()?;
        let event = DiagnosticEvent {
            schema_version: DIAGNOSTIC_TRACE_SCHEMA_VERSION,
            sequence: self.next_sequence,
            process_generation: self.process_generation.clone(),
            component: input.component,
            event_name: input.event_name,
            stage: input.stage,
            outcome: input.outcome,
            actor: input.actor,
            occurred_at: input.occurred_at,
            session_id: input.session_id,
            correlation_id: input.correlation_id,
            error_code: input.error_code,
            duration_ms: input.duration_ms,
            metadata: input.metadata,
        };
        event.validate()?;
        let mut line = serde_json::to_vec(&event)?;
        line.push(b'\n');
        if line.len() > self.limits.max_event_bytes
            || line.len() as u64 > self.limits.max_file_bytes
        {
            return Err(DiagnosticError::LimitExceeded("event byte cap".into()));
        }
        if self.active_bytes > 0
            && self.active_bytes.saturating_add(line.len() as u64) > self.limits.max_file_bytes
        {
            self.rotate()?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join(ACTIVE_FILE))?;
        file.write_all(&line)?;
        file.flush()?;
        file.sync_data()?;
        self.active_first.get_or_insert(event.sequence);
        self.active_last = Some(event.sequence);
        self.active_bytes = self.active_bytes.saturating_add(line.len() as u64);
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| DiagnosticError::LimitExceeded("sequence exhausted".into()))?;
        self.enforce_retention(event.occurred_at)?;
        Ok(event)
    }
    pub fn status(&self) -> DiagnosticResult<DiagnosticStatus> {
        let snapshot = self.snapshot()?;
        Ok(DiagnosticStatus {
            trace_schema_version: DIAGNOSTIC_TRACE_SCHEMA_VERSION,
            retention_days: self.limits.retention_ms / 86_400_000,
            maximum_bytes: self.limits.max_total_bytes,
            file_count: snapshot.sources.len(),
            stored_bytes: snapshot.sources.iter().map(|file| file.byte_count).sum(),
            event_count: snapshot.events.len(),
        })
    }
    pub fn prepare_bundle(
        &self,
        app_version: &str,
        build_commit: &str,
        now: i64,
    ) -> DiagnosticResult<PreparedDiagnosticBundle> {
        validate_token(app_version, "app_version", 64)?;
        validate_token(build_commit, "build_commit", 128)?;
        if now < 0 {
            return Err(DiagnosticError::InvalidInput("timestamp is invalid".into()));
        }
        let snapshot = self.snapshot()?;
        let excluded = excluded();
        let bytes = serde_json::to_vec_pretty(&Bundle {
            manifest: Manifest {
                bundle_schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
                trace_schema_version: DIAGNOSTIC_TRACE_SCHEMA_VERSION,
                created_at: now,
                app_version: app_version.to_owned(),
                build_commit: build_commit.to_owned(),
                event_count: snapshot.events.len(),
                source_files: snapshot.sources.clone(),
                excluded_by_default: excluded.clone(),
                redaction_policy: "allowlist_v1".into(),
            },
            events: &snapshot.events,
        })?;
        if bytes.len() > self.limits.max_bundle_bytes {
            return Err(DiagnosticError::LimitExceeded("bundle byte cap".into()));
        }
        let preview = DiagnosticBundlePreview {
            preview_id: format!("{:x}", Sha256::digest(&bytes)),
            bundle_schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
            event_count: snapshot.events.len(),
            source_file_count: snapshot.sources.len(),
            byte_count: bytes.len() as u64,
            first_occurred_at: snapshot.events.first().map(|event| event.occurred_at),
            last_occurred_at: snapshot.events.last().map(|event| event.occurred_at),
            redaction_count: 0,
            excluded_by_default: excluded,
        };
        Ok(PreparedDiagnosticBundle { preview, bytes })
    }
    fn recover_and_index(&mut self) -> DiagnosticResult<()> {
        let active = self.root.join(ACTIVE_FILE);
        if active.exists() {
            reject_symlink(&active)?;
            let mut bytes = fs::read(&active)?;
            if bytes.len() as u64 > self.limits.max_file_bytes {
                return Err(DiagnosticError::LimitExceeded(
                    "active trace byte cap".into(),
                ));
            }
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                let complete = bytes
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map_or(0, |i| i + 1);
                OpenOptions::new()
                    .write(true)
                    .open(&active)?
                    .set_len(complete as u64)?;
                bytes.truncate(complete);
            }
            let events = parse_trace(&bytes, &active)?;
            self.active_first = events.first().map(|event| event.sequence);
            self.active_last = events.last().map(|event| event.sequence);
            self.active_bytes = bytes.len() as u64;
        }
        if let Some(maximum) = self
            .snapshot()?
            .events
            .iter()
            .map(|event| event.sequence)
            .max()
        {
            self.next_sequence = maximum
                .checked_add(1)
                .ok_or_else(|| DiagnosticError::LimitExceeded("sequence exhausted".into()))?;
        }
        Ok(())
    }
    fn rotate(&mut self) -> DiagnosticResult<()> {
        let (Some(first), Some(last)) = (self.active_first, self.active_last) else {
            return Ok(());
        };
        let destination = self.root.join(rotated_name(first, last));
        if destination.exists() {
            return Err(DiagnosticError::CorruptTrace("rotation collision".into()));
        }
        fs::rename(self.root.join(ACTIVE_FILE), destination)?;
        self.active_first = None;
        self.active_last = None;
        self.active_bytes = 0;
        Ok(())
    }
    fn enforce_retention(&mut self, now: i64) -> DiagnosticResult<()> {
        let cutoff = now.saturating_sub(self.limits.retention_ms);
        let active_path = self.root.join(ACTIVE_FILE);
        if active_path.is_file() {
            let bytes = read_bounded(&active_path, self.limits.max_file_bytes)?;
            let events = parse_trace(&bytes, &active_path)?;
            if events
                .last()
                .is_some_and(|event| event.occurred_at < cutoff)
            {
                fs::remove_file(&active_path)?;
                self.active_first = None;
                self.active_last = None;
                self.active_bytes = 0;
            }
        }
        for file in self.rotated()? {
            if file.last_occurred_at < cutoff {
                fs::remove_file(file.path)?;
            }
        }
        let files = self.rotated()?;
        let active = self
            .root
            .join(ACTIVE_FILE)
            .metadata()
            .map(|value| value.len())
            .unwrap_or(0);
        let mut total = active + files.iter().map(|file| file.bytes).sum::<u64>();
        let mut count = usize::from(active > 0) + files.len();
        for file in files {
            if total <= self.limits.max_total_bytes && count <= self.limits.max_files {
                break;
            }
            fs::remove_file(file.path)?;
            total = total.saturating_sub(file.bytes);
            count = count.saturating_sub(1);
        }
        if total > self.limits.max_total_bytes || count > self.limits.max_files {
            return Err(DiagnosticError::LimitExceeded(
                "active trace exceeds rolling limits".into(),
            ));
        }
        Ok(())
    }
    fn rotated(&self) -> DiagnosticResult<Vec<Rotated>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ACTIVE_FILE {
                continue;
            }
            let Some((first, last)) = parse_rotated_name(&name) else {
                return Err(DiagnosticError::UnknownEntry(name));
            };
            reject_symlink(&entry.path())?;
            let bytes = read_bounded(&entry.path(), self.limits.max_file_bytes)?;
            let events = parse_trace(&bytes, &entry.path())?;
            if events.first().map(|event| event.sequence) != Some(first)
                || events.last().map(|event| event.sequence) != Some(last)
            {
                return Err(DiagnosticError::CorruptTrace(
                    "filename sequence mismatch".into(),
                ));
            }
            files.push(Rotated {
                path: entry.path(),
                first,
                bytes: bytes.len() as u64,
                last_occurred_at: events.last().map(|event| event.occurred_at).unwrap_or(0),
            });
        }
        files.sort_by_key(|file| file.first);
        Ok(files)
    }
    fn snapshot(&self) -> DiagnosticResult<Snapshot> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let order = if name == ACTIVE_FILE {
                u64::MAX
            } else if let Some((first, _)) = parse_rotated_name(&name) {
                first
            } else {
                return Err(DiagnosticError::UnknownEntry(name));
            };
            if !entry.file_type()?.is_file() {
                return Err(DiagnosticError::UnknownEntry(name));
            }
            entries.push((order, name, entry.path()));
        }
        entries.sort_by_key(|entry| entry.0);
        let mut events = Vec::new();
        let mut sources = Vec::new();
        let mut seen = BTreeSet::new();
        for (_, name, path) in entries {
            let bytes = read_bounded(&path, self.limits.max_file_bytes)?;
            let parsed = parse_trace(&bytes, &path)?;
            for event in &parsed {
                if !seen.insert(event.sequence) {
                    return Err(DiagnosticError::CorruptTrace("duplicate sequence".into()));
                }
            }
            sources.push(SourceFile {
                name,
                byte_count: bytes.len() as u64,
                event_count: parsed.len(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            });
            events.extend(parsed);
        }
        events.sort_by_key(|event| event.sequence);
        Ok(Snapshot { events, sources })
    }
}
struct Rotated {
    path: PathBuf,
    first: u64,
    bytes: u64,
    last_occurred_at: i64,
}
struct Snapshot {
    events: Vec<DiagnosticEvent>,
    sources: Vec<SourceFile>,
}

fn parse_trace(bytes: &[u8], path: &Path) -> DiagnosticResult<Vec<DiagnosticEvent>> {
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err(DiagnosticError::CorruptTrace(format!(
            "{} has incomplete tail",
            path.display()
        )));
    }
    let mut events = Vec::new();
    let mut previous = None;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        if line.len() > DIAGNOSTIC_MAX_EVENT_BYTES {
            return Err(DiagnosticError::LimitExceeded(
                "stored event parser cap".into(),
            ));
        }
        let event: DiagnosticEvent = serde_json::from_slice(line)?;
        event.validate()?;
        if previous.is_some_and(|value| event.sequence <= value) {
            return Err(DiagnosticError::CorruptTrace(
                "sequence is not increasing".into(),
            ));
        }
        previous = Some(event.sequence);
        events.push(event);
    }
    Ok(events)
}
fn read_bounded(path: &Path, limit: u64) -> DiagnosticResult<Vec<u8>> {
    let metadata = path.metadata()?;
    if metadata.len() > limit {
        return Err(DiagnosticError::LimitExceeded("trace file cap".into()));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(DiagnosticError::LimitExceeded(
            "trace grew while reading".into(),
        ));
    }
    Ok(bytes)
}
fn rotated_name(first: u64, last: u64) -> String {
    format!("trace-{first:020}-{last:020}.ndjson")
}
fn parse_rotated_name(name: &str) -> Option<(u64, u64)> {
    let value = name.strip_prefix("trace-")?.strip_suffix(".ndjson")?;
    let (first, last) = value.split_once('-')?;
    if first.len() != 20 || last.len() != 20 {
        return None;
    }
    Some((first.parse().ok()?, last.parse().ok()?))
}
fn validate_token(value: &str, field: &str, maximum: usize) -> DiagnosticResult<()> {
    let allowed = value.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'-' | b'_' | b'.' | b':')
    });
    if value.is_empty() || value.len() > maximum || !allowed {
        return Err(DiagnosticError::InvalidInput(format!(
            "{field} is not a machine token"
        )));
    }
    Ok(())
}
fn validate_limits(limits: DiagnosticLimits) -> DiagnosticResult<()> {
    if limits.retention_ms <= 0
        || limits.max_total_bytes == 0
        || limits.max_file_bytes == 0
        || limits.max_files == 0
        || limits.max_event_bytes == 0
        || limits.max_bundle_bytes == 0
        || limits.max_file_bytes > limits.max_total_bytes
    {
        return Err(DiagnosticError::InvalidInput(
            "limits are inconsistent".into(),
        ));
    }
    Ok(())
}
fn reject_symlink(path: &Path) -> DiagnosticResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let mut is_reparse = metadata.file_type().is_symlink();
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                is_reparse |= metadata.file_attributes()
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                    != 0;
            }
            if is_reparse {
                Err(DiagnosticError::InvalidInput(
                    "diagnostic path is a symlink or reparse point".into(),
                ))
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
fn validate_export_destination(destination: &Path) -> DiagnosticResult<()> {
    if !destination.is_absolute() {
        return Err(DiagnosticError::InvalidInput(
            "export path must be absolute".into(),
        ));
    }
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| DiagnosticError::InvalidInput("export filename is invalid".into()))?;
    if !name.ends_with(".wigigadiag.json") || name.len() > 160 {
        return Err(DiagnosticError::InvalidInput(
            "export extension is invalid".into(),
        ));
    }
    if destination.exists() {
        return Err(DiagnosticError::InvalidInput(
            "export will not overwrite a file".into(),
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| DiagnosticError::InvalidInput("export parent is missing".into()))?;
    reject_symlink(parent)?;
    if !parent.is_dir() {
        return Err(DiagnosticError::InvalidInput(
            "export parent is not a directory".into(),
        ));
    }
    Ok(())
}
fn excluded() -> Vec<String> {
    [
        "audio",
        "transcript",
        "clipboard",
        "window_title",
        "absolute_path",
        "environment",
        "secret",
        "token",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
