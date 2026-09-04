use crate::{Database, StorageError};
use ed25519_dalek::{Signature, VerifyingKey};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::RANGE;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MODEL_MANAGER_VERSION: u32 = 1;
pub const SUPPORTED_MODEL_ABI: &str = "wigigadict-model-abi-v1";
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 256;
const DISK_HEADROOM_BYTES: u64 = 64 * 1024 * 1024;
const DOWNLOAD_CHUNK_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub package_id: String,
    pub engine_family: String,
    pub model_name: String,
    pub model_version: String,
    pub release_sequence: u64,
    pub source_uri: String,
    pub license_id: String,
    pub expected_size: u64,
    pub signature_key_id: String,
    pub minimum_manager_version: u32,
    pub expires_at_ms: i64,
    pub compatibility_abi: String,
    pub files: Vec<ManifestFile>,
    pub runtime: RuntimeManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub download_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub profile_id: String,
    pub profile_version: u32,
    pub adapter_type: String,
    pub adapter_version: String,
    pub device_kind: String,
    pub device_id: Option<String>,
    pub settings: Value,
    pub probe_file: String,
}

#[derive(Debug, Clone)]
pub struct SignedManifest {
    pub manifest_json: Vec<u8>,
    pub signature_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreview {
    pub package_id: String,
    pub model_name: String,
    pub model_version: String,
    pub source_uri: String,
    pub license_id: String,
    pub expected_size: u64,
    pub manifest_sha256: String,
    pub signature_key_id: String,
    pub compatibility_abi: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInstallReceipt {
    pub package_id: String,
    pub profile_id: String,
    pub install_job_id: String,
    pub installed_root: PathBuf,
    pub manifest_sha256: String,
    pub active_profile_id: String,
}

/// One row of the model screen: a package, the runtime profile it installed (if it got that far)
/// and the progress of its most recent install job. Catalog-only metadata such as languages or
/// hardware requirements is not stored here - it comes from the signed catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub package_id: String,
    pub engine_family: String,
    pub model_name: String,
    pub model_version: String,
    pub license_id: String,
    pub expected_size: u64,
    pub install_state: String,
    pub installed_at: Option<i64>,
    pub profile_id: Option<String>,
    pub device_kind: Option<String>,
    pub health_state: Option<String>,
    pub is_active: bool,
    pub job_state: Option<String>,
    pub bytes_downloaded: u64,
}

#[derive(Debug)]
pub enum ModelManagerError {
    InvalidManifest(String),
    UnknownSigningKey(String),
    RevokedSigningKey(String),
    InvalidSignature,
    DowngradeRejected { found: u64, floor: u64 },
    IncompatibleAbi(String),
    PathRejected(String),
    InsufficientDisk { required: u64, available: u64 },
    CorruptArtifact(String),
    ProbeFailed(String),
    Conflict(String),
    Network(String),
    Cancelled,
    Io(std::io::Error),
    Storage(StorageError),
    Sqlite(rusqlite::Error),
    ClockBeforeUnixEpoch,
}

impl Display for ModelManagerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidManifest(detail) => write!(formatter, "invalid model manifest: {detail}"),
            Self::UnknownSigningKey(key) => write!(formatter, "unknown model signing key: {key}"),
            Self::RevokedSigningKey(key) => write!(formatter, "revoked model signing key: {key}"),
            Self::InvalidSignature => write!(formatter, "model manifest signature is invalid"),
            Self::DowngradeRejected { found, floor } => {
                write!(
                    formatter,
                    "release sequence {found} is below installed floor {floor}"
                )
            }
            Self::IncompatibleAbi(abi) => write!(formatter, "unsupported model ABI: {abi}"),
            Self::PathRejected(path) => write!(formatter, "model path is rejected: {path}"),
            Self::InsufficientDisk {
                required,
                available,
            } => write!(
                formatter,
                "insufficient disk space: required {required} bytes, available {available}"
            ),
            Self::CorruptArtifact(path) => write!(formatter, "corrupt model artifact: {path}"),
            Self::ProbeFailed(code) => {
                write!(formatter, "runtime compatibility probe failed: {code}")
            }
            Self::Conflict(detail) => write!(formatter, "model manager conflict: {detail}"),
            Self::Network(detail) => write!(formatter, "model download failed: {detail}"),
            Self::Cancelled => formatter.write_str("model download was cancelled"),
            Self::Io(error) => write!(formatter, "model manager I/O error: {error}"),
            Self::Storage(error) => write!(formatter, "model manager storage error: {error}"),
            Self::Sqlite(error) => write!(formatter, "model manager SQLite error: {error}"),
            Self::ClockBeforeUnixEpoch => write!(formatter, "system clock is before Unix epoch"),
        }
    }
}

impl std::error::Error for ModelManagerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ModelManagerError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<StorageError> for ModelManagerError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<rusqlite::Error> for ModelManagerError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub type ModelManagerResult<T> = Result<T, ModelManagerError>;

#[derive(Clone, Default)]
pub struct TrustedKeyRing {
    keys: BTreeMap<String, VerifyingKey>,
    revoked: BTreeSet<String>,
}

impl TrustedKeyRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key_id: impl Into<String>, bytes: [u8; 32]) -> ModelManagerResult<()> {
        let key_id = key_id.into();
        validate_token("signature_key_id", &key_id)?;
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|_| ModelManagerError::InvalidManifest("invalid Ed25519 public key".into()))?;
        if key.is_weak() {
            return Err(ModelManagerError::InvalidManifest(
                "weak Ed25519 public key is forbidden".into(),
            ));
        }
        self.keys.insert(key_id, key);
        Ok(())
    }

    pub fn revoke(&mut self, key_id: impl Into<String>) {
        self.revoked.insert(key_id.into());
    }

    fn verify(&self, signed: &SignedManifest, key_id: &str) -> ModelManagerResult<()> {
        self.verify_bytes(&signed.manifest_json, &signed.signature_hex, key_id)
    }

    /// Verifies a detached signature over arbitrary signed bytes, so the model catalog reuses
    /// exactly the key ring, revocation list and strict Ed25519 check that manifests use.
    pub(crate) fn verify_bytes(
        &self,
        bytes: &[u8],
        signature_hex: &str,
        key_id: &str,
    ) -> ModelManagerResult<()> {
        if self.revoked.contains(key_id) {
            return Err(ModelManagerError::RevokedSigningKey(key_id.into()));
        }
        let key = self
            .keys
            .get(key_id)
            .ok_or_else(|| ModelManagerError::UnknownSigningKey(key_id.into()))?;
        let signature =
            decode_hex::<64>(signature_hex).map_err(|_| ModelManagerError::InvalidSignature)?;
        let signature = Signature::from_bytes(&signature);
        key.verify_strict(bytes, &signature)
            .map_err(|_| ModelManagerError::InvalidSignature)
    }
}

