use serde::Deserialize;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use wigigadict_protocol::{
    CancelRequest, EngineKind, EngineOutput, LanguageHint, PROTOCOL_VERSION, RuntimeSpec,
    ShellCommand, SidecarEvent, TranscribeRequest, WhisperProfile,
};
use wigigadict_storage::{
    AsrCompletionMetrics, AsrDispatcher, AsrLease, CleanupRepository, DeliveryStatus,
    DiagnosticComponent, DiagnosticEventName, DiagnosticOutcome, DiagnosticStage, EvidenceClass,
    MAX_SESSION_PCM_BYTES, ModelManifest, TranscriptSnapshot,
};

use crate::archive::ArchiveService;
use crate::diagnostics::{DiagnosticService, pipeline_event};
use crate::insertion::{DeliveryRun, WindowsInsertionCoordinator};
use crate::ipc::{SidecarClient, find_sidecar};
use crate::overlay::{OverlayPhase, OverlayService, OverlayStatus};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const EVENT_INTERVAL: Duration = Duration::from_millis(500);
const DATABASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// The ASR worker ships with the application, exactly like the sidecar: it speaks this build's
/// CLI contract. Binding it to a model package instead would mean downloading and storing one
/// identical 54 MB binary per model, and letting those copies drift away from the app.
const WORKER_FILENAME: &str = "wigigadict-asr-worker-x86_64-pc-windows-msvc.exe";
const WORKER_PLAIN_FILENAME: &str = "wigigadict-asr-worker.exe";

#[derive(Clone)]
struct ServiceStatus {
    state: &'static str,
    detail: String,
}

