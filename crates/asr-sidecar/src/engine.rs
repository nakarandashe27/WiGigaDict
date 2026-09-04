use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wigigadict_protocol::{
    AsrSegment, EngineKind, EngineOutput, LanguageHint, MAX_NDJSON_LINE_BYTES, TranscribeRequest,
    WhisperProfile,
};

const HASH_BUFFER_BYTES: usize = 64 * 1024;
const MAX_WORKER_STDERR_BYTES: usize = 16 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub trait AsrEngineAdapter: Send + Sync + 'static {
    fn transcribe(
        &self,
        request: &TranscribeRequest,
        cancelled: &AtomicBool,
    ) -> Result<EngineTranscript, EngineError>;
}

#[derive(Debug)]
pub struct EngineTranscript {
    pub output: EngineOutput,
    pub inference_ms: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WhisperWorkerAdapter;

#[derive(Debug)]
pub enum EngineError {
    InvalidRequest,
    ArtifactMismatch,
    Spawn(io::Error),
    WorkerIo,
    WorkerExited,
    InvalidOutput,
    Timeout,
    Cancelled,
}

impl EngineError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::ArtifactMismatch => "artifact_mismatch",
            Self::Spawn(_) => "worker_spawn_failed",
            Self::WorkerIo => "worker_io_failed",
            Self::WorkerExited => "worker_exited",
            Self::InvalidOutput => "invalid_worker_output",
            Self::Timeout => "worker_timeout",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn transient(&self) -> bool {
        matches!(
            self,
            Self::Spawn(_) | Self::WorkerIo | Self::WorkerExited | Self::Timeout
        )
    }
}

impl Display for EngineError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "{}: {error}", self.code()),
            _ => formatter.write_str(self.code()),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            _ => None,
        }
    }
}

impl AsrEngineAdapter for WhisperWorkerAdapter {
    fn transcribe(
        &self,
        request: &TranscribeRequest,
        cancelled: &AtomicBool,
    ) -> Result<EngineTranscript, EngineError> {
        request
            .validate()
            .map_err(|_| EngineError::InvalidRequest)?;
        // Both variants are ggml Whisper weights driven by the same run-whisper worker; a future
        // engine on another runtime must not silently reach this adapter.
        if !matches!(
            request.runtime.engine,
            EngineKind::WhisperLargeV3TurboQ5 | EngineKind::WhisperGgml
        ) {
            return Err(EngineError::InvalidRequest);
        }
        verify_sha256(&request.runtime.worker_path, &request.runtime.worker_sha256)?;
        verify_sha256(&request.runtime.model_path, &request.runtime.model_sha256)?;
        verify_sha256(&request.audio_path, &request.audio_sha256)?;

        let (profile, threads) = match request.runtime.profile {
            WhisperProfile::Vulkan => ("gpu", "0"),
            WhisperProfile::CpuT16 => ("cpu-t16", "16"),
        };
        let language = match request.language {
            LanguageHint::Auto => "auto",
            LanguageHint::Ru => "ru",
            LanguageHint::En => "en",
        };
        let output = TempOutput::new(&request.request_id)?;
        let mut command = Command::new(&request.runtime.worker_path);
        command
            .arg("run-whisper")
            .arg("--model")
            .arg(&request.runtime.model_path)
            .arg("--audio")
            .arg(&request.audio_path)
            .arg("--sample")
            .arg(&request.attempt_id)
            .arg("--language")
            .arg(language)
            .arg("--profile")
            .arg(profile)
            .arg("--mode")
            .arg("cold")
            .arg("--threads")
            .arg(threads)
            .arg("--output")
            .arg(&output.path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(EngineError::Spawn)?;
        let stdout = child.stdout.take().ok_or(EngineError::WorkerIo)?;
        let stderr = child.stderr.take().ok_or(EngineError::WorkerIo)?;
        let stdout_reader = read_bounded(stdout, MAX_NDJSON_LINE_BYTES);
        let stderr_reader = read_bounded(stderr, MAX_WORKER_STDERR_BYTES);
        let started = Instant::now();
        let deadline = Duration::from_millis(request.timeout_ms);

        let status = loop {
            if cancelled.load(Ordering::Acquire) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineError::Cancelled);
            }
            if started.elapsed() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineError::Timeout);
            }
            match child.try_wait().map_err(|_| EngineError::WorkerIo)? {
                Some(status) => break status,
                None => thread::sleep(Duration::from_millis(25)),
            }
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| EngineError::WorkerIo)?
            .map_err(|_| EngineError::WorkerIo)?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| EngineError::WorkerIo)?
            .map_err(|_| EngineError::WorkerIo)?;
        if stdout.len() > MAX_NDJSON_LINE_BYTES || stderr.len() > MAX_WORKER_STDERR_BYTES {
            return Err(EngineError::InvalidOutput);
        }
        if !status.success() {
            // The bounded stderr tail is the only crash evidence a worker leaves
            // behind (GPU allocation failures land here). It goes to the sidecar's
            // own stderr, which the shell appends to logs/asr-sidecar.log; without
            // it every crash is an opaque worker_exited.
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            eprintln!(
                "{stamp} asr worker exited with {status}, stderr tail:\n{}",
                String::from_utf8_lossy(&stderr).trim_end()
            );
            return Err(EngineError::WorkerExited);
        }
        let record: WhisperRunRecord =
            serde_json::from_slice(&output.read()?).map_err(|_| EngineError::InvalidOutput)?;
        let transcript = EngineOutput {
            text: record.text,
            segments: record.segments,
        };
        transcript
            .validate(request.audio_duration_ms)
            .map_err(|_| EngineError::InvalidOutput)?;
        Ok(EngineTranscript {
            output: transcript,
            inference_ms: record.inference_ms,
        })
    }
}