pub trait DiskSpace {
    fn available_bytes(&self, path: &Path) -> ModelManagerResult<u64>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemDiskSpace;

impl DiskSpace for SystemDiskSpace {
    fn available_bytes(&self, path: &Path) -> ModelManagerResult<u64> {
        available_disk_bytes(path)
    }
}

/// Carries download progress out of the byte loop and the cancel decision back into it.
pub trait DownloadObserver {
    /// Receives the bytes appended since the previous call. Returning `false` cancels the
    /// download and keeps the partial file so a later install resumes from it.
    fn advanced(&self, bytes: u64) -> bool;
}

/// Observer for callers that neither report progress nor cancel.
#[derive(Debug, Clone, Copy, Default)]
pub struct IgnoreProgress;

impl DownloadObserver for IgnoreProgress {
    fn advanced(&self, _bytes: u64) -> bool {
        true
    }
}

pub trait RangeDownloader {
    fn download(
        &self,
        uri: &str,
        offset: u64,
        expected_size: u64,
        destination: &mut File,
        observer: &dyn DownloadObserver,
    ) -> ModelManagerResult<()>;
}

#[derive(Clone)]
pub struct ReqwestRangeDownloader {
    client: Client,
}

impl ReqwestRangeDownloader {
    pub fn new() -> ModelManagerResult<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(30 * 60))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|error| ModelManagerError::Network(error.to_string()))?;
        Ok(Self { client })
    }
}

impl RangeDownloader for ReqwestRangeDownloader {
    fn download(
        &self,
        uri: &str,
        offset: u64,
        expected_size: u64,
        destination: &mut File,
        observer: &dyn DownloadObserver,
    ) -> ModelManagerResult<()> {
        validate_https_uri(uri)?;
        let mut request = self.client.get(uri);
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let response = request
            .send()
            .map_err(|error| ModelManagerError::Network(error.to_string()))?;
        if response.url().scheme() != "https" {
            return Err(ModelManagerError::Network(
                "download redirected away from HTTPS".into(),
            ));
        }
        let write_offset = match (offset, response.status()) {
            (0, status) if status.is_success() => {
                destination.set_len(0)?;
                destination.seek(SeekFrom::Start(0))?;
                0
            }
            (_, StatusCode::PARTIAL_CONTENT) => {
                destination.seek(SeekFrom::Start(offset))?;
                offset
            }
            (_, StatusCode::OK) => {
                destination.set_len(0)?;
                destination.seek(SeekFrom::Start(0))?;
                0
            }
            (_, status) => {
                return Err(ModelManagerError::Network(format!(
                    "unexpected HTTP status {status}"
                )));
            }
        };
        let remaining = expected_size.checked_sub(write_offset).ok_or_else(|| {
            ModelManagerError::CorruptArtifact("partial download exceeds manifest size".into())
        })?;
        let written = copy_observed(
            &mut response.take(remaining + 1),
            destination,
            remaining,
            observer,
        )?;
        if written > remaining {
            destination.set_len(expected_size)?;
            return Err(ModelManagerError::CorruptArtifact(
                "download exceeded manifest size".into(),
            ));
        }
        destination.sync_all()?;
        Ok(())
    }
}

pub trait RuntimeProbe {
    fn probe(&self, package_root: &Path, manifest: &ModelManifest) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FileCompatibilityProbe;

impl RuntimeProbe for FileCompatibilityProbe {
    fn probe(&self, package_root: &Path, manifest: &ModelManifest) -> Result<(), String> {
        let probe = checked_join(package_root, &manifest.runtime.probe_file)
            .map_err(|error| error.to_string())?;
        let metadata = fs::metadata(&probe).map_err(|_| "probe_file_missing".to_owned())?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err("probe_file_empty".into());
        }
        let mut file = File::open(probe).map_err(|_| "probe_file_unreadable".to_owned())?;
        let mut byte = [0_u8; 1];
        file.read_exact(&mut byte)
            .map_err(|_| "probe_file_unreadable".to_owned())
    }
}

pub struct ModelManager {
    database: Database,
    root: PathBuf,
    trusted_keys: TrustedKeyRing,
}

impl ModelManager {
    pub fn open(
        database_path: impl AsRef<Path>,
        managed_root: impl AsRef<Path>,
        trusted_keys: TrustedKeyRing,
    ) -> ModelManagerResult<Self> {
        let database = Database::open(database_path)?;
        Self::from_database(database, managed_root, trusted_keys)
    }

    pub fn open_in_memory(
        managed_root: impl AsRef<Path>,
        trusted_keys: TrustedKeyRing,
    ) -> ModelManagerResult<Self> {
        let database = Database::open_in_memory()?;
        Self::from_database(database, managed_root, trusted_keys)
    }

    fn from_database(
        database: Database,
        managed_root: impl AsRef<Path>,
        trusted_keys: TrustedKeyRing,
    ) -> ModelManagerResult<Self> {
        let root = managed_root.as_ref().to_path_buf();
        prepare_managed_root(&root)?;
        Ok(Self {
            database,
            root,
            trusted_keys,
        })
    }

    pub fn preview(
        &self,
        signed: &SignedManifest,
        now_ms: i64,
    ) -> ModelManagerResult<ModelPreview> {
        let (manifest, manifest_sha256) = self.verify_manifest(signed, now_ms)?;
        Ok(ModelPreview {
            package_id: manifest.package_id,
            model_name: manifest.model_name,
            model_version: manifest.model_version,
            source_uri: manifest.source_uri,
            license_id: manifest.license_id,
            expected_size: manifest.expected_size,
            manifest_sha256,
            signature_key_id: manifest.signature_key_id,
            compatibility_abi: manifest.compatibility_abi,
        })
    }
}

