//! Model screen backend: the signed catalog, the installed packages and the one install that may
//! run at a time.
//!
//! Downloading is the only thing in WiGigaDict that touches the network, and it only ever starts
//! from an explicit click here.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};
use wigigadict_storage::{
    CatalogEntry, DownloadObserver, FileCompatibilityProbe, InstalledModel, ModelCatalog,
    ModelManager, ModelManagerError, ModelManifest, ReqwestRangeDownloader, SystemDiskSpace,
    TrustedKeyRing, verify_catalog,
};

use crate::shell_lifecycle;

/// Public half of the catalog signing key, injected at the release/desktop boundary. Without it
/// the catalog is simply unavailable: there is no unsigned fallback to degrade into.
const CATALOG_PUBLIC_KEY_HEX: Option<&str> = option_env!("WIGIGADICT_CATALOG_PUBLIC_KEY");
const CATALOG_KEY_ID: &str = "wigigadict-catalog-v1";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsView {
    pub items: Vec<ModelItem>,
    pub active_profile_id: Option<String>,
    /// Why the catalog is not showing, when it is not showing. An empty screen with no reason is
    /// indistinguishable from a broken app.
    pub catalog_error: Option<String>,
    pub busy_package_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelItem {
    pub package_id: String,
    pub profile_id: Option<String>,
    pub display_name: String,
    pub summary: String,
    pub languages: Vec<String>,
    pub license_id: String,
    pub total_bytes: u64,
    pub device_kind: String,
    pub min_ram_mb: Option<u32>,
    pub min_vram_mb: Option<u32>,
    pub recommended: bool,
    /// False means this project never ran the model. The screen must not show accuracy or speed
    /// for those: we would be inventing numbers.
    pub owner_measured: bool,
    /// `available` | `downloading` | `paused` | `installed` | `failed`
    pub state: String,
    pub is_active: bool,
    pub bytes_downloaded: u64,
    pub health_state: Option<String>,
    /// False for a package installed by some earlier route that the catalog no longer lists.
    pub in_catalog: bool,
    /// Set when these exact weights are already on disk inside a different package. Offering the
    /// download anyway would spend hundreds of megabytes on bytes the user already has.
    pub duplicate_of: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    package_id: String,
    bytes_downloaded: u64,
    total_bytes: u64,
}

struct ActiveInstall {
    package_id: String,
    cancel: Arc<AtomicBool>,
    /// A pause keeps the partial download; a cancel throws it away.
    discard: Arc<AtomicBool>,
}

pub struct ModelService {
    database_path: PathBuf,
    managed_root: PathBuf,
    catalog: Option<ModelCatalog>,
    catalog_error: Option<String>,
    install: Arc<Mutex<Option<ActiveInstall>>>,
}

impl ModelService {
    pub fn new(
        database_path: impl AsRef<Path>,
        managed_root: impl AsRef<Path>,
        catalog_dir: impl AsRef<Path>,
    ) -> Self {
        let (catalog, catalog_error) = match load_catalog(catalog_dir.as_ref()) {
            Ok(catalog) => (Some(catalog), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            database_path: database_path.as_ref().to_owned(),
            managed_root: managed_root.as_ref().to_owned(),
            catalog,
            catalog_error,
            install: Arc::new(Mutex::new(None)),
        }
    }

    /// Opens the manager for local work: listing, activating and removing what is already on disk.
    ///
    /// A missing signing key must not hide the models the user already installed, so the ring is
    /// allowed to be empty here. Nothing is weakened by that: an empty ring trusts no signature at
    /// all, so any install attempted through it still fails closed.
    fn manager(&self) -> Result<ModelManager, String> {
        let keys = trusted_keys().unwrap_or_default();
        ModelManager::open(&self.database_path, &self.managed_root, keys)
            .map_err(|error| error.to_string())
    }

    fn entry(&self, package_id: &str) -> Result<&CatalogEntry, String> {
        self.catalog
            .as_ref()
            .ok_or_else(|| {
                self.catalog_error
                    .clone()
                    .unwrap_or_else(|| "catalog is unavailable".to_owned())
            })?
            .entries
            .iter()
            .find(|entry| {
                entry
                    .manifest()
                    .map(|manifest| manifest.package_id == package_id)
                    .unwrap_or(false)
            })
            .ok_or_else(|| "package is not in the catalog".to_owned())
    }

    fn busy(&self) -> Option<String> {
        self.install
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|active| active.package_id.clone()))
    }

    fn view(&self) -> Result<ModelsView, String> {
        let manager = self.manager()?;
        let installed = manager.list_packages().map_err(|error| error.to_string())?;
        let active_profile_id = manager.active_profile_id().ok().flatten();
        let busy = self.busy();

        let weights = installed_weight_hashes(&self.managed_root, &installed);
        let mut items = Vec::new();
        if let Some(catalog) = &self.catalog {
            for entry in &catalog.entries {
                let manifest = entry.manifest().map_err(|error| error.to_string())?;
                let row = installed
                    .iter()
                    .find(|row| row.package_id == manifest.package_id);
                let duplicate_of = if row.is_some() {
                    None
                } else {
                    weights_sha256(&manifest).and_then(|hash| {
                        weights
                            .iter()
                            .find(|(installed_hash, _)| *installed_hash == hash)
                            .map(|(_, package)| package.clone())
                    })
                };
                items.push(catalog_item(
                    entry,
                    &manifest,
                    row,
                    busy.as_deref(),
                    duplicate_of,
                ));
            }
        }
        // A package installed before this catalog existed is still real and still occupies disk.
        for row in &installed {
            if items.iter().any(|item| item.package_id == row.package_id) {
                continue;
            }
            if row.install_state == "absent" {
                continue;
            }
            items.push(installed_only_item(row, busy.as_deref()));
        }

        Ok(ModelsView {
            items,
            active_profile_id,
            catalog_error: self.catalog_error.clone(),
            busy_package_id: busy,
        })
    }
}