pub struct SidecarRuntime {
    status: Arc<Mutex<ServiceStatus>>,
    shutdown: mpsc::Sender<()>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl SidecarRuntime {
    /// Starts the dispatcher, or returns a degraded runtime when the sidecar cannot be started.
    ///
    /// A missing sidecar must never take the shell down: the packaged layout differs from the
    /// developer layout, and a hard failure here used to kill the process right after the main
    /// window appeared. The shell stays usable and reports the reason through `runtime_status`.
    pub fn start(app: AppHandle, database_path: PathBuf, managed_root: PathBuf) -> Self {
        let (shutdown, shutdown_rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(ServiceStatus {
            state: "ready",
            detail: "ASR dispatcher is waiting for durable audio".into(),
        }));
        let sidecar_path = match find_sidecar() {
            Ok(path) => path,
            Err(_) => {
                return Self::degraded(shutdown, status, "asr_sidecar_missing");
            }
        };
        let worker_status = status.clone();
        let worker = thread::Builder::new()
            .name("wigigadict-asr-dispatcher".into())
            .spawn(move || {
                service_loop(
                    app,
                    database_path,
                    managed_root,
                    sidecar_path,
                    shutdown_rx,
                    worker_status,
                );
            });
        match worker {
            Ok(worker) => Self {
                status,
                shutdown,
                worker: Mutex::new(Some(worker)),
            },
            Err(_) => Self::degraded(shutdown, status, "asr_dispatcher_unavailable"),
        }
    }

    /// Content-free degraded runtime: the shell runs, recognition is reported as unavailable.
    fn degraded(
        shutdown: mpsc::Sender<()>,
        status: Arc<Mutex<ServiceStatus>>,
        reason: &str,
    ) -> Self {
        if let Ok(mut current) = status.lock() {
            *current = ServiceStatus {
                state: "unavailable",
                detail: reason.to_owned(),
            };
        }
        Self {
            status,
            shutdown,
            worker: Mutex::new(None),
        }
    }

    /// `false` when the dispatcher thread does not exist, so no queued attempt can ever run.
    pub fn accepts_work(&self) -> bool {
        self.worker
            .lock()
            .map(|worker| worker.is_some())
            .unwrap_or(false)
    }

    pub fn status(&self) -> crate::RuntimeStatus {
        let current = self
            .status
            .lock()
            .map(|value| value.clone())
            .unwrap_or(ServiceStatus {
                state: "unavailable",
                detail: "ASR status mutex was poisoned".into(),
            });
        crate::RuntimeStatus {
            state: current.state,
            protocol: PROTOCOL_VERSION.to_owned(),
            sidecar: "0.0.1".to_owned(),
            detail: current.detail,
        }
    }
}

impl Drop for SidecarRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Ok(mut worker) = self.worker.lock()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn service_loop(
    app: AppHandle,
    database_path: PathBuf,
    managed_root: PathBuf,
    sidecar_path: PathBuf,
    shutdown: Receiver<()>,
    status: Arc<Mutex<ServiceStatus>>,
) {
    let mut dispatcher = match AsrDispatcher::open(&database_path) {
        Ok(dispatcher) => dispatcher,
        Err(_) => {
            set_status(&status, "unavailable", "ASR storage is unavailable");
            return;
        }
    };
    let mut cleanup = match CleanupRepository::open(&database_path) {
        Ok(cleanup) => cleanup,
        Err(_) => {
            set_status(&status, "unavailable", "Cleanup storage is unavailable");
            return;
        }
    };
    let mut insertion = match WindowsInsertionCoordinator::open(&database_path) {
        Ok(insertion) => insertion,
        Err(_) => {
            set_status(&status, "unavailable", "Delivery storage is unavailable");
            return;
        }
    };
    let owner = format!("desktop-{}", std::process::id());
    loop {
        match shutdown.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }
        let lease = match dispatcher.lease_next(&owner, now_ms()) {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                if let Ok(Some(selection)) = cleanup.cleanup_next_default(now_ms()) {
                    deliver_and_publish(&app, &mut insertion, &selection.selected);
                }
                if shutdown.recv_timeout(POLL_INTERVAL).is_ok() {
                    break;
                }
                continue;
            }
            Err(_) => {
                // One transient queue error used to end this thread for the whole process
                // lifetime: nothing was ever transcribed again until the app was restarted.
                set_status(&status, "unavailable", "ASR queue lease failed");
                if shutdown.recv_timeout(POLL_INTERVAL).is_ok() {
                    break;
                }
                continue;
            }
        };
        set_status(&status, "processing", "Offline transcription is running");
        if let Some(diagnostics) = app.try_state::<DiagnosticService>() {
            diagnostics.record_essential(pipeline_event(
                DiagnosticComponent::Asr,
                DiagnosticEventName::AsrState,
                DiagnosticStage::Lease,
                DiagnosticOutcome::Started,
                &lease.session_id,
                None,
            ));
        }
        publish_overlay(
            &app,
            OverlayStatus::new(
                OverlayPhase::Processing,
                Some(lease.session_id.clone()),
                None,
            ),
        );
        if dispatcher.mark_running(&lease.key, now_ms()).is_err() {
            release_failure_and_publish(&mut dispatcher, &app, &lease, "lease_start_failed", true);
            set_status(&status, "unavailable", "ASR lease could not start");
            continue;
        }
        let request = match build_request(&lease, &managed_root) {
            Ok(request) => {
                if request.runtime.profile == WhisperProfile::CpuT16 && lease.key.generation > 1 {
                    publish_overlay(
                        &app,
                        OverlayStatus::new(
                            OverlayPhase::Processing,
                            Some(lease.session_id.clone()),
                            Some("processing_cpu_fallback"),
                        ),
                    );
                }
                request
            }
            Err(_) => {
                release_failure_and_publish(
                    &mut dispatcher,
                    &app,
                    &lease,
                    "runtime_contract_invalid",
                    false,
                );
                set_status(&status, "unavailable", "Active ASR runtime is invalid");
                continue;
            }
        };
        let should_stop = supervise_lease(
            &mut dispatcher,
            &mut cleanup,
            &mut insertion,
            &app,
            &lease,
            request,
            &sidecar_path,
            &shutdown,
        );
        if should_stop {
            break;
        }
        set_status(
            &status,
            "ready",
            "ASR dispatcher is waiting for durable audio",
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "supervision coordinates one leased attempt across its bounded repositories and IPC"
)]
fn supervise_lease(
    dispatcher: &mut AsrDispatcher,
    cleanup: &mut CleanupRepository,
    insertion: &mut WindowsInsertionCoordinator,
    app: &AppHandle,
    lease: &AsrLease,
    request: TranscribeRequest,
    sidecar_path: &Path,
    shutdown: &Receiver<()>,
) -> bool {
    let mut client = match SidecarClient::start(sidecar_path) {
        Ok(client) => client,
        Err(_) => {
            release_failure_and_publish(dispatcher, app, lease, "sidecar_start_failed", true);
            return false;
        }
    };
    if client
        .send(&ShellCommand::Transcribe(Box::new(request.clone())))
        .is_err()
    {
        release_failure_and_publish(dispatcher, app, lease, "sidecar_write_failed", true);
        return false;
    }
    let started = Instant::now();
    let deadline = Duration::from_millis(request.timeout_ms.saturating_add(5_000));
    let mut last_heartbeat = Instant::now();
    loop {
        match shutdown.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => {
                let _ = client.send(&ShellCommand::Cancel(CancelRequest {
                    request_id: request.request_id.clone(),
                }));
                let _ =
                    dispatcher.release_failure(&lease.key, "application_shutdown", true, now_ms());
                return true;
            }
            Err(TryRecvError::Empty) => {}
        }
        if started.elapsed() >= deadline {
            let _ = client.send(&ShellCommand::Cancel(CancelRequest {
                request_id: request.request_id.clone(),
            }));
            release_failure_and_publish(dispatcher, app, lease, "sidecar_timeout", true);
            return false;
        }
        let event = match client.receive(EVENT_INTERVAL) {
            Ok(Some(event)) => Some(event),
            Ok(None) => None,
            Err(_) => {
                release_failure_and_publish(dispatcher, app, lease, "sidecar_crash", true);
                return false;
            }
        };
        if last_heartbeat.elapsed() >= DATABASE_HEARTBEAT_INTERVAL {
            if dispatcher.heartbeat(&lease.key, now_ms()).is_err() {
                return false;
            }
            last_heartbeat = Instant::now();
        }
        match event {
            Some(SidecarEvent::Accepted(value)) if value.request_id == request.request_id => {}
            Some(SidecarEvent::Heartbeat(value)) if value.request_id == request.request_id => {
                if dispatcher.heartbeat(&lease.key, now_ms()).is_err() {
                    return false;
                }
                last_heartbeat = Instant::now();
            }
            Some(SidecarEvent::Result(value))
                if value.request_id == request.request_id
                    && value.attempt_id == lease.key.attempt_id
                    && value.lease_generation == lease.key.generation =>
            {
                let output = EngineOutput {
                    text: value.text,
                    segments: value.segments,
                };
                if output.validate(request.audio_duration_ms).is_err() {
                    release_failure_and_publish(
                        dispatcher,
                        app,
                        lease,
                        "invalid_worker_output",
                        false,
                    );
                    return false;
                }
                let metrics = AsrCompletionMetrics {
                    inference_ms: value.inference_ms,
                    worker_restarts: lease.key.generation.saturating_sub(1),
                    profile: profile_label(request.runtime.profile).into(),
                };
                let transcript_id = format!("raw-{}", lease.key.attempt_id);
                match dispatcher.complete_raw(
                    &lease.key,
                    &transcript_id,
                    &output.text,
                    &metrics,
                    now_ms(),
                ) {
                    Ok(raw) => match cleanup.cleanup_raw(&raw.transcript_id, now_ms()) {
                        Ok(selection) => {
                            if let Some(archive) = app.try_state::<ArchiveService>() {
                                archive.archive_transcript(
                                    &selection.selected.session_id,
                                    &selection.selected.content,
                                );
                            }
                            deliver_and_publish(app, insertion, &selection.selected);
                        }
                        Err(_) => publish_overlay(
                            app,
                            OverlayStatus::new(
                                OverlayPhase::Error,
                                Some(lease.session_id.clone()),
                                Some("cleanup_failed"),
                            ),
                        ),
                    },
                    Err(_) => publish_overlay(
                        app,
                        OverlayStatus::new(
                            OverlayPhase::Error,
                            Some(lease.session_id.clone()),
                            Some("transcript_commit_failed"),
                        ),
                    ),
                }
                return false;
            }
            Some(SidecarEvent::Error(value))
                if value.request_id.as_deref() == Some(request.request_id.as_str()) =>
            {
                release_failure_and_publish(dispatcher, app, lease, &value.code, value.transient);
                return false;
            }
            Some(SidecarEvent::Cancelled(value)) if value.request_id == request.request_id => {
                release_failure_and_publish(dispatcher, app, lease, "sidecar_cancelled", true);
                return false;
            }
            Some(SidecarEvent::Pong(_)) | None => {}
            Some(_) => {
                release_failure_and_publish(dispatcher, app, lease, "protocol_mismatch", false);
                return false;
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WhisperSettings {
    worker_path: String,
    model_path: String,
    #[serde(default)]
    language: Option<LanguageHint>,
    timeout_ms: u64,
    threads: u16,
}

fn build_request(lease: &AsrLease, managed_root: &Path) -> Result<TranscribeRequest, String> {
    if lease.adapter_type != "transcribe-rs" || lease.adapter_version != "0.3.11" {
        return Err("unsupported adapter".into());
    }
    let settings: WhisperSettings =
        serde_json::from_value(lease.runtime_settings.clone()).map_err(|_| "invalid settings")?;
    let profile = match (lease.device_kind.as_str(), settings.threads) {
        ("vulkan", 0) => WhisperProfile::Vulkan,
        ("cpu", 16) => WhisperProfile::CpuT16,
        _ => return Err("unsupported device profile".into()),
    };
    // ADR-008: the retry generation never reuses the Vulkan profile. A busy GPU
    // (another application holding VRAM) is the dominant worker_exited cause, and
    // repeating the same profile made the retry fail identically. The switch is
    // explicit, not hidden: the overlay reports CPU processing and the attempt
    // metrics record the profile that actually ran.
    let profile = if lease.key.generation > 1 && profile == WhisperProfile::Vulkan {
        WhisperProfile::CpuT16
    } else {
        profile
    };
    let managed_root = managed_root
        .canonicalize()
        .map_err(|_| "managed root unavailable")?;
    let package_root = resolve_managed(&managed_root, &lease.model_storage_key)?;
    let manifest: ModelManifest = serde_json::from_slice(
        &std::fs::read(package_root.join(".wigigadict-manifest.json"))
            .map_err(|_| "manifest unavailable")?,
    )
    .map_err(|_| "manifest invalid")?;
    // The model name is no longer pinned: the signed catalog may offer any ggml Whisper package,
    // and each one is still bound to this lease by profile id and exact settings.
    if manifest.engine_family != "whisper"
        || manifest.runtime.profile_id != lease.runtime_profile_id
        || manifest.runtime.settings != lease.runtime_settings
    {
        return Err("manifest/runtime mismatch".into());
    }
    // The frozen ADR-006 package keeps its original engine variant so the Step 16 baseline stays
    // byte-identical; anything else from the catalog travels as the general ggml engine.
    let engine = if manifest.model_name == "large-v3-turbo-q5" {
        EngineKind::WhisperLargeV3TurboQ5
    } else {
        EngineKind::WhisperGgml
    };
    let model_file = manifest
        .files
        .iter()
        .find(|file| file.path == settings.model_path)
        .ok_or("model missing from manifest")?;
    // A package that ships its own worker keeps using it, so the frozen ADR-006 package produces
    // byte-identical requests. Catalog packages carry weights only and borrow this build's worker.
    let (worker_path, worker_sha256) = match manifest
        .files
        .iter()
        .find(|file| file.path == settings.worker_path)
    {
        Some(file) => (
            resolve_managed(&package_root, &settings.worker_path)?,
            file.sha256.clone(),
        ),
        None => bundled_worker()?,
    };
    let model_path = resolve_managed(&package_root, &settings.model_path)?;
    let audio_path = resolve_managed(&managed_root, &lease.audio_storage_key)?;
    let request = TranscribeRequest {
        request_id: lease.key.attempt_id.clone(),
        attempt_id: lease.key.attempt_id.clone(),
        lease_generation: lease.key.generation,
        audio_path: unicode_path(&audio_path)?,
        audio_sha256: lease.audio_sha256.clone(),
        audio_byte_size: lease.audio_byte_size,
        audio_duration_ms: lease.duration_ms,
        language: settings.language.unwrap_or(LanguageHint::Auto),
        timeout_ms: settings.timeout_ms,
        runtime: RuntimeSpec {
            engine,
            worker_path: unicode_path(&worker_path)?,
            worker_sha256,
            model_path: unicode_path(&model_path)?,
            model_sha256: model_file.sha256.clone(),
            profile,
        },
    };
    request.validate().map_err(|error| error.to_string())?;
    if request.audio_byte_size > MAX_SESSION_PCM_BYTES {
        return Err("audio exceeds admission limit".into());
    }
    Ok(request)
}

/// Locates the worker that ships with this build and hashes it once per process.
///
/// ponytail: the expected hash is computed from the same file the sidecar then verifies, so the
/// check proves the binary did not change mid-session, not that it is the one we shipped. The
/// shipped worker's integrity comes from the installer signature, exactly like the sidecar's.
/// Pin a build-time hash here if the worker ever ships from somewhere less trusted.
fn bundled_worker() -> Result<(PathBuf, String), String> {
    static WORKER: OnceLock<Result<(PathBuf, String), String>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let path = worker_candidates()
                .into_iter()
                .find(|candidate| candidate.is_file())
                .ok_or("bundled ASR worker is missing")?;
            let digest = sha256_file(&path)?;
            Ok((path, digest))
        })
        .clone()
}

fn worker_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("WIGIGADICT_ASR_WORKER") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        for name in [WORKER_FILENAME, WORKER_PLAIN_FILENAME] {
            paths.push(directory.join(name));
            paths.push(directory.join("resources").join(name));
            paths.push(directory.join("binaries").join(name));
        }
    }
    for name in [WORKER_FILENAME, WORKER_PLAIN_FILENAME] {
        paths.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join(name),
        );
    }
    paths
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|_| "worker is unreadable")?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|_| "worker is unreadable")?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn resolve_managed(root: &Path, key: &str) -> Result<PathBuf, String> {
    let relative = Path::new(key);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("managed path is not normalized".into());
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "managed path missing")?;
    if !path.starts_with(root) || !path.is_file() && !path.is_dir() {
        return Err("managed path escaped root".into());
    }
    Ok(path)
}