impl ModelManager {
    pub fn active_profile_id(&self) -> ModelManagerResult<Option<String>> {
        Ok(self
            .database
            .connection
            .query_row(
                "SELECT active_runtime_profile_id FROM app_configuration WHERE is_active=1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Every known package, installed or not, so the model screen can show a finished install and
    /// an interrupted download in the same list.
    pub fn list_packages(&self) -> ModelManagerResult<Vec<InstalledModel>> {
        let mut statement = self.database.connection.prepare(
            "SELECT m.id,m.engine_family,m.model_name,m.model_version,m.license_id,
                    m.expected_size,m.install_state,m.installed_at,
                    p.id,p.device_kind,p.health_state,
                    COALESCE(c.active_runtime_profile_id=p.id,0),
                    j.state,COALESCE(j.bytes_downloaded,0)
             FROM model_package m
             LEFT JOIN runtime_profile p ON p.model_package_id=m.id
             LEFT JOIN app_configuration c ON c.is_active=1
             LEFT JOIN model_install_job j ON j.id=(
                 SELECT id FROM model_install_job
                 WHERE model_package_id=m.id
                 ORDER BY updated_at DESC,id DESC LIMIT 1
             )
             ORDER BY m.model_name,m.model_version,p.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(InstalledModel {
                package_id: row.get(0)?,
                engine_family: row.get(1)?,
                model_name: row.get(2)?,
                model_version: row.get(3)?,
                license_id: row.get(4)?,
                expected_size: row.get::<_, i64>(5)?.max(0) as u64,
                install_state: row.get(6)?,
                installed_at: row.get(7)?,
                profile_id: row.get(8)?,
                device_kind: row.get(9)?,
                health_state: row.get(10)?,
                is_active: row.get::<_, i64>(11)? != 0,
                job_state: row.get(12)?,
                bytes_downloaded: row.get::<_, i64>(13)?.max(0) as u64,
            })
        })?;
        let mut packages = Vec::new();
        for row in rows {
            packages.push(row?);
        }
        Ok(packages)
    }

    /// Frees the managed bytes of a package without erasing its identity: the row becomes
    /// `absent` and its profiles are disabled, so recognition history keeps resolving and the
    /// immutable configuration snapshots stay valid. Refuses to touch the active package or one
    /// with a running install.
    pub fn remove_package(&mut self, package_id: &str, observed_at: i64) -> ModelManagerResult<()> {
        validate_token("package_id", package_id)?;
        let active: Option<i64> = self
            .database
            .connection
            .query_row(
                "SELECT 1 FROM app_configuration c
                 JOIN runtime_profile p ON p.id=c.active_runtime_profile_id
                 WHERE c.is_active=1 AND p.model_package_id=?1",
                [package_id],
                |row| row.get(0),
            )
            .optional()?;
        if active.is_some() {
            return Err(ModelManagerError::Conflict(
                "the active package cannot be removed; activate another profile first".into(),
            ));
        }
        let running: Option<String> = self
            .database
            .connection
            .query_row(
                "SELECT id FROM model_install_job
                 WHERE model_package_id=?1
                   AND state IN ('queued','downloading','verifying','installing')",
                [package_id],
                |row| row.get(0),
            )
            .optional()?;
        if running.is_some() {
            return Err(ModelManagerError::Conflict(
                "cancel the running install before removing this package".into(),
            ));
        }

        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let known = transaction.execute(
            "UPDATE model_package
             SET install_state='absent',installed_at=NULL,updated_at=?2
             WHERE id=?1",
            params![package_id, observed_at],
        )?;
        if known == 0 {
            return Err(ModelManagerError::Conflict("unknown package".into()));
        }
        transaction.execute(
            "UPDATE runtime_profile
             SET enabled=0,health_state='unknown',last_health_at=NULL,updated_at=?2
             WHERE model_package_id=?1",
            params![package_id, observed_at],
        )?;
        transaction.commit()?;

        // ponytail: state first, then bytes. A crash in between leaks a directory instead of
        // leaving a package that claims to be installed with no weights behind it; reinstalling
        // the same package verifies that directory against the manifest and reuses it.
        for root in [
            self.installed_root(package_id)?,
            self.staging_root(package_id)?,
        ] {
            remove_managed_directory(&root)?;
        }
        Ok(())
    }

    pub fn install_offline(
        &mut self,
        signed: &SignedManifest,
        source_root: impl AsRef<Path>,
        probe: &dyn RuntimeProbe,
    ) -> ModelManagerResult<ModelInstallReceipt> {
        self.install_offline_with(signed, source_root, &SystemDiskSpace, probe, now_ms()?)
    }

    pub fn install_offline_with(
        &mut self,
        signed: &SignedManifest,
        source_root: impl AsRef<Path>,
        disk: &dyn DiskSpace,
        probe: &dyn RuntimeProbe,
        observed_at: i64,
    ) -> ModelManagerResult<ModelInstallReceipt> {
        let source_root = canonical_directory(source_root.as_ref())?;
        self.install_with(signed, disk, probe, observed_at, |file, destination| {
            copy_offline_file(&source_root, file, destination)
        })
    }

    pub fn install_online(
        &mut self,
        signed: &SignedManifest,
        probe: &dyn RuntimeProbe,
    ) -> ModelManagerResult<ModelInstallReceipt> {
        let downloader = ReqwestRangeDownloader::new()?;
        self.install_online_with(
            signed,
            &downloader,
            &SystemDiskSpace,
            probe,
            now_ms()?,
            &IgnoreProgress,
        )
    }

    pub fn install_online_with(
        &mut self,
        signed: &SignedManifest,
        downloader: &dyn RangeDownloader,
        disk: &dyn DiskSpace,
        probe: &dyn RuntimeProbe,
        observed_at: i64,
        observer: &dyn DownloadObserver,
    ) -> ModelManagerResult<ModelInstallReceipt> {
        self.install_with(signed, disk, probe, observed_at, |file, destination| {
            download_file(downloader, file, destination, observer)
        })
    }

    pub fn activate_profile(
        &mut self,
        profile_id: &str,
        observed_at: i64,
    ) -> ModelManagerResult<String> {
        validate_token("profile_id", profile_id)?;
        let ready: Option<String> = self
            .database
            .connection
            .query_row(
                "SELECT p.id
                 FROM runtime_profile p
                 JOIN model_package m ON m.id=p.model_package_id
                 WHERE p.id=?1 AND p.health_state='healthy' AND p.enabled=1
                   AND m.install_state='installed'",
                [profile_id],
                |row| row.get(0),
            )
            .optional()?;
        if ready.is_none() {
            return Err(ModelManagerError::Conflict(
                "only an enabled healthy profile from an installed package can be activated".into(),
            ));
        }
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        activate_configuration(&transaction, profile_id, observed_at)?;
        transaction.commit()?;
        Ok(profile_id.into())
    }

    pub fn probe_profile(
        &mut self,
        profile_id: &str,
        probe: &dyn RuntimeProbe,
        observed_at: i64,
    ) -> ModelManagerResult<()> {
        validate_token("profile_id", profile_id)?;
        let package_id: String = self.database.connection.query_row(
            "SELECT model_package_id FROM runtime_profile WHERE id=?1",
            [profile_id],
            |row| row.get(0),
        )?;
        let installed_root = self.installed_root(&package_id)?;
        let manifest_bytes = fs::read(installed_root.join(".wigigadict-manifest.json"))?;
        let manifest: ModelManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))?;
        let result = probe.probe(&installed_root, &manifest);
        let (health, code) = match result {
            Ok(()) => ("healthy", None),
            Err(code) => ("unhealthy", Some(code)),
        };
        self.database.connection.execute(
            "UPDATE runtime_profile
             SET health_state=?2,last_health_at=?3,updated_at=?3
             WHERE id=?1",
            params![profile_id, health, observed_at],
        )?;
        if let Some(code) = code {
            return Err(ModelManagerError::ProbeFailed(code));
        }
        Ok(())
    }

