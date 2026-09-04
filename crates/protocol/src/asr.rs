use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::path::Path;

pub const MAX_PROTOCOL_PATH_BYTES: usize = 32_767;
pub const MAX_RAW_TRANSCRIPT_BYTES: usize = 1024 * 1024;
pub const MAX_ASR_SEGMENTS: usize = 4096;
pub const MAX_AUDIO_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_AUDIO_DURATION_MS: u64 = 1_100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolValidationError(pub String);

impl Display for ProtocolValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolValidationError {}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// The exact ADR-006 package. Kept as its own variant so the Step 16 golden-flow baseline
    /// keeps producing byte-identical requests after the catalog arrived.
    WhisperLargeV3TurboQ5,
    /// Any other ggml Whisper model from the signed catalog. Same worker, same CLI, different
    /// weights.
    WhisperGgml,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WhisperProfile {
    Vulkan,
    CpuT16,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LanguageHint {
    Auto,
    Ru,
    En,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSpec {
    pub engine: EngineKind,
    pub worker_path: String,
    pub worker_sha256: String,
    pub model_path: String,
    pub model_sha256: String,
    pub profile: WhisperProfile,
}

impl RuntimeSpec {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_absolute_path("worker_path", &self.worker_path)?;
        validate_absolute_path("model_path", &self.model_path)?;
        validate_sha256("worker_sha256", &self.worker_sha256)?;
        validate_sha256("model_sha256", &self.model_sha256)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TranscribeRequest {
    pub request_id: String,
    pub attempt_id: String,
    pub lease_generation: u32,
    pub audio_path: String,
    pub audio_sha256: String,
    pub audio_byte_size: u64,
    pub audio_duration_ms: u64,
    pub language: LanguageHint,
    pub timeout_ms: u64,
    pub runtime: RuntimeSpec,
}

impl TranscribeRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_token("request_id", &self.request_id)?;
        validate_token("attempt_id", &self.attempt_id)?;
        if self.lease_generation == 0 {
            return invalid("lease_generation must be positive");
        }
        validate_absolute_path("audio_path", &self.audio_path)?;
        validate_sha256("audio_sha256", &self.audio_sha256)?;
        if self.audio_byte_size == 0 || self.audio_byte_size > MAX_AUDIO_BYTES {
            return invalid("audio_byte_size is outside the 32 MiB contract");
        }
        if self.audio_duration_ms == 0 || self.audio_duration_ms > MAX_AUDIO_DURATION_MS {
            return invalid("audio_duration_ms is outside the dictation contract");
        }
        if !(1_000..=600_000).contains(&self.timeout_ms) {
            return invalid("timeout_ms must be between 1 s and 10 min");
        }
        self.runtime.validate()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ping {
    pub nonce: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShellCommand {
    Transcribe(Box<TranscribeRequest>),
    Cancel(CancelRequest),
    Ping(Ping),
}

impl ShellCommand {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Transcribe(request) => request.validate(),
            Self::Cancel(request) => validate_token("request_id", &request.request_id),
            Self::Ping(_) => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Accepted {
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    pub request_id: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AsrSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EngineOutput {
    pub text: String,
    pub segments: Vec<AsrSegment>,
}

impl EngineOutput {
    pub fn validate(&self, duration_ms: u64) -> Result<(), ProtocolValidationError> {
        if self.text.len() > MAX_RAW_TRANSCRIPT_BYTES {
            return invalid("raw transcript exceeds 1 MiB");
        }
        if self.segments.len() > MAX_ASR_SEGMENTS {
            return invalid("ASR segment count exceeds the protocol limit");
        }
        let mut previous_end = 0;
        let mut segment_text_bytes = 0usize;
        for segment in &self.segments {
            if segment.start_ms < previous_end || segment.end_ms < segment.start_ms {
                return invalid("ASR segments are not monotonic");
            }
            if segment.end_ms > duration_ms {
                return invalid("ASR segment exceeds WAV duration");
            }
            segment_text_bytes = segment_text_bytes
                .checked_add(segment.text.len())
                .ok_or_else(|| ProtocolValidationError("segment text size overflow".into()))?;
            previous_end = segment.end_ms;
        }
        if segment_text_bytes > MAX_RAW_TRANSCRIPT_BYTES {
            return invalid("ASR segment text exceeds 1 MiB");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TranscribeResult {
    pub request_id: String,
    pub attempt_id: String,
    pub lease_generation: u32,
    pub text: String,
    pub segments: Vec<AsrSegment>,
    pub inference_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SidecarFailure {
    pub request_id: Option<String>,
    pub code: String,
    pub transient: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cancelled {
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pong {
    pub nonce: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SidecarEvent {
    Accepted(Accepted),
    Heartbeat(Heartbeat),
    Result(TranscribeResult),
    Error(SidecarFailure),
    Cancelled(Cancelled),
    Pong(Pong),
}

impl SidecarEvent {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Accepted(value) => validate_token("request_id", &value.request_id),
            Self::Heartbeat(value) => validate_token("request_id", &value.request_id),
            Self::Result(value) => {
                validate_token("request_id", &value.request_id)?;
                validate_token("attempt_id", &value.attempt_id)?;
                if value.lease_generation == 0 {
                    return invalid("lease_generation must be positive");
                }
                EngineOutput {
                    text: value.text.clone(),
                    segments: value.segments.clone(),
                }
                .validate(MAX_AUDIO_DURATION_MS)
            }
            Self::Error(value) => {
                if let Some(request_id) = &value.request_id {
                    validate_token("request_id", request_id)?;
                }
                validate_error_code(&value.code)
            }
            Self::Cancelled(value) => validate_token("request_id", &value.request_id),
            Self::Pong(_) => Ok(()),
        }
    }
}

fn validate_absolute_path(name: &str, value: &str) -> Result<(), ProtocolValidationError> {
    if value.is_empty()
        || value.len() > MAX_PROTOCOL_PATH_BYTES
        || value.contains('\0')
        || (!Path::new(value).is_absolute()
            && !(value.len() >= 3
                && value.as_bytes()[1] == b':'
                && matches!(value.as_bytes()[2], b'\\' | b'/')))
    {
        return invalid(&format!("{name} must be a bounded absolute path"));
    }
    Ok(())
}

fn validate_sha256(name: &str, value: &str) -> Result<(), ProtocolValidationError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invalid(&format!("{name} must be SHA-256 hex"));
    }
    Ok(())
}

fn validate_token(name: &str, value: &str) -> Result<(), ProtocolValidationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return invalid(&format!("invalid {name}"));
    }
    Ok(())
}

fn validate_error_code(value: &str) -> Result<(), ProtocolValidationError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return invalid("invalid sidecar error code");
    }
    Ok(())
}

fn invalid<T>(detail: &str) -> Result<T, ProtocolValidationError> {
    Err(ProtocolValidationError(detail.into()))
}