fn unicode_path(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "runtime path is not Unicode".into())
}

fn profile_label(profile: WhisperProfile) -> &'static str {
    match profile {
        WhisperProfile::Vulkan => "gpu",
        WhisperProfile::CpuT16 => "cpu-t16",
    }
}

fn release_failure_and_publish(
    dispatcher: &mut AsrDispatcher,
    app: &AppHandle,
    lease: &AsrLease,
    error_code: &str,
    transient: bool,
) {
    let (phase, reason) =
        match dispatcher.release_failure(&lease.key, error_code, transient, now_ms()) {
            Ok(true) => (OverlayPhase::Processing, "retry_scheduled"),
            Ok(false) => (OverlayPhase::Error, error_code),
            Err(_) => (OverlayPhase::Error, "asr_state_conflict"),
        };
    if let Some(diagnostics) = app.try_state::<DiagnosticService>() {
        diagnostics.record_essential(pipeline_event(
            DiagnosticComponent::Asr,
            DiagnosticEventName::AsrState,
            DiagnosticStage::Transcribe,
            if matches!(phase, OverlayPhase::Processing) {
                DiagnosticOutcome::Recovered
            } else {
                DiagnosticOutcome::Failed
            },
            &lease.session_id,
            Some(reason),
        ));
    }
    publish_overlay(
        app,
        OverlayStatus::new(phase, Some(lease.session_id.clone()), Some(reason)),
    );
}
fn deliver_and_publish(
    app: &AppHandle,
    insertion: &mut WindowsInsertionCoordinator,
    transcript: &TranscriptSnapshot,
) {
    let (status, evidence, error_code) = match insertion.deliver_initial(transcript) {
        Ok(DeliveryRun::Completed(receipt)) => (
            receipt.status,
            Some(receipt.evidence_class),
            receipt.error_code,
        ),
        Ok(DeliveryRun::Existing(status)) => (status, None, None),
        Err(_) => {
            publish_overlay(
                app,
                OverlayStatus::new(
                    OverlayPhase::Error,
                    Some(transcript.session_id.clone()),
                    Some("delivery_failed"),
                ),
            );
            return;
        }
    };
    let (phase, reason) = match status {
        DeliveryStatus::Pending => (OverlayPhase::Processing, None),
        DeliveryStatus::Delivered => (OverlayPhase::Delivered, None),
        // Uncertain covers very different owner situations: an empty transcript
        // (nothing to insert), a complete transport without confirming evidence
        // (text almost certainly landed), and genuinely ambiguous failures. The
        // overlay names them apart; the evidence policy itself is unchanged.
        DeliveryStatus::Uncertain => (
            OverlayPhase::Uncertain,
            Some(if error_code.as_deref() == Some("empty_transcript") {
                "empty_transcript"
            } else if evidence == Some(EvidenceClass::TransportOnly) {
                "delivery_transport_only"
            } else {
                "delivery_unconfirmed"
            }),
        ),
        DeliveryStatus::Failed => (OverlayPhase::Error, Some("delivery_failed")),
    };
    if let Some(diagnostics) = app.try_state::<DiagnosticService>() {
        let outcome = match status {
            DeliveryStatus::Pending => DiagnosticOutcome::Started,
            DeliveryStatus::Delivered => DiagnosticOutcome::Succeeded,
            DeliveryStatus::Uncertain => DiagnosticOutcome::Uncertain,
            DeliveryStatus::Failed => DiagnosticOutcome::Failed,
        };
        let focus_failure = matches!(status, DeliveryStatus::Uncertain | DeliveryStatus::Failed);
        diagnostics.record_essential(pipeline_event(
            DiagnosticComponent::Delivery,
            if focus_failure {
                DiagnosticEventName::FocusCheck
            } else {
                DiagnosticEventName::DeliveryState
            },
            if focus_failure {
                DiagnosticStage::TargetCheck
            } else {
                DiagnosticStage::Commit
            },
            outcome,
            &transcript.session_id,
            reason,
        ));
    }
    publish_overlay(
        app,
        OverlayStatus::new(phase, Some(transcript.session_id.clone()), reason),
    );
}