    fn install_with<F>(
        &mut self,
        signed: &SignedManifest,
        disk: &dyn DiskSpace,
        probe: &dyn RuntimeProbe,
        observed_at: i64,
        mut materialize: F,
    ) -> ModelManagerResult<ModelInstallReceipt>
    where
        F: FnMut(&ManifestFile, &Path) -> ModelManagerResult<()>,
    {
        let (manifest, manifest_sha256) = self.verify_manifest(signed, observed_at)?;
        self.reject_downgrade(&manifest)?;
        let required = manifest
            .expected_size
            .checked_add(DISK_HEADROOM_BYTES)
            .ok_or_else(|| ModelManagerError::InvalidManifest("size overflow".into()))?;
        let available = disk.available_bytes(&self.root)?;
        if available < required {
            return Err(ModelManagerError::InsufficientDisk {
                required,
                available,
            });
        }

        let (job_id, staging_root) =
            self.begin_install(&manifest, &manifest_sha256, observed_at)?;
        let installed_root = self.installed_root(&manifest.package_id)?;
        let outcome = (|| {
            if installed_root.exists() {
                verify_materialized_package(&installed_root, &manifest)?;
                probe
                    .probe(&installed_root, &manifest)
                    .map_err(ModelManagerError::ProbeFailed)?;
                return Ok(());
            }

            prepare_staging_root(&staging_root)?;
            let mut completed_bytes = 0_u64;
            for file in &manifest.files {
                let destination = checked_join(&staging_root, &file.path)?;
                prepare_destination(&staging_root, &destination)?;
                materialize(file, &destination)?;
                verify_file(&destination, file)?;
                completed_bytes = completed_bytes
                    .checked_add(file.size)
                    .ok_or_else(|| ModelManagerError::InvalidManifest("size overflow".into()))?;
                self.update_job_progress(&job_id, completed_bytes, observed_at)?;
            }
            write_package_metadata(&staging_root, signed)?;
            self.update_job_state(&job_id, "installing", observed_at)?;
            probe
                .probe(&staging_root, &manifest)
                .map_err(ModelManagerError::ProbeFailed)?;
            if installed_root.exists() {
                return Err(ModelManagerError::Conflict(
                    "installed package target appeared during commit".into(),
                ));
            }
            fs::rename(&staging_root, &installed_root)?;
            Ok(())
        })();

        if let Err(error) = outcome {
            let _ = self.mark_install_failed(&manifest.package_id, &job_id, &error, observed_at);
            return Err(error);
        }

        let commit = self.commit_install(
            &manifest,
            &manifest_sha256,
            &job_id,
            &installed_root,
            observed_at,
        );
        if let Err(error) = commit {
            let _ = self.mark_install_failed(&manifest.package_id, &job_id, &error, observed_at);
            return Err(error);
        }
        let active_profile_id = self
            .active_profile_id()?
            .ok_or_else(|| ModelManagerError::Conflict("active profile commit was lost".into()))?;
        Ok(ModelInstallReceipt {
            package_id: manifest.package_id,
            profile_id: manifest.runtime.profile_id,
            install_job_id: job_id,
            installed_root,
            manifest_sha256,
            active_profile_id,
        })
    }

