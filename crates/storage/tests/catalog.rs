#![allow(linker_messages)]
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use wigigadict_storage::{
    CATALOG_SCHEMA_VERSION, CatalogEntry, CatalogRequirements, ModelCatalog, ModelManager,
    ModelManagerError, TrustedKeyRing, verify_catalog,
};

const KEY_ID: &str = "catalog-test-key";

fn manifest_json() -> String {
    serde_json::to_string(&json!({
        "schema_version": 1,
        "package_id": "whisper-small-cpu",
        "engine_family": "whisper",
        "model_name": "small",
        "model_version": "5359861c",
        "release_sequence": 1,
        "source_uri": "https://huggingface.co/ggerganov/whisper.cpp",
        "license_id": "MIT-whisper.cpp-model-card",
        "expected_size": 487_601_967_u64,
        "signature_key_id": KEY_ID,
        "minimum_manager_version": 1,
        "expires_at_ms": 4_102_444_800_000_i64,
        "compatibility_abi": "wigigadict-model-abi-v1",
        "files": [{
            "path": "ggml-small.bin",
            "size": 487_601_967_u64,
            "sha256": "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            "download_uri": "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin"
        }],
        "runtime": {
            "profile_id": "whisper-small-cpu",
            "profile_version": 1,
            "adapter_type": "transcribe-rs",
            "adapter_version": "0.3.11",
            "device_kind": "cpu",
            "device_id": null,
            "settings": {"worker_path": "w.exe", "model_path": "ggml-small.bin", "timeout_ms": 120000, "threads": 16},
            "probe_file": "ggml-small.bin"
        }
    }))
    .unwrap()
}

fn catalog(key: &SigningKey) -> ModelCatalog {
    let manifest_json = manifest_json();
    let signature = key.sign(manifest_json.as_bytes()).to_bytes();
    ModelCatalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        catalog_version: 1,
        generated_at_ms: 1_788_000_000_000,
        signature_key_id: KEY_ID.into(),
        entries: vec![CatalogEntry {
            display_name: "Whisper small".into(),
            summary: "Multilingual CPU model.".into(),
            languages: vec!["ru".into(), "en".into()],
            requirements: CatalogRequirements {
                device_kind: "cpu".into(),
                min_ram_mb: 2048,
                min_vram_mb: None,
            },
            recommended: false,
            owner_measured: false,
            manifest_json,
            manifest_signature_hex: signature.iter().map(|byte| format!("{byte:02x}")).collect(),
        }],
    }
}

fn sign(key: &SigningKey, bytes: &[u8]) -> String {
    key.sign(bytes)
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ring(key: &SigningKey) -> TrustedKeyRing {
    let mut keys = TrustedKeyRing::new();
    keys.insert(KEY_ID, key.verifying_key().to_bytes()).unwrap();
    keys
}

#[test]
fn a_signed_catalog_parses_and_any_tampering_is_fail_closed() {
    let key = SigningKey::from_bytes(&[11_u8; 32]);
    let bytes = serde_json::to_vec(&catalog(&key)).unwrap();
    let signature = sign(&key, &bytes);

    let parsed = verify_catalog(&ring(&key), &bytes, &signature).unwrap();
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].display_name, "Whisper small");
    assert!(!parsed.entries[0].owner_measured);

    // A single flipped byte in the shipped document invalidates the whole catalog.
    let mut tampered = bytes.clone();
    let position = tampered
        .windows(13)
        .position(|window| window == b"Whisper small")
        .expect("display name in document");
    tampered[position] = b'X';
    assert!(matches!(
        verify_catalog(&ring(&key), &tampered, &signature),
        Err(ModelManagerError::InvalidSignature)
    ));

    // A catalog signed by a key we do not trust is not a catalog.
    let stranger = SigningKey::from_bytes(&[12_u8; 32]);
    assert!(matches!(
        verify_catalog(&ring(&key), &bytes, &sign(&stranger, &bytes)),
        Err(ModelManagerError::InvalidSignature)
    ));

    let mut revoked = ring(&key);
    revoked.revoke(KEY_ID);
    assert!(matches!(
        verify_catalog(&revoked, &bytes, &signature),
        Err(ModelManagerError::RevokedSigningKey(_))
    ));

    let empty = TrustedKeyRing::new();
    assert!(matches!(
        verify_catalog(&empty, &bytes, &signature),
        Err(ModelManagerError::UnknownSigningKey(_))
    ));
}

#[test]
fn the_entry_manifest_is_exactly_what_the_installer_accepts() {
    let key = SigningKey::from_bytes(&[11_u8; 32]);
    let bytes = serde_json::to_vec(&catalog(&key)).unwrap();
    let parsed = verify_catalog(&ring(&key), &bytes, &sign(&key, &bytes)).unwrap();
    let entry = &parsed.entries[0];

    let manifest = entry.manifest().unwrap();
    assert_eq!(manifest.package_id, "whisper-small-cpu");
    assert_eq!(manifest.expected_size, 487_601_967);

    // The handoff that matters: the bytes the catalog ships are the bytes ModelManager verifies.
    let root = std::env::temp_dir().join(format!("wigigadict-catalog-{}", std::process::id()));
    let manager = ModelManager::open_in_memory(&root, ring(&key)).unwrap();
    let preview = manager
        .preview(&entry.signed_manifest(), 1_788_000_000_000)
        .unwrap();
    assert_eq!(preview.package_id, "whisper-small-cpu");
    assert_eq!(preview.license_id, "MIT-whisper.cpp-model-card");
    let _ = std::fs::remove_dir_all(&root);
}
