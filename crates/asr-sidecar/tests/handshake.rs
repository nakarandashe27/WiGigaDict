use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use wigigadict_protocol::{HelloAck, MAX_NDJSON_LINE_BYTES, PROTOCOL_VERSION, Role};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/contracts/ndjson")
}

fn fixture_file(name: &str) -> File {
    File::open(fixtures().join(name)).expect("contract fixture must be readable")
}

fn sidecar() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wigigadict-asr-sidecar"))
}

#[test]
fn fixture_driven_handshake_returns_a_versioned_ack() {
    let output = sidecar()
        .stdin(Stdio::from(fixture_file("hello.valid.ndjson")))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("sidecar must start");

    assert!(
        output.status.success(),
        "valid handshake failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = String::from_utf8(output.stdout).expect("hello_ack must be UTF-8");
    let ack: HelloAck = serde_json::from_str(&response).expect("hello_ack must be valid JSON");

    assert_eq!(ack.message_type, "hello_ack");
    assert_eq!(ack.protocol, PROTOCOL_VERSION);
    assert_eq!(ack.role, Role::Sidecar);
}

#[test]
fn incompatible_fixture_is_rejected() {
    let output = sidecar()
        .stdin(Stdio::from(fixture_file("hello.wrong-protocol.ndjson")))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("sidecar must start");

    assert!(!output.status.success());
}

#[test]
fn oversized_ndjson_is_rejected_without_an_ack() {
    let path = std::env::temp_dir().join(format!(
        "wigigadict-oversized-handshake-{}.ndjson",
        std::process::id()
    ));
    let mut input = File::create(&path).expect("oversized fixture must be created");
    input
        .write_all(&vec![b'x'; MAX_NDJSON_LINE_BYTES + 1])
        .unwrap();
    input.write_all(b"\n").unwrap();
    input.flush().unwrap();
    drop(input);

    let output = sidecar()
        .stdin(Stdio::from(
            File::open(&path).expect("oversized fixture must reopen"),
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .expect("sidecar must start");
    let _ = fs::remove_file(path);

    assert!(output.stdout.is_empty());
    assert!(!output.status.success());
}