    fn verify_manifest(
        &self,
        signed: &SignedManifest,
        observed_at: i64,
    ) -> ModelManagerResult<(ModelManifest, String)> {
        if signed.manifest_json.is_empty() || signed.manifest_json.len() > MAX_MANIFEST_BYTES {
            return Err(ModelManagerError::InvalidManifest(
                "manifest size is outside 1..=1 MiB".into(),
            ));
        }
        let manifest: ModelManifest = serde_json::from_slice(&signed.manifest_json)
            .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))?;
        self.trusted_keys
            .verify(signed, &manifest.signature_key_id)?;
        validate_manifest(&manifest, observed_at)?;
        Ok((manifest, sha256_hex(&signed.manifest_json)))
    }

    fn reject_downgrade(&self, manifest: &ModelManifest) -> ModelManagerResult<()> {
        let floor: Option<i64> = self
            .database
            .connection
            .query_row(
                "SELECT MAX(release_sequence)
                 FROM model_package
                 WHERE engine_family=?1 AND model_name=?2 AND install_state='installed'",
                params![manifest.engine_family, manifest.model_name],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let floor = floor
            .map(u64::try_from)
            .transpose()
            .map_err(|_| ModelManagerError::Conflict("negative release floor".into()))?;
        if floor.is_some_and(|floor| manifest.release_sequence < floor) {
            return Err(ModelManagerError::DowngradeRejected {
                found: manifest.release_sequence,
                floor: floor.unwrap_or_default(),
            });
        }
        Ok(())
    }

    fn begin_install(
        &mut self,
        manifest: &ModelManifest,
        manifest_sha256: &str,
        observed_at: i64,
    ) -> ModelManagerResult<(String, PathBuf)> {
        let existing_checksum: Option<String> = self
            .database
            .connection
            .query_row(
                "SELECT checksum FROM model_package WHERE id=?1",
                [manifest.package_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if existing_checksum
            .as_deref()
            .is_some_and(|checksum| checksum != manifest_sha256)
        {
            return Err(ModelManagerError::Conflict(
                "package id is already bound to a different signed manifest".into(),
            ));
        }

        let staging_root = self.staging_root(&manifest.package_id)?;
        let partial_storage_key = relative_storage_key(&self.root, &staging_root)?;
        let storage_key =
            relative_storage_key(&self.root, &self.installed_root(&manifest.package_id)?)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO model_package(
                id,engine_family,model_name,model_version,source_uri,license_id,expected_size,
                checksum_algorithm,checksum,storage_key,install_state,installed_at,created_at,
                updated_at,signature_key_id,release_sequence,compatibility_abi
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,'sha256',?8,?9,'downloading',NULL,?10,?10,?11,?12,?13)
             ON CONFLICT(id) DO UPDATE SET
                install_state='downloading',updated_at=excluded.updated_at",
            params![
                manifest.package_id,
                manifest.engine_family,
                manifest.model_name,
                manifest.model_version,
                manifest.source_uri,
                manifest.license_id,
                i64::try_from(manifest.expected_size).map_err(|_| {
                    ModelManagerError::InvalidManifest("expected_size exceeds SQLite i64".into())
                })?,
                manifest_sha256,
                storage_key,
                observed_at,
                manifest.signature_key_id,
                i64::try_from(manifest.release_sequence).map_err(|_| {
                    ModelManagerError::InvalidManifest("release_sequence exceeds SQLite i64".into())
                })?,
                manifest.compatibility_abi,
            ],
        )?;
        let active_job: Option<String> = transaction
            .query_row(
                "SELECT id FROM model_install_job
                 WHERE model_package_id=?1
                   AND state IN ('queued','downloading','verifying','installing')",
                [manifest.package_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let job_id =
            active_job.unwrap_or_else(|| format!("{}-install-{observed_at}", manifest.package_id));
        transaction.execute(
            "INSERT INTO model_install_job(
                id,model_package_id,state,bytes_downloaded,total_bytes,resume_token,
                partial_storage_key,started_at,updated_at,completed_at,error_code
             ) VALUES(?1,?2,'downloading',0,?3,NULL,?4,?5,?5,NULL,NULL)
             ON CONFLICT(id) DO UPDATE SET state='downloading',updated_at=excluded.updated_at,
                completed_at=NULL,error_code=NULL",
            params![
                job_id,
                manifest.package_id,
                i64::try_from(manifest.expected_size).unwrap_or(i64::MAX),
                partial_storage_key,
                observed_at,
            ],
        )?;
        transaction.commit()?;
        Ok((job_id, staging_root))
    }

    fn update_job_progress(
        &self,
        job_id: &str,
        bytes: u64,
        observed_at: i64,
    ) -> ModelManagerResult<()> {
        self.database.connection.execute(
            "UPDATE model_install_job
             SET state='verifying',bytes_downloaded=?2,updated_at=?3
             WHERE id=?1",
            params![
                job_id,
                i64::try_from(bytes)
                    .map_err(|_| ModelManagerError::InvalidManifest("size exceeds i64".into()))?,
                observed_at,
            ],
        )?;
        Ok(())
    }

    fn update_job_state(
        &self,
        job_id: &str,
        state: &str,
        observed_at: i64,
    ) -> ModelManagerResult<()> {
        self.database.connection.execute(
            "UPDATE model_install_job SET state=?2,updated_at=?3 WHERE id=?1",
            params![job_id, state, observed_at],
        )?;
        Ok(())
    }

    fn commit_install(
        &mut self,
        manifest: &ModelManifest,
        manifest_sha256: &str,
        job_id: &str,
        installed_root: &Path,
        observed_at: i64,
    ) -> ModelManagerResult<()> {
        let settings = serde_json::to_string(&manifest.runtime.settings)
            .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))?;
        let settings_hash = sha256_hex(settings.as_bytes());
        let existing_profile_package: Option<String> = self
            .database
            .connection
            .query_row(
                "SELECT model_package_id FROM runtime_profile WHERE id=?1",
                [manifest.runtime.profile_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if existing_profile_package
            .as_deref()
            .is_some_and(|package| package != manifest.package_id)
        {
            return Err(ModelManagerError::Conflict(
                "profile id is already bound to another package".into(),
            ));
        }
        let storage_key = relative_storage_key(&self.root, installed_root)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE model_package
             SET install_state='installed',installed_at=?2,updated_at=?2,storage_key=?3,
                 checksum=?4,signature_key_id=?5,release_sequence=?6,compatibility_abi=?7
             WHERE id=?1",
            params![
                manifest.package_id,
                observed_at,
                storage_key,
                manifest_sha256,
                manifest.signature_key_id,
                i64::try_from(manifest.release_sequence).map_err(|_| {
                    ModelManagerError::InvalidManifest("release_sequence exceeds i64".into())
                })?,
                manifest.compatibility_abi,
            ],
        )?;
        transaction.execute(
            "INSERT INTO runtime_profile(
                id,profile_version,model_package_id,adapter_type,adapter_version,
                device_kind,device_id,settings,settings_hash,health_state,last_health_at,
                enabled,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'healthy',?10,1,?10,?10)
             ON CONFLICT(id) DO UPDATE SET
                adapter_version=excluded.adapter_version,device_id=excluded.device_id,
                settings=excluded.settings,settings_hash=excluded.settings_hash,
                health_state='healthy',last_health_at=excluded.last_health_at,
                enabled=1,updated_at=excluded.updated_at",
            params![
                manifest.runtime.profile_id,
                i64::from(manifest.runtime.profile_version),
                manifest.package_id,
                manifest.runtime.adapter_type,
                manifest.runtime.adapter_version,
                manifest.runtime.device_kind,
                manifest.runtime.device_id,
                settings,
                settings_hash,
                observed_at,
            ],
        )?;
        transaction.execute(
            "UPDATE model_install_job
             SET state='succeeded',bytes_downloaded=total_bytes,updated_at=?2,
                 completed_at=?2,error_code=NULL
             WHERE id=?1",
            params![job_id, observed_at],
        )?;
        // Installing is not switching. The very first package has to become active or the user
        // downloads a model and dictation still reports that none is present; every later one
        // waits for an explicit "make active", so a download never moves the model in use.
        let active: Option<String> = transaction
            .query_row(
                "SELECT active_runtime_profile_id FROM app_configuration WHERE is_active=1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if active.is_none() {
            activate_configuration(&transaction, &manifest.runtime.profile_id, observed_at)?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn mark_install_failed(
        &mut self,
        package_id: &str,
        job_id: &str,
        error: &ModelManagerError,
        observed_at: i64,
    ) -> ModelManagerResult<()> {
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // A cancel is a pause, not a defeat: the package stays `downloading` and the staging
        // bytes stay on disk, so the next install resumes instead of starting over.
        let cancelled = matches!(error, ModelManagerError::Cancelled);
        if !cancelled {
            transaction.execute(
                "UPDATE model_package
                 SET install_state='failed',updated_at=?2
                 WHERE id=?1 AND install_state<>'installed'",
                params![package_id, observed_at],
            )?;
        }
        transaction.execute(
            "UPDATE model_install_job
             SET state=?4,updated_at=?2,completed_at=?2,error_code=?3
             WHERE id=?1",
            params![
                job_id,
                observed_at,
                error_code(error),
                if cancelled { "cancelled" } else { "failed" },
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn staging_root(&self, package_id: &str) -> ModelManagerResult<PathBuf> {
        validate_token("package_id", package_id)?;
        checked_join(&self.root.join("staging"), package_id)
    }

    fn installed_root(&self, package_id: &str) -> ModelManagerResult<PathBuf> {
        validate_token("package_id", package_id)?;
        checked_join(&self.root.join("installed"), package_id)
    }
}

/// Deletes a package directory, tolerating the Windows habit of holding a handle open just after
/// the files inside were read.
///
/// A retry covers the usual antivirus or lingering-handle case. If the directory still refuses to
/// go but is already empty, the removal has done its job - the bytes are freed - so an inert empty
/// folder must not fail the whole operation and leave the user thinking the model is still there.
fn remove_managed_directory(root: &Path) -> ModelManagerResult<()> {
    for attempt in 0..3 {
        match fs::remove_dir_all(root) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                if attempt == 2 {
                    let empty = fs::read_dir(root)
                        .map(|mut entries| entries.next().is_none())
                        .unwrap_or(false);
                    if empty {
                        return Ok(());
                    }
                    return Err(ModelManagerError::Io(error));
                }
                std::thread::sleep(Duration::from_millis(150 * (attempt + 1) as u64));
            }
        }
    }
    Ok(())
}

fn activate_configuration(
    transaction: &rusqlite::Transaction<'_>,
    profile_id: &str,
    observed_at: i64,
) -> ModelManagerResult<()> {
    type ActiveConfiguration = (String, Option<String>, Option<String>, i64, i64, i64);
    let previous: Option<ActiveConfiguration> = transaction
        .query_row(
            "SELECT hotkey_binding,microphone_device_id,active_cleanup_profile_id,
                    startup_enabled,warmup_enabled,diagnostic_mode
             FROM app_configuration WHERE is_active=1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    transaction.execute(
        "UPDATE app_configuration
         SET is_active=0,superseded_at=?1
         WHERE is_active=1",
        [observed_at],
    )?;
    let config_version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(config_version),0)+1 FROM app_configuration",
        [],
        |row| row.get(0),
    )?;
    let (hotkey, microphone, cleanup, startup, warmup, diagnostic) =
        previous.unwrap_or_else(|| ("F8".into(), None, None, 0, 0, 0));
    transaction.execute(
        "INSERT INTO app_configuration(
            id,schema_version,config_version,is_active,hotkey_binding,microphone_device_id,
            active_runtime_profile_id,active_cleanup_profile_id,startup_enabled,warmup_enabled,
            diagnostic_mode,created_at,activated_at,superseded_at
         ) VALUES(?1,1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?10,NULL)",
        params![
            format!("model-config-{config_version}-{observed_at}"),
            config_version,
            hotkey,
            microphone,
            profile_id,
            cleanup,
            startup,
            warmup,
            diagnostic,
            observed_at,
        ],
    )?;
    Ok(())
}

fn validate_manifest(manifest: &ModelManifest, observed_at: i64) -> ModelManagerResult<()> {
    if manifest.schema_version != 1 {
        return Err(ModelManagerError::InvalidManifest(
            "schema_version must be 1".into(),
        ));
    }
    for (field, value) in [
        ("package_id", manifest.package_id.as_str()),
        ("engine_family", manifest.engine_family.as_str()),
        ("model_name", manifest.model_name.as_str()),
        ("model_version", manifest.model_version.as_str()),
        ("signature_key_id", manifest.signature_key_id.as_str()),
        ("profile_id", manifest.runtime.profile_id.as_str()),
        ("adapter_type", manifest.runtime.adapter_type.as_str()),
        ("adapter_version", manifest.runtime.adapter_version.as_str()),
    ] {
        validate_token(field, value)?;
    }
    if manifest.license_id.trim().is_empty() || manifest.license_id.len() > 128 {
        return Err(ModelManagerError::InvalidManifest(
            "license_id must be 1..=128 characters".into(),
        ));
    }
    validate_source_uri(&manifest.source_uri)?;
    if manifest.release_sequence == 0 {
        return Err(ModelManagerError::InvalidManifest(
            "release_sequence must be positive".into(),
        ));
    }
    if manifest.minimum_manager_version > MODEL_MANAGER_VERSION {
        return Err(ModelManagerError::InvalidManifest(format!(
            "manager version {} is below manifest floor {}",
            MODEL_MANAGER_VERSION, manifest.minimum_manager_version
        )));
    }
    if manifest.expires_at_ms < observed_at {
        return Err(ModelManagerError::InvalidManifest(
            "manifest is expired".into(),
        ));
    }
    if manifest.compatibility_abi != SUPPORTED_MODEL_ABI {
        return Err(ModelManagerError::IncompatibleAbi(
            manifest.compatibility_abi.clone(),
        ));
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_PACKAGE_FILES {
        return Err(ModelManagerError::InvalidManifest(
            "files must contain 1..=256 entries".into(),
        ));
    }
    if !matches!(
        manifest.runtime.device_kind.as_str(),
        "cpu" | "vulkan" | "directml"
    ) {
        return Err(ModelManagerError::InvalidManifest(
            "unsupported device_kind".into(),
        ));
    }
    if manifest.runtime.profile_version == 0 {
        return Err(ModelManagerError::InvalidManifest(
            "profile_version must be positive".into(),
        ));
    }
    if !manifest.runtime.settings.is_object() {
        return Err(ModelManagerError::InvalidManifest(
            "runtime settings must be a JSON object".into(),
        ));
    }
    let mut paths = BTreeSet::new();
    let mut total = 0_u64;
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        if file.path.starts_with(".wigigadict-") {
            return Err(ModelManagerError::PathRejected(file.path.clone()));
        }
        if !paths.insert(file.path.to_ascii_lowercase()) {
            return Err(ModelManagerError::InvalidManifest(
                "file paths must be unique case-insensitively".into(),
            ));
        }
        if file.size == 0 {
            return Err(ModelManagerError::InvalidManifest(format!(
                "{} has zero size",
                file.path
            )));
        }
        validate_sha256(&file.sha256)?;
        if let Some(uri) = &file.download_uri {
            validate_https_uri(uri)?;
        }
        total = total
            .checked_add(file.size)
            .ok_or_else(|| ModelManagerError::InvalidManifest("package size overflow".into()))?;
    }
    if total != manifest.expected_size || total == 0 || total > i64::MAX as u64 {
        return Err(ModelManagerError::InvalidManifest(
            "expected_size must equal the sum of file sizes and fit SQLite".into(),
        ));
    }
    validate_relative_path(&manifest.runtime.probe_file)?;
    if !paths.contains(&manifest.runtime.probe_file.to_ascii_lowercase()) {
        return Err(ModelManagerError::InvalidManifest(
            "probe_file must name a manifest file".into(),
        ));
    }
    Ok(())
}

fn validate_token(field: &str, value: &str) -> ModelManagerResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ModelManagerError::InvalidManifest(format!(
            "{field} must be 1..=128 safe ASCII characters"
        )));
    }
    Ok(())
}