/// SHA-256 of the weights each installed package holds, so the catalog can recognise the same
/// model arriving under a different package id.
fn installed_weight_hashes(
    managed_root: &Path,
    installed: &[InstalledModel],
) -> Vec<(String, String)> {
    let mut hashes = Vec::new();
    for row in installed {
        if row.install_state != "installed" {
            continue;
        }
        let path = managed_root
            .join("installed")
            .join(&row.package_id)
            .join(".wigigadict-manifest.json");
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<ModelManifest>(&bytes) else {
            continue;
        };
        if let Some(weights) = weights_sha256(&manifest) {
            hashes.push((weights, row.package_id.clone()));
        }
    }
    hashes
}

/// The hash of the file the runtime actually loads as the model, ignoring anything else the
/// package carries.
fn weights_sha256(manifest: &ModelManifest) -> Option<String> {
    let model_path = manifest.runtime.settings.get("model_path")?.as_str()?;
    manifest
        .files
        .iter()
        .find(|file| file.path == model_path)
        .map(|file| file.sha256.clone())
}

fn catalog_item(
    entry: &CatalogEntry,
    manifest: &ModelManifest,
    row: Option<&InstalledModel>,
    busy: Option<&str>,
    duplicate_of: Option<String>,
) -> ModelItem {
    let state = card_state(busy == Some(manifest.package_id.as_str()), row);
    ModelItem {
        package_id: manifest.package_id.clone(),
        profile_id: row.and_then(|row| row.profile_id.clone()),
        display_name: entry.display_name.clone(),
        summary: entry.summary.clone(),
        languages: entry.languages.clone(),
        license_id: manifest.license_id.clone(),
        total_bytes: manifest.expected_size,
        device_kind: entry.requirements.device_kind.clone(),
        min_ram_mb: Some(entry.requirements.min_ram_mb),
        min_vram_mb: entry.requirements.min_vram_mb,
        recommended: entry.recommended,
        owner_measured: entry.owner_measured,
        state: state.to_owned(),
        is_active: row.is_some_and(|row| row.is_active),
        bytes_downloaded: row.map_or(0, |row| row.bytes_downloaded),
        health_state: row.and_then(|row| row.health_state.clone()),
        in_catalog: true,
        duplicate_of,
    }
}