#[derive(Deserialize)]
struct WhisperRunRecord {
    inference_ms: u64,
    text: String,
    segments: Vec<AsrSegment>,
}

struct TempOutput {
    directory: PathBuf,
    path: PathBuf,
}

impl TempOutput {
    fn new(request_id: &str) -> Result<Self, EngineError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "wigigadict-asr-{}-{request_id}-{nonce}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&directory).map_err(|_| EngineError::WorkerIo)?;
        let path = directory.join("result.ndjson");
        Ok(Self { directory, path })
    }

    fn read(&self) -> Result<Vec<u8>, EngineError> {
        let file = File::open(&self.path).map_err(|_| EngineError::InvalidOutput)?;
        let mut bytes = Vec::new();
        file.take((MAX_NDJSON_LINE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| EngineError::InvalidOutput)?;
        if bytes.len() > MAX_NDJSON_LINE_BYTES {
            return Err(EngineError::InvalidOutput);
        }
        Ok(bytes)
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn verify_sha256(path: &str, expected: &str) -> Result<(), EngineError> {
    let mut file = File::open(path).map_err(|_| EngineError::ArtifactMismatch)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| EngineError::ArtifactMismatch)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hash.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(EngineError::ArtifactMismatch)
    }
}

fn read_bounded(
    reader: impl Read + Send + 'static,
    limit: usize,
) -> thread::JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut output = Vec::new();
        reader.take((limit + 1) as u64).read_to_end(&mut output)?;
        Ok(output)
    })
}

pub fn spawn_transcription(
    adapter: Arc<dyn AsrEngineAdapter>,
    request: TranscribeRequest,
    cancelled: Arc<AtomicBool>,
    completed: std::sync::mpsc::Sender<JobResult>,
) {
    thread::spawn(move || {
        let result = adapter.transcribe(&request, &cancelled);
        let _ = completed.send(JobResult { request, result });
    });
}

pub struct JobResult {
    pub request: TranscribeRequest,
    pub result: Result<EngineTranscript, EngineError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_classification_allows_only_process_failures_to_retry() {
        assert!(EngineError::WorkerExited.transient());
        assert!(EngineError::Timeout.transient());
        assert!(!EngineError::ArtifactMismatch.transient());
        assert!(!EngineError::InvalidOutput.transient());
        assert!(!EngineError::Cancelled.transient());
    }

    #[test]
    fn missing_artifact_is_rejected_before_worker_spawn() {
        assert!(matches!(
            verify_sha256(r"C:\definitely-missing\worker.exe", "00"),
            Err(EngineError::ArtifactMismatch)
        ));
    }
}
