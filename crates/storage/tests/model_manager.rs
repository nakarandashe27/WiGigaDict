#![allow(linker_messages)]
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use wigigadict_storage::{
    DiskSpace, DownloadObserver, IgnoreProgress, ManifestFile, ModelManager, ModelManagerError,
    ModelManifest, RangeDownloader, RuntimeManifest, RuntimeProbe, SignedManifest, TrustedKeyRing,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct FixedDisk(u64);

impl DiskSpace for FixedDisk {
    fn available_bytes(&self, _path: &Path) -> Result<u64, ModelManagerError> {
        Ok(self.0)
    }
}

struct Probe(Result<(), &'static str>);

impl RuntimeProbe for Probe {
    fn probe(&self, root: &Path, manifest: &ModelManifest) -> Result<(), String> {
        self.0
            .as_ref()
            .map(|_| ())
            .map_err(|code| (*code).to_owned())?;
        if root.join(&manifest.runtime.probe_file).is_file() {
            Ok(())
        } else {
            Err("probe_file_missing".into())
        }
    }
}

struct FakeDownloader {
    bytes: Vec<u8>,
    chunk: usize,
    offsets: Mutex<Vec<u64>>,
}

impl FakeDownloader {
    fn new(bytes: &[u8], chunk: usize) -> Self {
        Self {
            bytes: bytes.to_vec(),
            chunk,
            offsets: Mutex::new(Vec::new()),
        }
    }
}

impl RangeDownloader for FakeDownloader {
    fn download(
        &self,
        _uri: &str,
        offset: u64,
        expected_size: u64,
        destination: &mut File,
        observer: &dyn DownloadObserver,
    ) -> Result<(), ModelManagerError> {
        self.offsets.lock().unwrap().push(offset);
        assert_eq!(expected_size, self.bytes.len() as u64);
        destination.seek(SeekFrom::Start(offset))?;
        // Chunked like the real downloader so a cancel can land mid-file.
        for chunk in self.bytes[offset as usize..].chunks(self.chunk) {
            destination.write_all(chunk)?;
            if !observer.advanced(chunk.len() as u64) {
                destination.sync_all()?;
                return Err(ModelManagerError::Cancelled);
            }
        }
        destination.sync_all()?;
        Ok(())
    }
}

/// Cancels once the download has appended `limit` bytes.
struct CancelAfter {
    limit: u64,
    seen: Mutex<u64>,
}

impl DownloadObserver for CancelAfter {
    fn advanced(&self, bytes: u64) -> bool {
        let mut seen = self.seen.lock().unwrap();
        *seen += bytes;
        *seen < self.limit
    }
}

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    signing: SigningKey,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "wigigadict-step9-{label}-{}-{unique}",
            std::process::id()
        ));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        Self {
            root,
            source,
            signing: SigningKey::from_bytes(&[7_u8; 32]),
        }
    }

    fn manager(&self) -> ModelManager {
        let mut keys = TrustedKeyRing::new();
        keys.insert("test-key", self.signing.verifying_key().to_bytes())
            .unwrap();
        ModelManager::open_in_memory(self.root.join("managed"), keys).unwrap()
    }

    fn write(&self, bytes: &[u8]) {
        fs::write(self.source.join("model.bin"), bytes).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.root.exists() {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn manifest(
    package: &str,
    profile: &str,
    release: u64,
    bytes: &[u8],
    online: bool,
) -> ModelManifest {
    ModelManifest {
        schema_version: 1,
        package_id: package.into(),
        engine_family: "whisper.cpp".into(),
        model_name: "tiny-fixture".into(),
        model_version: format!("0.0.{release}"),
        release_sequence: release,
        source_uri: if online {
            "https://models.example.test/tiny/".into()
        } else {
            "file://explicit-offline-import".into()
        },
        license_id: "MIT-fixture-only".into(),
        expected_size: bytes.len() as u64,
        signature_key_id: "test-key".into(),
        minimum_manager_version: 1,
        expires_at_ms: 10_000,
        compatibility_abi: "wigigadict-model-abi-v1".into(),
        files: vec![ManifestFile {
            path: "model.bin".into(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
            download_uri: online.then(|| "https://models.example.test/tiny/model.bin".into()),
        }],
        runtime: RuntimeManifest {
            profile_id: profile.into(),
            profile_version: 1,
            adapter_type: "whisper.cpp".into(),
            adapter_version: "fixture-adapter-1".into(),
            device_kind: "cpu".into(),
            device_id: None,
            settings: json!({"language": "auto"}),
            probe_file: "model.bin".into(),
        },
    }
}

fn signed(manifest: &ModelManifest, key: &SigningKey) -> SignedManifest {
    let manifest_json = serde_json::to_vec(manifest).unwrap();
    let signature = key.sign(&manifest_json).to_bytes();
    SignedManifest {
        manifest_json,
        signature_hex: signature.iter().map(|byte| format!("{byte:02x}")).collect(),
    }
}

#[test]
fn offline_install_previews_then_activates_after_probe() {
    let fixture = Fixture::new("offline");
    let bytes = b"not-a-real-model";
    fixture.write(bytes);
    let manifest = manifest("package-1", "profile-1", 1, bytes, false);
    let signed = signed(&manifest, &fixture.signing);
    let mut manager = fixture.manager();

    let preview = manager.preview(&signed, 100).unwrap();
    assert_eq!(preview.license_id, "MIT-fixture-only");
    assert_eq!(manager.active_profile_id().unwrap(), None);
    let receipt = manager
        .install_offline_with(
            &signed,
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            100,
        )
        .unwrap();
    assert_eq!(receipt.active_profile_id, "profile-1");
    assert_eq!(
        fs::read(receipt.installed_root.join("model.bin")).unwrap(),
        bytes
    );
}

#[test]
fn online_install_resumes_partial_without_network() {
    let fixture = Fixture::new("online");
    let bytes = b"resumable-fixture";
    let manifest = manifest("package-online", "profile-online", 1, bytes, true);
    let signed = signed(&manifest, &fixture.signing);
    let mut manager = fixture.manager();
    let partial = fixture.root.join("managed/staging/package-online");
    fs::create_dir(&partial).unwrap();
    fs::write(partial.join("model.bin"), &bytes[..5]).unwrap();
    let downloader = FakeDownloader::new(bytes, bytes.len());

    manager
        .install_online_with(
            &signed,
            &downloader,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            100,
            &IgnoreProgress,
        )
        .unwrap();
    assert_eq!(*downloader.offsets.lock().unwrap(), vec![5]);
}

#[test]
fn cancelled_download_stays_resumable_and_is_not_a_failure() {
    let fixture = Fixture::new("cancel");
    let bytes = b"resumable-fixture";
    let manifest = manifest("package-cancel", "profile-cancel", 1, bytes, true);
    let signed = signed(&manifest, &fixture.signing);
    let mut manager = fixture.manager();
    let downloader = FakeDownloader::new(bytes, 4);

    let cancelled = manager.install_online_with(
        &signed,
        &downloader,
        &FixedDisk(u64::MAX),
        &Probe(Ok(())),
        100,
        &CancelAfter {
            limit: 8,
            seen: Mutex::new(0),
        },
    );
    assert!(matches!(cancelled, Err(ModelManagerError::Cancelled)));

    let partial = fixture
        .root
        .join("managed/staging/package-cancel/model.bin");
    assert_eq!(fs::metadata(&partial).unwrap().len(), 8);
    let listed = manager.list_packages().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].package_id, "package-cancel");
    assert_eq!(listed[0].install_state, "downloading");
    assert_eq!(listed[0].job_state.as_deref(), Some("cancelled"));
    assert_eq!(listed[0].profile_id, None);
    assert!(!listed[0].is_active);

    manager
        .install_online_with(
            &signed,
            &downloader,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            200,
            &IgnoreProgress,
        )
        .unwrap();
    assert_eq!(*downloader.offsets.lock().unwrap(), vec![0, 8]);
    let listed = manager.list_packages().unwrap();
    assert_eq!(listed[0].install_state, "installed");
    assert_eq!(listed[0].profile_id.as_deref(), Some("profile-cancel"));
    assert!(listed[0].is_active);
}

#[test]
fn removal_frees_managed_bytes_but_never_the_active_package() {
    let fixture = Fixture::new("remove");
    let bytes = b"not-a-real-model";
    fixture.write(bytes);
    let first = manifest("package-old", "profile-old", 1, bytes, false);
    let second = manifest("package-new", "profile-new", 2, bytes, false);
    let mut manager = fixture.manager();
    let receipt = manager
        .install_offline_with(
            &signed(&first, &fixture.signing),
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            100,
        )
        .unwrap();

    assert!(matches!(
        manager.remove_package("package-old", 150),
        Err(ModelManagerError::Conflict(_))
    ));
    assert!(receipt.installed_root.is_dir());

    let second_receipt = manager
        .install_offline_with(
            &signed(&second, &fixture.signing),
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            200,
        )
        .unwrap();
    // Installing is not switching: the second package must not take the model in use away from
    // the first one just because it finished downloading.
    assert_eq!(second_receipt.active_profile_id, "profile-old");
    assert!(matches!(
        manager.remove_package("package-old", 250),
        Err(ModelManagerError::Conflict(_))
    ));

    manager.activate_profile("profile-new", 260).unwrap();
    manager.remove_package("package-old", 300).unwrap();

    assert!(!receipt.installed_root.exists());
    let listed = manager.list_packages().unwrap();
    assert_eq!(listed.len(), 2);
    let old = listed
        .iter()
        .find(|row| row.package_id == "package-old")
        .expect("removed package keeps its identity");
    assert_eq!(old.install_state, "absent");
    assert_eq!(old.installed_at, None);
    assert_eq!(old.health_state.as_deref(), Some("unknown"));
    assert!(!old.is_active);
    let new = listed
        .iter()
        .find(|row| row.package_id == "package-new")
        .expect("surviving package");
    assert_eq!(new.install_state, "installed");
    assert!(new.is_active);
}

#[test]
fn signature_traversal_and_abi_rejections_are_fail_closed() {
    let fixture = Fixture::new("rejections");
    let bytes = b"fixture";
    let base = manifest("package-base", "profile-base", 1, bytes, false);
    let mut revoked = TrustedKeyRing::new();
    revoked
        .insert("test-key", fixture.signing.verifying_key().to_bytes())
        .unwrap();
    revoked.revoke("test-key");
    let revoked_manager =
        ModelManager::open_in_memory(fixture.root.join("revoked"), revoked).unwrap();
    assert!(matches!(
        revoked_manager.preview(&signed(&base, &fixture.signing), 100),
        Err(ModelManagerError::RevokedSigningKey(_))
    ));

    let mut invalid = signed(&base, &fixture.signing);
    invalid.signature_hex.replace_range(0..2, "00");
    let manager = fixture.manager();
    assert!(matches!(
        manager.preview(&invalid, 100),
        Err(ModelManagerError::InvalidSignature)
    ));

    let mut traversal = base.clone();
    traversal.files[0].path = "../escape.bin".into();
    traversal.runtime.probe_file = "../escape.bin".into();
    assert!(matches!(
        manager.preview(&signed(&traversal, &fixture.signing), 100),
        Err(ModelManagerError::PathRejected(_))
    ));
    assert!(!fixture.root.join("escape.bin").exists());

    let mut abi = base;
    abi.compatibility_abi = "future-abi".into();
    assert!(matches!(
        manager.preview(&signed(&abi, &fixture.signing), 100),
        Err(ModelManagerError::IncompatibleAbi(_))
    ));
}

#[test]
fn failed_candidates_never_replace_last_known_good() {
    let fixture = Fixture::new("lkg");
    let good = b"known-good";
    fixture.write(good);
    let mut manager = fixture.manager();
    manager
        .install_offline_with(
            &signed(
                &manifest("package-good", "profile-good", 10, good, false),
                &fixture.signing,
            ),
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            100,
        )
        .unwrap();

    let candidate = b"candidate";
    let disk = signed(
        &manifest("package-disk", "profile-disk", 11, candidate, false),
        &fixture.signing,
    );
    assert!(matches!(
        manager.install_offline_with(&disk, &fixture.source, &FixedDisk(0), &Probe(Ok(())), 200,),
        Err(ModelManagerError::InsufficientDisk { .. })
    ));

    fixture.write(b"corrupt!");
    let corrupt = signed(
        &manifest("package-corrupt", "profile-corrupt", 12, candidate, false),
        &fixture.signing,
    );
    assert!(matches!(
        manager.install_offline_with(
            &corrupt,
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            250,
        ),
        Err(ModelManagerError::CorruptArtifact(_))
    ));
    fixture.write(candidate);

    let probe = signed(
        &manifest("package-probe", "profile-probe", 13, candidate, false),
        &fixture.signing,
    );
    assert!(matches!(
        manager.install_offline_with(
            &probe,
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Err("adapter_probe_failed")),
            300,
        ),
        Err(ModelManagerError::ProbeFailed(_))
    ));
    assert_eq!(
        manager.active_profile_id().unwrap().as_deref(),
        Some("profile-good")
    );
}

#[test]
fn downgrade_does_not_replace_current_profile() {
    let fixture = Fixture::new("downgrade");
    let bytes = b"release";
    fixture.write(bytes);
    let mut manager = fixture.manager();
    let current = manifest("package-current", "profile-current", 20, bytes, false);
    manager
        .install_offline_with(
            &signed(&current, &fixture.signing),
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            100,
        )
        .unwrap();
    let older = manifest("package-older", "profile-older", 19, bytes, false);
    assert!(matches!(
        manager.install_offline_with(
            &signed(&older, &fixture.signing),
            &fixture.source,
            &FixedDisk(u64::MAX),
            &Probe(Ok(())),
            200,
        ),
        Err(ModelManagerError::DowngradeRejected { .. })
    ));
    assert_eq!(
        manager.active_profile_id().unwrap().as_deref(),
        Some("profile-current")
    );
}