fn installed_only_item(row: &InstalledModel, busy: Option<&str>) -> ModelItem {
    ModelItem {
        package_id: row.package_id.clone(),
        profile_id: row.profile_id.clone(),
        display_name: format!("{} {}", row.model_name, row.model_version),
        summary: String::new(),
        languages: Vec::new(),
        license_id: row.license_id.clone(),
        total_bytes: row.expected_size,
        device_kind: row.device_kind.clone().unwrap_or_default(),
        min_ram_mb: None,
        min_vram_mb: None,
        recommended: false,
        owner_measured: false,
        state: install_state(row, busy == Some(row.package_id.as_str())).to_owned(),
        is_active: row.is_active,
        bytes_downloaded: row.bytes_downloaded,
        health_state: row.health_state.clone(),
        in_catalog: false,
        duplicate_of: None,
    }
}

/// State of a catalog card.
///
/// A download that has just started has no database row yet: the install thread writes one only
/// when it reaches `begin_install`. Without the `running` shortcut the card stays on "available"
/// for the whole download and the progress bar never appears, so pressing Скачать looks like
/// nothing happened at all.
fn card_state(running: bool, row: Option<&InstalledModel>) -> &'static str {
    if running {
        return "downloading";
    }
    row.map_or("available", |row| install_state(row, false))
}

fn install_state(row: &InstalledModel, running: bool) -> &'static str {
    match row.install_state.as_str() {
        "installed" => "installed",
        "absent" => "available",
        "failed" | "corrupt" => "failed",
        // Bytes on disk with no live thread is a pause the user can resume, not a failure.
        _ if running => "downloading",
        _ => "paused",
    }
}

fn trusted_keys() -> Result<TrustedKeyRing, String> {
    let hex = CATALOG_PUBLIC_KEY_HEX.ok_or_else(|| {
        "this build has no catalog signing key: rebuild with WIGIGADICT_CATALOG_PUBLIC_KEY"
            .to_owned()
    })?;
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err("catalog signing key must be 64 hex characters".into());
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| "catalog signing key is not hex".to_owned())?;
    }
    let mut keys = TrustedKeyRing::new();
    keys.insert(CATALOG_KEY_ID, bytes)
        .map_err(|error| error.to_string())?;
    Ok(keys)
}

fn load_catalog(directory: &Path) -> Result<ModelCatalog, String> {
    let keys = trusted_keys()?;
    let catalog_path = directory.join("catalog.json");
    let signature_path = directory.join("catalog.sig");
    if !catalog_path.is_file() || !signature_path.is_file() {
        return Err("this build ships no model catalog".into());
    }
    let catalog_json = std::fs::read(&catalog_path).map_err(|error| error.to_string())?;
    let signature = std::fs::read_to_string(&signature_path).map_err(|error| error.to_string())?;
    verify_catalog(&keys, &catalog_json, signature.trim()).map_err(|error| error.to_string())
}

/// Turns download bytes into throttled UI events and carries the cancel flag into the byte loop.
struct ProgressEmitter {
    app: AppHandle,
    package_id: String,
    total_bytes: u64,
    downloaded: AtomicU64,
    last_emit: Mutex<Instant>,
    cancel: Arc<AtomicBool>,
}

