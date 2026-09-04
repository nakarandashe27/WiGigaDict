use std::fs;
use std::path::{Path, PathBuf};

use wigigadict_protocol::{Hello, HelloAck, PROTOCOL_VERSION, Role};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/contracts/ndjson")
}

fn read(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("contract fixture must be readable")
}

#[test]
fn valid_hello_matches_the_wire_contract() {
    let hello: Hello = serde_json::from_str(read("hello.valid.ndjson").trim()).unwrap();

    assert_eq!(hello.message_type, "hello");
    assert_eq!(hello.protocol, PROTOCOL_VERSION);
    assert_eq!(hello.role, Role::Shell);
}

#[test]
fn valid_ack_matches_the_wire_contract() {
    let ack: HelloAck = serde_json::from_str(read("hello-ack.valid.ndjson").trim()).unwrap();

    assert_eq!(ack.message_type, "hello_ack");
    assert_eq!(ack.protocol, PROTOCOL_VERSION);
    assert_eq!(ack.role, Role::Sidecar);
}

#[test]
fn unknown_critical_fields_are_rejected() {
    let result = serde_json::from_str::<Hello>(read("hello.unknown-field.ndjson").trim());
    assert!(result.is_err());
}

#[test]
fn incompatible_protocol_remains_visible_to_the_caller() {
    let hello: Hello = serde_json::from_str(read("hello.wrong-protocol.ndjson").trim()).unwrap();
    assert_ne!(hello.protocol, PROTOCOL_VERSION);
}