fn validate_source_uri(uri: &str) -> ModelManagerResult<()> {
    if uri.starts_with("https://") || uri.starts_with("file://") {
        return Ok(());
    }
    Err(ModelManagerError::InvalidManifest(
        "source_uri must use https:// or file://".into(),
    ))
}

fn validate_https_uri(uri: &str) -> ModelManagerResult<()> {
    if uri.starts_with("https://") && !uri.chars().any(char::is_whitespace) {
        return Ok(());
    }
    Err(ModelManagerError::InvalidManifest(
        "download_uri must use HTTPS".into(),
    ))
}

fn validate_sha256(value: &str) -> ModelManagerResult<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ModelManagerError::InvalidManifest(
            "sha256 must contain exactly 64 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> ModelManagerResult<()> {
    if value.is_empty() || value.len() > 240 || value.contains(':') {
        return Err(ModelManagerError::PathRejected(value.into()));
    }
    let path = Path::new(value);
    let mut components = 0_usize;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(ModelManagerError::PathRejected(value.into()));
        };
        let name = name
            .to_str()
            .ok_or_else(|| ModelManagerError::PathRejected(value.into()))?;
        let normalized = name.trim_end_matches([' ', '.']);
        let stem = normalized.split('.').next().unwrap_or_default();
        if normalized != name
            || normalized.is_empty()
            || is_windows_reserved_name(stem)
            || name.contains('\0')
        {
            return Err(ModelManagerError::PathRejected(value.into()));
        }
        components += 1;
    }
    if components == 0 {
        return Err(ModelManagerError::PathRejected(value.into()));
    }
    Ok(())
}

