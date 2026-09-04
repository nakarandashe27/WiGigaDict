use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use wigigadict_storage::{
    ConfigurationRepository, ConfigurationUpdate, FileCompatibilityProbe, ManifestFile,
    ModelManager, ModelManifest, RuntimeManifest, SignedManifest, TrustedKeyRing,
};

const CONFIRMATION: &str = "provision-owner-m1";
const KEY_ID: &str = "wigigadict-m1-owner-local-v1";
const MODEL_NAME: &str = "ggml-large-v3-turbo-q5_0.bin";
const MODEL_SHA256: &str = "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2";
const MODEL_BYTES: u64 = 574_041_195;
const WORKER_NAME: &str = "wigigadict-asr-worker.exe";
const PACKAGE_ID: &str = "whisper-large-v3-turbo-q5-vulkan-m1";
const PROFILE_ID: &str = "whisper-large-v3-turbo-q5-vulkan";

fn main() {
    if let Err(error) = run() {
        eprintln!("golden-flow-provision: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 6 || arguments[0] != CONFIRMATION {
        return Err(format!(
            "usage: golden-flow-provision {CONFIRMATION} <database> <managed-root> <worker> <model> <hotkey>"
        )
        .into());
    }

    let database = absolute_path(&arguments[1])?;
    let managed_root = canonical_directory(&arguments[2])?;
    let database_parent = canonical_directory(
        database
            .parent()
            .ok_or("database path has no parent directory")?,
    )?;
    if !database_parent.starts_with(&managed_root) {
        return Err("database must stay under the managed root".into());
    }
    let worker = canonical_file(&arguments[3])?;
    let model = canonical_file(&arguments[4])?;
    let hotkey = &arguments[5];

    let worker_file = artifact(WORKER_NAME, &worker)?;
    let model_file = artifact(MODEL_NAME, &model)?;
    if model_file.size != MODEL_BYTES || model_file.sha256 != MODEL_SHA256 {
        return Err("model does not match the frozen M1 artifact".into());
    }

    let source = TemporarySource::new()?;
    source.materialize(WORKER_NAME, &worker)?;
    source.materialize(MODEL_NAME, &model)?;

    let signing = SigningKey::from_bytes(&[23_u8; 32]);
    let manifest = ModelManifest {
        schema_version: 1,
        package_id: PACKAGE_ID.into(),
        engine_family: "whisper".into(),
        model_name: "large-v3-turbo-q5".into(),
        model_version: "5359861c-q5_0".into(),
        release_sequence: 1,
        source_uri: "file://owner-local-benchmark".into(),
        license_id: "MIT-whisper.cpp-model-card".into(),
        expected_size: worker_file
            .size
            .checked_add(model_file.size)
            .ok_or("package size overflow")?,
        signature_key_id: KEY_ID.into(),
        minimum_manager_version: 1,
        expires_at_ms: 4_102_444_800_000,
        compatibility_abi: "wigigadict-model-abi-v1".into(),
        files: vec![worker_file, model_file],
        runtime: RuntimeManifest {
            profile_id: PROFILE_ID.into(),
            profile_version: 1,
            adapter_type: "transcribe-rs".into(),
            adapter_version: "0.3.11".into(),
            device_kind: "vulkan".into(),
            device_id: None,
            settings: json!({
                "worker_path": WORKER_NAME,
                "model_path": MODEL_NAME,
                "timeout_ms": 120_000,
                "threads": 0
            }),
            probe_file: WORKER_NAME.into(),
        },
    };
    let manifest_json = serde_json::to_vec(&manifest)?;
    let signature = signing.sign(&manifest_json).to_bytes();
    let signed = SignedManifest {
        manifest_json,
        signature_hex: signature.iter().map(|byte| format!("{byte:02x}")).collect(),
    };
    let mut keys = TrustedKeyRing::new();
    keys.insert(KEY_ID, signing.verifying_key().to_bytes())?;
    let mut manager = ModelManager::open(&database, &managed_root, keys)?;
    let now = now_ms()?;
    let preview = manager.preview(&signed, now)?;
    if preview.package_id != PACKAGE_ID
        || preview.expected_size != manifest.expected_size
        || preview.license_id != manifest.license_id
    {
        return Err("verified preview differs from the frozen package".into());
    }
    let receipt = manager.install_offline(&signed, source.path(), &FileCompatibilityProbe)?;

    let mut configurations = ConfigurationRepository::open(&database)?;
    let current = configurations
        .active()?
        .ok_or("active configuration is missing after runtime activation")?;
    let updated = configurations.update(
        &ConfigurationUpdate {
            expected_config_version: current.config_version,
            hotkey_binding: hotkey.clone(),
            microphone_device_id: current.microphone_device_id,
            active_runtime_profile_id: Some(receipt.profile_id.clone()),
            active_cleanup_profile_id: current.active_cleanup_profile_id,
            startup_enabled: current.startup_enabled,
            warmup_enabled: current.warmup_enabled,
            diagnostic_mode: current.diagnostic_mode,
            archive_directory: current.archive_directory,
        },
        now_ms()?,
    )?;

    println!("package_id={}", receipt.package_id);
    println!("profile_id={}", receipt.active_profile_id);
    println!("manifest_sha256={}", receipt.manifest_sha256);
    println!("worker_sha256={}", manifest.files[0].sha256);
    println!("model_sha256={}", manifest.files[1].sha256);
    println!("hotkey={}", updated.hotkey_binding);
    Ok(())
}

fn absolute_path(value: &str) -> Result<PathBuf, Box<dyn Error>> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("all provisioning paths must be absolute".into());
    }
    Ok(path)
}

fn canonical_directory(value: impl AsRef<Path>) -> Result<PathBuf, Box<dyn Error>> {
    let path = value.as_ref();
    if !path.is_absolute() || !path.is_dir() {
        return Err("managed paths must be existing absolute directories".into());
    }
    Ok(path.canonicalize()?)
}

fn canonical_file(value: &str) -> Result<PathBuf, Box<dyn Error>> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || !path.is_file() {
        return Err("artifact paths must be existing absolute files".into());
    }
    Ok(path.canonicalize()?)
}

fn artifact(name: &str, source: &Path) -> Result<ManifestFile, Box<dyn Error>> {
    let size = source.metadata()?.len();
    Ok(ManifestFile {
        path: name.into(),
        size,
        sha256: sha256_file(source)?,
        download_uri: None,
    })
}

fn sha256_file(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn now_ms() -> Result<i64, Box<dyn Error>> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

struct TemporarySource {
    path: PathBuf,
}

impl TemporarySource {
    fn new() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "wigigadict-golden-flow-provision-{}-{}",
            std::process::id(),
            now_ms()?
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn materialize(&self, name: &str, source: &Path) -> Result<(), Box<dyn Error>> {
        let destination = self.path.join(name);
        if fs::hard_link(source, &destination).is_err() {
            fs::copy(source, destination)?;
        }
        Ok(())
    }
}

impl Drop for TemporarySource {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