impl DownloadObserver for ProgressEmitter {
    fn advanced(&self, bytes: u64) -> bool {
        let downloaded = self.downloaded.fetch_add(bytes, Ordering::Relaxed) + bytes;
        if let Ok(mut last) = self.last_emit.lock()
            && last.elapsed() >= PROGRESS_INTERVAL
        {
            *last = Instant::now();
            let _ = self.app.emit_to(
                "main",
                "models-progress",
                ProgressEvent {
                    package_id: self.package_id.clone(),
                    bytes_downloaded: downloaded,
                    total_bytes: self.total_bytes,
                },
            );
        }
        !self.cancel.load(Ordering::Relaxed)
    }
}

/// Bytes already on disk from an interrupted download, so a resumed install reports real progress
/// instead of restarting the bar at zero.
fn staged_bytes(managed_root: &Path, manifest: &ModelManifest) -> u64 {
    let staging = managed_root.join("staging").join(&manifest.package_id);
    manifest
        .files
        .iter()
        .filter_map(|file| std::fs::metadata(staging.join(&file.path)).ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

fn changed(app: &AppHandle) {
    let _ = app.emit_to("main", "models-changed", ());
}

#[tauri::command]
pub fn models_list(
    window: tauri::WebviewWindow,
    service: State<'_, ModelService>,
) -> Result<ModelsView, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    service.view()
}

#[tauri::command]
pub fn models_install_start(
    app: AppHandle,
    window: tauri::WebviewWindow,
    service: State<'_, ModelService>,
    package_id: String,
) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    let entry = service.entry(&package_id)?;
    let manifest = entry.manifest().map_err(|error| error.to_string())?;
    let signed = entry.signed_manifest();

    let cancel = Arc::new(AtomicBool::new(false));
    let discard = Arc::new(AtomicBool::new(false));
    {
        let mut guard = service
            .install
            .lock()
            .map_err(|_| "install lock poisoned")?;
        if let Some(active) = guard.as_ref() {
            return Err(format!("{} is already installing", active.package_id));
        }
        *guard = Some(ActiveInstall {
            package_id: package_id.clone(),
            cancel: cancel.clone(),
            discard: discard.clone(),
        });
    }

    let database_path = service.database_path.clone();
    let managed_root = service.managed_root.clone();
    let slot = service.install.clone();
    std::thread::spawn(move || {
        let observer = ProgressEmitter {
            app: app.clone(),
            package_id: package_id.clone(),
            total_bytes: manifest.expected_size,
            downloaded: AtomicU64::new(staged_bytes(&managed_root, &manifest)),
            last_emit: Mutex::new(Instant::now() - PROGRESS_INTERVAL),
            cancel,
        };
        let outcome = (|| {
            let keys = trusted_keys()?;
            let mut manager = ModelManager::open(&database_path, &managed_root, keys)
                .map_err(|error| error.to_string())?;
            let downloader = ReqwestRangeDownloader::new().map_err(|error| error.to_string())?;
            let result = manager.install_online_with(
                &signed,
                &downloader,
                &SystemDiskSpace,
                &FileCompatibilityProbe,
                now_ms(),
                &observer,
            );
            match result {
                Ok(_) => Ok(()),
                Err(ModelManagerError::Cancelled) => {
                    if discard.load(Ordering::Relaxed) {
                        manager
                            .remove_package(&package_id, now_ms())
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            }
        })();
        if let Ok(mut guard) = slot.lock() {
            *guard = None;
        }
        if let Err(error) = outcome {
            let _ = app.emit_to("main", "models-error", error);
        }
        changed(&app);
    });
    Ok(())
}

/// Stops the running download and keeps its bytes, so the next start resumes.
#[tauri::command]
pub fn models_install_pause(
    window: tauri::WebviewWindow,
    service: State<'_, ModelService>,
) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    let guard = service
        .install
        .lock()
        .map_err(|_| "install lock poisoned")?;
    if let Some(active) = guard.as_ref() {
        active.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Stops the running download and throws its bytes away.
#[tauri::command]
pub fn models_install_cancel(
    window: tauri::WebviewWindow,
    service: State<'_, ModelService>,
) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    let guard = service
        .install
        .lock()
        .map_err(|_| "install lock poisoned")?;
    if let Some(active) = guard.as_ref() {
        active.discard.store(true, Ordering::Relaxed);
        active.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Installs from files the user already has. This is the only place an absolute path from outside
/// the managed root is accepted, and it still goes through the same signed manifest and checksums.
#[tauri::command]
pub fn models_import_local(
    app: AppHandle,
    window: tauri::WebviewWindow,
    service: State<'_, ModelService>,
    package_id: String,
    source_directory: String,
) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    if service.busy().is_some() {
        return Err("an install is already running".into());
    }
    let signed = service.entry(&package_id)?.signed_manifest();
    let mut manager = service.manager()?;
    manager
        .install_offline_with(
            &signed,
            Path::new(&source_directory),
            &SystemDiskSpace,
            &FileCompatibilityProbe,
            now_ms(),
        )
        .map_err(|error| error.to_string())?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn models_activate(
    app: AppHandle,
    window: tauri::WebviewWindow,
    service: State<'_, ModelService>,
    profile_id: String,
) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    service
        .manager()?
        .activate_profile(&profile_id, now_ms())
        .map_err(|error| error.to_string())?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn models_remove(
    app: AppHandle,
    window: tauri::WebviewWindow,
    service: State<'_, ModelService>,
    package_id: String,
) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    service
        .manager()?
        .remove_package(&package_id, now_ms())
        .map_err(|error| error.to_string())?;
    changed(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wigigadict_storage::InstalledModel;

    fn row(install_state: &str, bytes: u64) -> InstalledModel {
        InstalledModel {
            package_id: "p".into(),
            engine_family: "whisper".into(),
            model_name: "small".into(),
            model_version: "1".into(),
            license_id: "MIT".into(),
            expected_size: 100,
            install_state: install_state.into(),
            installed_at: None,
            profile_id: None,
            device_kind: None,
            health_state: None,
            is_active: false,
            job_state: None,
            bytes_downloaded: bytes,
        }
    }

    #[test]
    fn a_started_download_shows_as_downloading_before_its_row_exists() {
        // The thread has not written a row yet, but the user pressed the button and must see it.
        assert_eq!(card_state(true, None), "downloading");
        // Nothing running and no row at all: the card simply offers the download.
        assert_eq!(card_state(false, None), "available");
        // A finished install is reported from the row, not from the running flag.
        assert_eq!(card_state(false, Some(&row("installed", 100))), "installed");
    }

    #[test]
    fn an_interrupted_download_reads_as_paused_until_a_thread_picks_it_up() {
        assert_eq!(install_state(&row("downloading", 40), false), "paused");
        assert_eq!(install_state(&row("downloading", 40), true), "downloading");
        assert_eq!(install_state(&row("installed", 100), false), "installed");
        // A removed package offers itself for download again rather than looking broken.
        assert_eq!(install_state(&row("absent", 0), false), "available");
        assert_eq!(install_state(&row("failed", 0), false), "failed");
    }

    #[test]
    fn a_build_without_a_pinned_key_refuses_the_catalog_instead_of_trusting_it() {
        if CATALOG_PUBLIC_KEY_HEX.is_none() {
            let Err(error) = trusted_keys() else {
                panic!("a build with no pinned key must not produce a key ring");
            };
            assert!(error.contains("WIGIGADICT_CATALOG_PUBLIC_KEY"));
            let error = load_catalog(Path::new(".")).expect_err("no key means no catalog");
            assert!(!error.is_empty());
        }
    }

    #[test]
    fn the_checked_in_release_catalog_matches_the_pinned_public_key() {
        if CATALOG_PUBLIC_KEY_HEX.is_some() {
            let catalog = load_catalog(Path::new(env!("CARGO_MANIFEST_DIR")))
                .expect("the checked-in release catalog must verify");
            assert_eq!(catalog.signature_key_id, "wigigadict-catalog-v1");
            assert!(!catalog.entries.is_empty());
        }
    }
}