fn is_windows_reserved_name(value: &str) -> bool {
    let value = value.to_ascii_uppercase();
    matches!(value.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || value
            .strip_prefix("COM")
            .or_else(|| value.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                suffix.len() == 1
                    && suffix
                        .as_bytes()
                        .first()
                        .is_some_and(|byte| matches!(byte, b'1'..=b'9'))
            })
}

fn checked_join(base: &Path, relative: &str) -> ModelManagerResult<PathBuf> {
    validate_relative_path(relative)?;
    Ok(base.join(relative))
}

fn prepare_managed_root(root: &Path) -> ModelManagerResult<()> {
    fs::create_dir_all(root)?;
    reject_reparse(root)?;
    for child in ["staging", "installed"] {
        let path = root.join(child);
        fs::create_dir_all(&path)?;
        reject_reparse(&path)?;
    }
    Ok(())
}

fn prepare_staging_root(root: &Path) -> ModelManagerResult<()> {
    if root.exists() {
        reject_reparse(root)?;
        if !root.is_dir() {
            return Err(ModelManagerError::PathRejected(root.display().to_string()));
        }
    } else {
        fs::create_dir(root)?;
    }
    Ok(())
}

fn prepare_destination(staging_root: &Path, destination: &Path) -> ModelManagerResult<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| ModelManagerError::PathRejected(destination.display().to_string()))?;
    let relative = parent
        .strip_prefix(staging_root)
        .map_err(|_| ModelManagerError::PathRejected(destination.display().to_string()))?;
    let mut cursor = staging_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(ModelManagerError::PathRejected(
                destination.display().to_string(),
            ));
        };
        cursor.push(name);
        if cursor.exists() {
            reject_reparse(&cursor)?;
        } else {
            fs::create_dir(&cursor)?;
        }
    }
    if destination.exists() {
        reject_reparse(destination)?;
        if !destination.is_file() {
            return Err(ModelManagerError::PathRejected(
                destination.display().to_string(),
            ));
        }
    }
    Ok(())
}
fn canonical_directory(path: &Path) -> ModelManagerResult<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(ModelManagerError::PathRejected(path.display().to_string()));
    }
    reject_reparse(path)?;
    Ok(fs::canonicalize(path)?)
}

