//! Signs the model catalog with the project's offline Ed25519 key.
//!
//! The private key never lives in this repository: it is passed by path, and the tool refuses a
//! path inside the working tree because the repository is meant to be published.

use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use wigigadict_storage::{
    CATALOG_SCHEMA_VERSION, CatalogEntry, CatalogRequirements, ModelCatalog, ModelManifest,
    TrustedKeyRing, verify_catalog,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogSource {
    catalog_version: u64,
    generated_at_ms: i64,
    signature_key_id: String,
    entries: Vec<SourceEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceEntry {
    display_name: String,
    summary: String,
    languages: Vec<String>,
    requirements: CatalogRequirements,
    recommended: bool,
    owner_measured: bool,
    manifest: ModelManifest,
}

const BUNDLED_WORKER: &str = "wigigadict-asr-worker.exe";

fn main() {
    if let Err(error) = run() {
        eprintln!("model-catalog-sign: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.first().map(String::as_str) {
        Some("keygen") if arguments.len() == 2 => keygen(Path::new(&arguments[1])),
        Some("sign") if arguments.len() == 4 => sign(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
        ),
        _ => Err(
            "usage: model-catalog-sign keygen <private-key-file>\n       \
                  model-catalog-sign sign <source.json> <private-key-file> <out-dir>"
                .into(),
        ),
    }
}

fn keygen(destination: &Path) -> Result<(), Box<dyn Error>> {
    reject_path_inside_working_tree(destination)?;
    if destination.exists() {
        return Err("refusing to overwrite an existing private key".into());
    }
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|error| format!("operating system randomness unavailable: {error}"))?;
    let key = SigningKey::from_bytes(&secret);
    fs::write(destination, hex(&secret))?;
    println!("private key written to {}", destination.display());
    println!("public key (pin this in the desktop build):");
    println!("{}", hex(&key.verifying_key().to_bytes()));
    Ok(())
}

fn sign(source: &Path, private_key: &Path, out_dir: &Path) -> Result<(), Box<dyn Error>> {
    reject_path_inside_working_tree(private_key)?;
    let key = read_private_key(private_key)?;
    let source: CatalogSource = serde_json::from_slice(&fs::read(source)?)?;
    if source.entries.is_empty() {
        return Err("catalog source lists no entries".into());
    }

    let mut entries = Vec::new();
    for entry in &source.entries {
        validate(entry)?;
        // Sign the exact bytes the catalog will carry, so what gets verified is what ships.
        let manifest_json = serde_json::to_string(&entry.manifest)?;
        let signature = key.sign(manifest_json.as_bytes()).to_bytes();
        entries.push(CatalogEntry {
            display_name: entry.display_name.clone(),
            summary: entry.summary.clone(),
            languages: entry.languages.clone(),
            requirements: entry.requirements.clone(),
            recommended: entry.recommended,
            owner_measured: entry.owner_measured,
            manifest_json,
            manifest_signature_hex: hex(&signature),
        });
    }

    let catalog = ModelCatalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        catalog_version: source.catalog_version,
        generated_at_ms: source.generated_at_ms,
        signature_key_id: source.signature_key_id.clone(),
        entries,
    };
    let catalog_json = serde_json::to_vec_pretty(&catalog)?;
    let signature = hex(&key.sign(&catalog_json).to_bytes());

    // Self-check: a tool that emits a catalog its own verifier rejects is worse than no tool.
    let mut ring = TrustedKeyRing::new();
    ring.insert(
        source.signature_key_id.as_str(),
        key.verifying_key().to_bytes(),
    )?;
    verify_catalog(&ring, &catalog_json, &signature)?;

    fs::create_dir_all(out_dir)?;
    fs::write(out_dir.join("catalog.json"), &catalog_json)?;
    fs::write(out_dir.join("catalog.sig"), &signature)?;
    println!(
        "signed {} entries into {}",
        catalog.entries.len(),
        out_dir.display()
    );
    println!("public key: {}", hex(&key.verifying_key().to_bytes()));
    Ok(())
}

/// Rejects a catalog entry whose package could not actually run once installed. A manifest that
/// names a worker or probe file it never downloads installs cleanly and then fails at the first
/// dictation, which is the worst place to discover it.
fn validate(entry: &SourceEntry) -> Result<(), Box<dyn Error>> {
    let manifest = &entry.manifest;
    let label = &manifest.package_id;
    let mut total = 0_u64;
    for file in &manifest.files {
        if file.size == 0 {
            return Err(format!("{label}: file {} has zero size", file.path).into());
        }
        if file.sha256.len() != 64 || !file.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{label}: file {} has no sha256", file.path).into());
        }
        match file.download_uri.as_deref() {
            Some(uri) if uri.starts_with("https://") => {}
            _ => {
                return Err(
                    format!("{label}: file {} needs an https download_uri", file.path).into(),
                );
            }
        }
        total = total
            .checked_add(file.size)
            .ok_or_else(|| format!("{label}: total size overflow"))?;
    }
    if total != manifest.expected_size {
        return Err(format!(
            "{label}: expected_size {} does not match the {total} bytes it lists",
            manifest.expected_size
        )
        .into());
    }
    let has = |path: &str| manifest.files.iter().any(|file| file.path == path);
    if !has(&manifest.runtime.probe_file) {
        return Err(format!(
            "{label}: probe_file {} is not one of the downloaded files",
            manifest.runtime.probe_file
        )
        .into());
    }
    // The desktop maps exactly two runtime profiles. Anything else installs cleanly and then
    // fails at the first dictation with "unsupported device profile".
    let threads = manifest
        .runtime
        .settings
        .get("threads")
        .and_then(|value| value.as_u64());
    if !matches!(
        (manifest.runtime.device_kind.as_str(), threads),
        ("vulkan", Some(0)) | ("cpu", Some(16))
    ) {
        return Err(format!(
            "{label}: device_kind/threads must be vulkan/0 or cpu/16; the desktop maps no other profile"
        )
        .into());
    }
    if let Some(path) = manifest
        .runtime
        .settings
        .get("model_path")
        .and_then(|value| value.as_str())
        && !has(path)
    {
        return Err(format!(
            "{label}: runtime model_path {path} is not one of the downloaded files"
        )
        .into());
    }
    // The worker ships with the application, so a package either carries its own copy or names
    // exactly the bundled one. Any other name would ask the desktop to run an unknown binary.
    if let Some(path) = manifest
        .runtime
        .settings
        .get("worker_path")
        .and_then(|value| value.as_str())
        && !has(path)
        && path != BUNDLED_WORKER
    {
        return Err(format!(
            "{label}: runtime worker_path {path} is neither a downloaded file nor the bundled {BUNDLED_WORKER}"
        )
        .into());
    }
    Ok(())
}

fn read_private_key(path: &Path) -> Result<SigningKey, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let text = text.trim();
    if text.len() != 64 {
        return Err("private key file must hold 64 hex characters".into());
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| "private key file is not hex")?;
    }
    Ok(SigningKey::from_bytes(&bytes))
}

/// The repository is published, so a key stored inside it would be published with it.
fn reject_path_inside_working_tree(path: &Path) -> Result<(), Box<dyn Error>> {
    let working_tree = std::env::current_dir()?.canonicalize()?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = absolute
        .parent()
        .ok_or("private key path has no parent directory")?;
    let parent: PathBuf = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    if parent.starts_with(&working_tree) {
        return Err(format!(
            "refusing to use a signing key inside the working tree ({}); \
             keep it outside the repository",
            working_tree.display()
        )
        .into());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