fn publish_overlay(app: &AppHandle, status: OverlayStatus) {
    if let Some(overlay) = app.try_state::<OverlayService>() {
        overlay.publish_pipeline(app, status);
    }
}
fn set_status(status: &Arc<Mutex<ServiceStatus>>, state: &'static str, detail: &str) {
    if let Ok(mut current) = status.lock() {
        current.state = state;
        current.detail = detail.into();
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_frozen_personal_mvp_profiles_are_mapped() {
        assert_eq!(profile_label(WhisperProfile::Vulkan), "gpu");
        assert_eq!(profile_label(WhisperProfile::CpuT16), "cpu-t16");
    }

    #[test]
    fn managed_path_rejects_traversal_before_filesystem_access() {
        let root = std::env::temp_dir();
        assert!(resolve_managed(&root, "../escape.bin").is_err());
        assert!(resolve_managed(&root, r"C:\escape.bin").is_err());
    }

    #[test]
    fn signed_manifest_materializes_any_catalog_whisper_but_no_other_engine() {
        use wigigadict_storage::{AsrLeaseKey, ManifestFile, RuntimeManifest};

        let root = std::env::temp_dir().join(format!(
            "wigigadict-step10-request-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let package = root.join("installed/model-1");
        let audio = root.join("audio");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::create_dir_all(&audio).unwrap();
        std::fs::write(package.join("worker.exe"), b"worker").unwrap();
        std::fs::write(package.join("model.bin"), b"model").unwrap();
        std::fs::write(audio.join("one.wav"), b"audio").unwrap();
        let settings = serde_json::json!({
            "worker_path": "worker.exe",
            "model_path": "model.bin",
            "language": "auto",
            "timeout_ms": 30_000,
            "threads": 0
        });
        let manifest = ModelManifest {
            schema_version: 1,
            package_id: "model-1".into(),
            engine_family: "whisper".into(),
            model_name: "large-v3-turbo-q5".into(),
            model_version: "1".into(),
            release_sequence: 1,
            source_uri: "https://example.invalid/model".into(),
            license_id: "MIT".into(),
            expected_size: 11,
            signature_key_id: "key-1".into(),
            minimum_manager_version: 1,
            expires_at_ms: i64::MAX,
            compatibility_abi: "wigigadict-model-abi-v1".into(),
            files: vec![
                ManifestFile {
                    path: "worker.exe".into(),
                    size: 6,
                    sha256: "a".repeat(64),
                    download_uri: None,
                },
                ManifestFile {
                    path: "model.bin".into(),
                    size: 5,
                    sha256: "b".repeat(64),
                    download_uri: None,
                },
            ],
            runtime: RuntimeManifest {
                profile_id: "runtime-1".into(),
                profile_version: 1,
                adapter_type: "transcribe-rs".into(),
                adapter_version: "0.3.11".into(),
                device_kind: "vulkan".into(),
                device_id: None,
                settings: settings.clone(),
                probe_file: "model.bin".into(),
            },
        };
        std::fs::write(
            package.join(".wigigadict-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut lease = AsrLease {
            key: AsrLeaseKey {
                attempt_id: "attempt-1".into(),
                owner: "worker-1".into(),
                generation: 1,
            },
            session_id: "session-1".into(),
            audio_storage_key: "audio/one.wav".into(),
            audio_sha256: "c".repeat(64),
            audio_byte_size: 5,
            duration_ms: 1_000,
            runtime_profile_id: "runtime-1".into(),
            adapter_type: "transcribe-rs".into(),
            adapter_version: "0.3.11".into(),
            device_kind: "vulkan".into(),
            runtime_settings: settings,
            model_storage_key: "installed/model-1".into(),
            lease_expires_at: 30_000,
        };
        let request = build_request(&lease, &root).unwrap();
        assert_eq!(request.runtime.profile, WhisperProfile::Vulkan);
        assert_eq!(request.runtime.worker_sha256, "a".repeat(64));
        assert_eq!(request.runtime.engine, EngineKind::WhisperLargeV3TurboQ5);
        // ADR-008: the retry generation must leave the Vulkan profile behind.
        lease.key.generation = 2;
        let retry = build_request(&lease, &root).unwrap();
        assert_eq!(retry.runtime.profile, WhisperProfile::CpuT16);
        lease.key.generation = 1;

        // Any other ggml model from the signed catalog must build a request instead of being
        // rejected by name, and it travels as the general engine variant.
        let write_manifest = |value: &ModelManifest| {
            std::fs::write(
                package.join(".wigigadict-manifest.json"),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap();
        };
        let mut catalog = manifest.clone();
        catalog.model_name = "small".into();
        write_manifest(&catalog);
        let general = build_request(&lease, &root).unwrap();
        assert_eq!(general.runtime.engine, EngineKind::WhisperGgml);

        // A different engine family is still refused: it needs another worker entirely.
        let mut foreign = catalog.clone();
        foreign.engine_family = "gigaam".into();
        write_manifest(&foreign);
        assert!(build_request(&lease, &root).is_err());

        // A catalog package carries weights only. It must no longer be rejected for lacking a
        // worker; the worker now comes from this build.
        let mut weights_only = catalog.clone();
        weights_only.files.retain(|file| file.path == "model.bin");
        weights_only.expected_size = 5;
        write_manifest(&weights_only);
        match build_request(&lease, &root) {
            Ok(request) => assert!(request.runtime.worker_path.ends_with(".exe")),
            // Where this build has no bundled worker, the failure names the worker, never the
            // manifest: the package itself is valid.
            Err(error) => assert_eq!(error, "bundled ASR worker is missing"),
        }

        write_manifest(&manifest);
        lease.runtime_settings["threads"] = serde_json::json!(99);
        assert!(build_request(&lease, &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