fn copy_offline_file(
    source_root: &Path,
    manifest_file: &ManifestFile,
    destination: &Path,
) -> ModelManagerResult<()> {
    let source = checked_join(source_root, &manifest_file.path)?;
    let metadata = fs::symlink_metadata(&source)?;
    reject_reparse(&source)?;
    if !metadata.is_file() || metadata.len() != manifest_file.size {
        return Err(ModelManagerError::CorruptArtifact(
            manifest_file.path.clone(),
        ));
    }
    let canonical_source = fs::canonicalize(&source)?;
    if !canonical_source.starts_with(source_root) {
        return Err(ModelManagerError::PathRejected(manifest_file.path.clone()));
    }
    let existing = destination.metadata().map(|value| value.len()).unwrap_or(0);
    if existing > manifest_file.size {
        return Err(ModelManagerError::CorruptArtifact(
            manifest_file.path.clone(),
        ));
    }
    if existing == manifest_file.size {
        return Ok(());
    }
    let mut input = File::open(canonical_source)?;
    input.seek(SeekFrom::Start(existing))?;
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(destination)?;
    let remaining = manifest_file.size - existing;
    let copied = std::io::copy(&mut input.take(remaining + 1), &mut output)?;
    if copied != remaining {
        if copied > remaining {
            output.set_len(manifest_file.size)?;
        }
        return Err(ModelManagerError::CorruptArtifact(
            manifest_file.path.clone(),
        ));
    }
    output.sync_all()?;
    Ok(())
}

/// Streams `reader` into `destination` in bounded chunks so progress is visible and a cancel is
/// honoured mid-file. `std::io::copy` could do neither: one 574 MB weight file reported nothing
/// until it finished and ignored every cancel until then.
fn copy_observed(
    reader: &mut impl Read,
    destination: &mut File,
    remaining: u64,
    observer: &dyn DownloadObserver,
) -> ModelManagerResult<u64> {
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_BYTES];
    let mut written = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(written);
        }
        destination.write_all(&buffer[..read])?;
        written = written.saturating_add(read as u64);
        if !observer.advanced(read as u64) {
            destination.sync_all()?;
            return Err(ModelManagerError::Cancelled);
        }
        if written > remaining {
            return Ok(written);
        }
    }
}

fn download_file(
    downloader: &dyn RangeDownloader,
    manifest_file: &ManifestFile,
    destination: &Path,
    observer: &dyn DownloadObserver,
) -> ModelManagerResult<()> {
    let uri = manifest_file.download_uri.as_deref().ok_or_else(|| {
        ModelManagerError::InvalidManifest(format!("{} has no download_uri", manifest_file.path))
    })?;
    let mut destination = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(destination)?;
    let offset = destination.metadata()?.len();
    if offset > manifest_file.size {
        return Err(ModelManagerError::CorruptArtifact(
            manifest_file.path.clone(),
        ));
    }
    if offset < manifest_file.size {
        downloader.download(uri, offset, manifest_file.size, &mut destination, observer)?;
    }
    Ok(())
}

fn verify_materialized_package(
    package_root: &Path,
    manifest: &ModelManifest,
) -> ModelManagerResult<()> {
    reject_reparse(package_root)?;
    for manifest_file in &manifest.files {
        let path = checked_join(package_root, &manifest_file.path)?;
        verify_file(&path, manifest_file)?;
    }
    Ok(())
}

fn verify_file(path: &Path, manifest_file: &ManifestFile) -> ModelManagerResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    reject_reparse(path)?;
    if !metadata.is_file() || metadata.len() != manifest_file.size {
        return Err(ModelManagerError::CorruptArtifact(
            manifest_file.path.clone(),
        ));
    }
    if sha256_file(path)? != manifest_file.sha256.to_ascii_lowercase() {
        return Err(ModelManagerError::CorruptArtifact(
            manifest_file.path.clone(),
        ));
    }
    Ok(())
}

fn write_package_metadata(root: &Path, signed: &SignedManifest) -> ModelManagerResult<()> {
    for (name, bytes) in [
        (".wigigadict-manifest.json", signed.manifest_json.as_slice()),
        (
            ".wigigadict-manifest.ed25519",
            signed.signature_hex.as_bytes(),
        ),
    ] {
        let path = root.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    Ok(())
}

fn sha256_file(path: &Path) -> ModelManagerResult<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], ()> {
    if value.len() != N * 2 {
        return Err(());
    }
    let mut output = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or(())?;
        let low = hex_nibble(pair[1]).ok_or(())?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn relative_storage_key(root: &Path, path: &Path) -> ModelManagerResult<String> {
    Ok(path
        .strip_prefix(root)
        .map_err(|_| ModelManagerError::PathRejected(path.display().to_string()))?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn reject_reparse(path: &Path) -> ModelManagerResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || has_windows_reparse_attribute(&metadata) {
        return Err(ModelManagerError::PathRejected(path.display().to_string()));
    }
    Ok(())
}

#[cfg(windows)]
fn has_windows_reparse_attribute(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn has_windows_reparse_attribute(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn available_disk_bytes(path: &Path) -> ModelManagerResult<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    // SAFETY: wide is a NUL-terminated path and available is a valid output pointer.
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if succeeded == 0 {
        return Err(ModelManagerError::Io(std::io::Error::last_os_error()));
    }
    Ok(available)
}

#[cfg(not(windows))]
fn available_disk_bytes(_path: &Path) -> ModelManagerResult<u64> {
    Ok(u64::MAX)
}

fn error_code(error: &ModelManagerError) -> &'static str {
    match error {
        ModelManagerError::InvalidManifest(_) => "invalid_manifest",
        ModelManagerError::UnknownSigningKey(_) => "unknown_signing_key",
        ModelManagerError::RevokedSigningKey(_) => "revoked_signing_key",
        ModelManagerError::InvalidSignature => "invalid_signature",
        ModelManagerError::DowngradeRejected { .. } => "downgrade_rejected",
        ModelManagerError::IncompatibleAbi(_) => "incompatible_abi",
        ModelManagerError::PathRejected(_) => "path_rejected",
        ModelManagerError::InsufficientDisk { .. } => "insufficient_disk",
        ModelManagerError::CorruptArtifact(_) => "corrupt_artifact",
        ModelManagerError::ProbeFailed(_) => "probe_failed",
        ModelManagerError::Conflict(_) => "conflict",
        ModelManagerError::Network(_) => "network_error",
        ModelManagerError::Cancelled => "cancelled",
        ModelManagerError::Io(_) => "io_error",
        ModelManagerError::Storage(_) => "storage_error",
        ModelManagerError::Sqlite(_) => "sqlite_error",
        ModelManagerError::ClockBeforeUnixEpoch => "clock_error",
    }
}

fn now_ms() -> ModelManagerResult<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ModelManagerError::ClockBeforeUnixEpoch)?;
    i64::try_from(duration.as_millis())
        .map_err(|_| ModelManagerError::InvalidManifest("timestamp exceeds i64".into()))
}
