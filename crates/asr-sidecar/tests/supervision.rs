#[cfg(feature = "fixture-engine")]
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use wigigadict_protocol::{
    EngineKind, LanguageHint, Ping, RuntimeSpec, ShellCommand, SidecarEvent, TranscribeRequest,
    WhisperProfile,
};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/contracts/ndjson")
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Client {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_wigigadict-asr-sidecar"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        stdin
            .write_all(
                fs::read(fixtures().join("hello.valid.ndjson"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        stdin.flush().unwrap();
        let mut ack = String::new();
        stdout.read_line(&mut ack).unwrap();
        assert!(ack.contains("hello_ack"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send_json(&mut self, value: &impl serde::Serialize) {
        serde_json::to_writer(&mut self.stdin, value).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn event(&mut self) -> SidecarEvent {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn finish(mut self) {
        drop(self.stdin);
        assert!(self.child.wait().unwrap().success());
    }
}

fn request() -> TranscribeRequest {
    TranscribeRequest {
        request_id: "request-1".into(),
        attempt_id: "attempt-1".into(),
        lease_generation: 1,
        audio_path: r"C:\missing\audio.wav".into(),
        audio_sha256: HASH.into(),
        audio_byte_size: 32_000,
        audio_duration_ms: 1_000,
        language: LanguageHint::Auto,
        timeout_ms: 30_000,
        runtime: RuntimeSpec {
            engine: EngineKind::WhisperLargeV3TurboQ5,
            worker_path: r"C:\missing\worker.exe".into(),
            worker_sha256: HASH.into(),
            model_path: r"C:\missing\model.bin".into(),
            model_sha256: HASH.into(),
            profile: WhisperProfile::Vulkan,
        },
    }
}

#[test]
fn ping_remains_responsive_without_loading_an_engine() {
    let mut client = Client::start();
    client.send_json(&ShellCommand::Ping(Ping { nonce: 42 }));
    assert!(matches!(
        client.event(),
        SidecarEvent::Pong(value) if value.nonce == 42
    ));
    client.finish();
}

#[test]
fn missing_signed_artifact_fails_after_accept_without_raw_paths_in_error() {
    let mut client = Client::start();
    client.send_json(&ShellCommand::Transcribe(Box::new(request())));
    assert!(matches!(
        client.event(),
        SidecarEvent::Accepted(value) if value.request_id == "request-1"
    ));
    assert!(matches!(
        client.event(),
        SidecarEvent::Error(value)
            if value.request_id.as_deref() == Some("request-1")
                && value.code == "artifact_mismatch"
                && !value.transient
    ));
    client.finish();
}

#[test]
fn arbitrary_profile_is_rejected_before_accept() {
    let mut client = Client::start();
    let mut value = serde_json::to_value(ShellCommand::Transcribe(Box::new(request()))).unwrap();
    value["runtime"]["profile"] = serde_json::json!("cpu-t99");
    client.send_json(&value);
    assert!(matches!(
        client.event(),
        SidecarEvent::Error(value)
            if value.request_id.is_none()
                && value.code == "invalid_message"
                && !value.transient
    ));
    client.finish();
}

#[cfg(feature = "fixture-engine")]
#[test]
fn frozen_worker_adapter_returns_a_bounded_result_process_to_process() {
    let root = std::env::temp_dir().join(format!(
        "wigigadict-sidecar-positive-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let model = root.join("model.bin");
    let audio = root.join("audio.wav");
    fs::write(&model, b"model").unwrap();
    fs::write(&audio, b"audio").unwrap();
    let worker = PathBuf::from(env!("CARGO_BIN_EXE_wigigadict-asr-fixture-worker"));
    let mut transcribe = request();
    transcribe.audio_path = audio.to_string_lossy().into_owned();
    transcribe.audio_sha256 = sha256(&audio);
    transcribe.audio_byte_size = fs::metadata(&audio).unwrap().len();
    transcribe.runtime.worker_path = worker.to_string_lossy().into_owned();
    transcribe.runtime.worker_sha256 = sha256(&worker);
    transcribe.runtime.model_path = model.to_string_lossy().into_owned();
    transcribe.runtime.model_sha256 = sha256(&model);

    let mut client = Client::start();
    client.send_json(&ShellCommand::Transcribe(Box::new(transcribe)));
    assert!(matches!(client.event(), SidecarEvent::Accepted(_)));
    assert!(matches!(
        client.event(),
        SidecarEvent::Result(value)
            if value.text == "fixture raw"
                && value.segments.len() == 1
                && value.segments[0].end_ms == 500
    ));
    client.finish();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "fixture-engine")]
fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
