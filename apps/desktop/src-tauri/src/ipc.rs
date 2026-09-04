use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use wigigadict_protocol::{
    Hello, HelloAck, MAX_NDJSON_LINE_BYTES, PROTOCOL_VERSION, Role, ShellCommand, SidecarEvent,
};

use crate::version::{APP_VERSION, BUILD_COMMIT};

const SIDECAR_FILENAME: &str = "wigigadict-asr-sidecar-x86_64-pc-windows-msvc.exe";

#[derive(Debug, thiserror::Error)]
pub enum SidecarClientError {
    #[error("ASR sidecar executable was not found; checked {0:?}")]
    NotFound(Vec<PathBuf>),
    #[error("failed to start ASR sidecar: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("sidecar I/O failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("invalid sidecar protocol: {0}")]
    Protocol(String),
    #[error("sidecar process exited")]
    Exited,
}

pub struct SidecarClient {
    child: Child,
    stdin: ChildStdin,
    events: mpsc::Receiver<Result<SidecarEvent, String>>,
}

impl SidecarClient {
    pub fn start(path: &Path) -> Result<Self, SidecarClientError> {
        let mut command = Command::new(path);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(sidecar_stderr());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(SidecarClientError::Spawn)?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            SidecarClientError::Protocol("sidecar stdin was not piped".to_owned())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            SidecarClientError::Protocol("sidecar stdout was not piped".to_owned())
        })?;
        let mut reader = BufReader::new(stdout);
        let hello = Hello {
            message_type: "hello".to_owned(),
            protocol: PROTOCOL_VERSION.to_owned(),
            app: APP_VERSION.to_owned(),
            commit: BUILD_COMMIT.to_owned(),
            role: Role::Shell,
        };
        serde_json::to_writer(&mut stdin, &hello)
            .map_err(|error| SidecarClientError::Protocol(error.to_string()))?;
        stdin.write_all(b"\n").map_err(SidecarClientError::Io)?;
        stdin.flush().map_err(SidecarClientError::Io)?;
        let response = read_bounded_line(&mut reader).map_err(SidecarClientError::Io)?;
        let ack: HelloAck = serde_json::from_slice(&response)
            .map_err(|error| SidecarClientError::Protocol(error.to_string()))?;
        if ack.message_type != "hello_ack"
            || ack.protocol != PROTOCOL_VERSION
            || ack.role != Role::Sidecar
            || ack.app != APP_VERSION
        {
            return Err(SidecarClientError::Protocol(
                "sidecar handshake mismatch".into(),
            ));
        }

        let (events_tx, events) = mpsc::channel();
        thread::spawn(move || {
            loop {
                match read_bounded_line(&mut reader) {
                    Ok(line) => {
                        let event = serde_json::from_slice::<SidecarEvent>(&line)
                            .map_err(|error| error.to_string())
                            .and_then(|event| {
                                event.validate().map_err(|error| error.to_string())?;
                                Ok(event)
                            });
                        if events_tx.send(event).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(error) => {
                        let _ = events_tx.send(Err(error.to_string()));
                        break;
                    }
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            events,
        })
    }

    pub fn send(&mut self, command: &ShellCommand) -> Result<(), SidecarClientError> {
        command
            .validate()
            .map_err(|error| SidecarClientError::Protocol(error.to_string()))?;
        serde_json::to_writer(&mut self.stdin, command)
            .map_err(|error| SidecarClientError::Protocol(error.to_string()))?;
        self.stdin
            .write_all(b"\n")
            .map_err(SidecarClientError::Io)?;
        self.stdin.flush().map_err(SidecarClientError::Io)
    }

    pub fn receive(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<SidecarEvent>, SidecarClientError> {
        match self.events.recv_timeout(timeout) {
            Ok(Ok(event)) => Ok(Some(event)),
            Ok(Err(detail)) => Err(SidecarClientError::Protocol(detail)),
            Err(RecvTimeoutError::Timeout) => {
                if self
                    .child
                    .try_wait()
                    .map_err(SidecarClientError::Io)?
                    .is_some()
                {
                    Err(SidecarClientError::Exited)
                } else {
                    Ok(None)
                }
            }
            Err(RecvTimeoutError::Disconnected) => Err(SidecarClientError::Exited),
        }
    }
}

impl Drop for SidecarClient {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The sidecar is quiet on stderr except for worker crash evidence (bounded
/// tails of the ASR worker's stderr), so this file grows only when transcription
/// fails. Content-free diagnostics stay unchanged; this is the owner-local crash
/// detail they deliberately omit. Falls back to null when the log cannot open.
fn sidecar_stderr() -> Stdio {
    let Ok(local) = env::var("LOCALAPPDATA") else {
        return Stdio::null();
    };
    let directory = Path::new(&local).join("WiGigaDict").join("logs");
    if std::fs::create_dir_all(&directory).is_err() {
        return Stdio::null();
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("asr-sidecar.log"))
    {
        Ok(file) => Stdio::from(file),
        Err(_) => Stdio::null(),
    }
}

pub fn find_sidecar() -> Result<PathBuf, SidecarClientError> {
    let candidates = candidate_paths();
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or(SidecarClientError::NotFound(candidates))
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = env::var("WIGIGADICT_ASR_SIDECAR") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(executable) = env::current_exe()
        && let Some(directory) = executable.parent()
    {
        paths.push(directory.join(SIDECAR_FILENAME));
        paths.push(directory.join("resources").join(SIDECAR_FILENAME));
        paths.push(directory.join("binaries").join(SIDECAR_FILENAME));
    }
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(SIDECAR_FILENAME),
    );
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/debug/wigigadict-asr-sidecar.exe"),
    );
    paths
}

fn read_bounded_line(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let count = reader
        .take((MAX_NDJSON_LINE_BYTES + 2) as u64)
        .read_until(b'\n', &mut line)?;
    if count == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "sidecar stdout closed",
        ));
    }
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    if line.len() > MAX_NDJSON_LINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "sidecar NDJSON line exceeds the protocol limit",
        ));
    }
    Ok(line)
}
