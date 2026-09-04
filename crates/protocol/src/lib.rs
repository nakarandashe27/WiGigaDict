//! Shared, intentionally small shell ↔ ASR-sidecar wire contract.

mod asr;
mod message;

pub use asr::{
    Accepted, AsrSegment, CancelRequest, Cancelled, EngineKind, EngineOutput, Heartbeat,
    LanguageHint, MAX_ASR_SEGMENTS, MAX_AUDIO_BYTES, MAX_AUDIO_DURATION_MS,
    MAX_PROTOCOL_PATH_BYTES, MAX_RAW_TRANSCRIPT_BYTES, Ping, Pong, ProtocolValidationError,
    RuntimeSpec, ShellCommand, SidecarEvent, SidecarFailure, TranscribeRequest, TranscribeResult,
    WhisperProfile,
};
pub use message::{Hello, HelloAck, MAX_NDJSON_LINE_BYTES, PROTOCOL_VERSION, Role};
