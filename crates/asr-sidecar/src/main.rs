use std::io::{self, BufRead, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use wigigadict_protocol::{
    Accepted, Cancelled, Heartbeat, Hello, HelloAck, MAX_NDJSON_LINE_BYTES, PROTOCOL_VERSION, Pong,
    Role, ShellCommand, SidecarEvent, SidecarFailure, TranscribeResult,
};

mod engine;

use engine::{AsrEngineAdapter, EngineError, JobResult, WhisperWorkerAdapter, spawn_transcription};

const APP_VERSION: &str = "0.0.1-dev";
const BUILD_COMMIT: &str = match option_env!("WIGIGADICT_BUILD_COMMIT") {
    Some(value) => value,
    None => "unknown",
};
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const COMMAND_POLL: Duration = Duration::from_millis(100);

enum ReaderEvent {
    Command(ShellCommand),
    Invalid,
    Fatal,
    Eof,
}

struct ActiveJob {
    request_id: String,
    cancelled: Arc<AtomicBool>,
    started: Instant,
    last_heartbeat: Instant,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    perform_handshake(&mut input, &mut stdout)?;
    drop(input);

    let (input_tx, input_rx) = mpsc::channel();
    let (job_tx, job_rx) = mpsc::channel();
    thread::spawn(move || read_commands(input_tx));
    let adapter: Arc<dyn AsrEngineAdapter> = Arc::new(WhisperWorkerAdapter);
    let mut active: Option<ActiveJob> = None;
    let mut input_closed = false;

    loop {
        while let Ok(completed) = job_rx.try_recv() {
            finish_job(&mut stdout, &mut active, completed)?;
        }
        if input_closed && active.is_none() {
            break;
        }

        match input_rx.recv_timeout(COMMAND_POLL) {
            Ok(ReaderEvent::Command(command)) => {
                if command.validate().is_err() {
                    write_event(
                        &mut stdout,
                        &SidecarEvent::Error(SidecarFailure {
                            request_id: None,
                            code: "invalid_request".into(),
                            transient: false,
                        }),
                    )?;
                    continue;
                }
                match command {
                    ShellCommand::Transcribe(request) => {
                        let request = *request;
                        if active.is_some() {
                            write_event(
                                &mut stdout,
                                &SidecarEvent::Error(SidecarFailure {
                                    request_id: Some(request.request_id),
                                    code: "sidecar_busy".into(),
                                    transient: true,
                                }),
                            )?;
                            continue;
                        }
                        let cancelled = Arc::new(AtomicBool::new(false));
                        let request_id = request.request_id.clone();
                        write_event(
                            &mut stdout,
                            &SidecarEvent::Accepted(Accepted {
                                request_id: request_id.clone(),
                            }),
                        )?;
                        spawn_transcription(
                            adapter.clone(),
                            request,
                            cancelled.clone(),
                            job_tx.clone(),
                        );
                        let now = Instant::now();
                        active = Some(ActiveJob {
                            request_id,
                            cancelled,
                            started: now,
                            last_heartbeat: now,
                        });
                    }
                    ShellCommand::Cancel(cancel) => match active.as_ref() {
                        Some(job) if job.request_id == cancel.request_id => {
                            job.cancelled.store(true, Ordering::Release);
                        }
                        _ => {
                            write_event(
                                &mut stdout,
                                &SidecarEvent::Error(SidecarFailure {
                                    request_id: Some(cancel.request_id),
                                    code: "unknown_request".into(),
                                    transient: false,
                                }),
                            )?;
                        }
                    },
                    ShellCommand::Ping(ping) => {
                        write_event(&mut stdout, &SidecarEvent::Pong(Pong { nonce: ping.nonce }))?;
                    }
                }
            }
            Ok(ReaderEvent::Invalid) => {
                write_event(
                    &mut stdout,
                    &SidecarEvent::Error(SidecarFailure {
                        request_id: None,
                        code: "invalid_message".into(),
                        transient: false,
                    }),
                )?;
            }
            Ok(ReaderEvent::Fatal | ReaderEvent::Eof) => {
                input_closed = true;
                if let Some(job) = &active {
                    job.cancelled.store(true, Ordering::Release);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => input_closed = true,
        }

        if let Some(job) = active.as_mut()
            && job.last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL
        {
            write_event(
                &mut stdout,
                &SidecarEvent::Heartbeat(Heartbeat {
                    request_id: job.request_id.clone(),
                    elapsed_ms: job.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                }),
            )?;
            job.last_heartbeat = Instant::now();
        }
    }
    Ok(())
}

fn perform_handshake(
    input: &mut impl BufRead,
    stdout: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some(line) = read_bounded_line(input)? {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let hello: Hello = serde_json::from_slice(&line)?;
        if hello.message_type != "hello"
            || hello.protocol != PROTOCOL_VERSION
            || hello.role != Role::Shell
        {
            return Err("unsupported or incompatible handshake".into());
        }
        let ack = HelloAck {
            message_type: "hello_ack".to_owned(),
            protocol: PROTOCOL_VERSION.to_owned(),
            app: APP_VERSION.to_owned(),
            commit: BUILD_COMMIT.to_owned(),
            role: Role::Sidecar,
        };
        serde_json::to_writer(&mut *stdout, &ack)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
        return Ok(());
    }
    Err("sidecar stdin closed before handshake".into())
}

fn read_commands(sender: mpsc::Sender<ReaderEvent>) {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    loop {
        match read_bounded_line(&mut input) {
            Ok(Some(line)) if line.iter().all(u8::is_ascii_whitespace) => {}
            Ok(Some(line)) => match serde_json::from_slice::<ShellCommand>(&line) {
                Ok(command) => {
                    if sender.send(ReaderEvent::Command(command)).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    if sender.send(ReaderEvent::Invalid).is_err() {
                        break;
                    }
                }
            },
            Ok(None) => {
                let _ = sender.send(ReaderEvent::Eof);
                break;
            }
            Err(_) => {
                let _ = sender.send(ReaderEvent::Fatal);
                break;
            }
        }
    }
}

fn finish_job(
    stdout: &mut impl Write,
    active: &mut Option<ActiveJob>,
    completed: JobResult,
) -> Result<(), Box<dyn std::error::Error>> {
    let matches_active = active
        .as_ref()
        .is_some_and(|job| job.request_id == completed.request.request_id);
    if !matches_active {
        return Ok(());
    }
    *active = None;
    match completed.result {
        Ok(transcript) => write_event(
            stdout,
            &SidecarEvent::Result(TranscribeResult {
                request_id: completed.request.request_id,
                attempt_id: completed.request.attempt_id,
                lease_generation: completed.request.lease_generation,
                text: transcript.output.text,
                segments: transcript.output.segments,
                inference_ms: transcript.inference_ms,
            }),
        )?,
        Err(EngineError::Cancelled) => write_event(
            stdout,
            &SidecarEvent::Cancelled(Cancelled {
                request_id: completed.request.request_id,
            }),
        )?,
        Err(error) => write_event(
            stdout,
            &SidecarEvent::Error(SidecarFailure {
                request_id: Some(completed.request.request_id),
                code: error.code().into(),
                transient: error.transient(),
            }),
        )?,
    }
    Ok(())
}

fn write_event(
    stdout: &mut impl Write,
    event: &SidecarEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    event.validate()?;
    serde_json::to_writer(&mut *stdout, event)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let bytes_read = reader
        .take((MAX_NDJSON_LINE_BYTES + 2) as u64)
        .read_until(b'\n', &mut line)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    if line.len() > MAX_NDJSON_LINE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "NDJSON line exceeds the protocol limit",
        ));
    }
    Ok(Some(line))
}
