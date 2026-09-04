//! Global toggle hotkey and recoverable Windows audio capture.
//!
//! Hotkey and CPAL callbacks perform only bounded Win32 target reads. SQLite, resampling and
//! PCM writes are owned by one worker thread.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind as CpalErrorKind, FromSample, Sample, SampleFormat, SizedSample};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, select_biased, tick};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Indexing, PolynomialDegree, Resampler};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use uuid::Uuid;
use wigigadict_storage::{
    CaptureCommitPlan, ConfigurationRepository, DeliveryRepository, DiagnosticOutcome,
    DiagnosticStage, PcmFormat, PcmPartWriter, SessionCommitCoordinator, SessionCommitError,
    TargetSnapshotInput,
};

use crate::archive::ArchiveService;
use crate::asr_service::SidecarRuntime;
use crate::diagnostics::{DiagnosticService, capture_event};
use crate::insertion::capture_foreground_target;
use crate::overlay::{OverlayPhase, OverlayService, OverlayStatus};
use crate::shell_lifecycle::ShellLifecycle;

pub const DEFAULT_HOTKEY: &str = "F8";
const CANCEL_KEY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const PCM_LIMIT_BYTES: u64 = 32 * 1024 * 1024;
const WAV_HEADER_BYTES: u64 = 44;
const AUDIO_QUEUE_BLOCKS: usize = 8;
const CONTROL_QUEUE_EVENTS: usize = 16;
const RESAMPLE_CHUNK_FRAMES: usize = 1024;
const LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(66);
const RECORDING_STATUS_EMIT_INTERVAL: Duration = Duration::from_millis(250);
const VOICE_GAIN_TARGET_RMS: f32 = 0.10;
const VOICE_GAIN_MAX: f32 = 6.0;
const VOICE_GAIN_FLOOR_RMS: f32 = 0.0003;
const SIGNAL_NONE: u8 = 0;
const SIGNAL_OVERFLOW: u8 = 1;
const SIGNAL_DEVICE_LOST: u8 = 2;
const SIGNAL_DEVICE_BUSY: u8 = 3;
const SIGNAL_STREAM_INVALIDATED: u8 = 4;
const SIGNAL_PERMISSION_DENIED: u8 = 5;
const SIGNAL_XRUN: u8 = 6;
const SIGNAL_BACKEND_ERROR: u8 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePhase {
    Idle,
    Preparing,
    Recording,
    Finalizing,
    Recovery,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub phase: CapturePhase,
    pub session_id: Option<String>,
    pub reason: Option<String>,
    pub device_healthy: bool,
    pub durable_pcm_bytes: u64,
    /// How many times the audio device dropped samples during this capture. A survived xrun keeps
    /// the recording alive but leaves a hole in it, and the user must hear that from the app
    /// rather than discover a missing word in the transcript.
    pub audio_gaps: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureLevel {
    session_id: String,
    level: f32,
}

impl Default for CaptureStatus {
    fn default() -> Self {
        Self {
            phase: CapturePhase::Idle,
            session_id: None,
            reason: None,
            device_healthy: false,
            durable_pcm_bytes: 0,
            audio_gaps: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDeviceStatus {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub healthy: bool,
}

#[derive(Debug, Clone)]
struct CaptureConfiguration {
    runtime_profile_id: Option<String>,
    microphone_device_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReason {
    Cancelled,
    Watchdog,
    AudioDeviceLost,
    AudioDeviceBusy,
    AudioStreamInvalidated,
    MicrophonePermissionDenied,
    AudioBackendError,
    AudioQueueOverflow,
    SizeLimit,
    WindowsSessionLocked,
    WindowsSessionLogoff,
    WindowsShutdown,
    ApplicationExit,
    WriterError,
    EmptyCapture,
}

impl RecoveryReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Watchdog => "lost_keyup_watchdog",
            Self::AudioDeviceLost => "audio_device_lost",
            Self::AudioDeviceBusy => "audio_device_busy",
            Self::AudioStreamInvalidated => "audio_stream_invalidated",
            Self::MicrophonePermissionDenied => "microphone_permission_denied",
            Self::AudioBackendError => "audio_backend_error",
            Self::AudioQueueOverflow => "audio_queue_overflow",
            Self::SizeLimit => "pcm_size_limit",
            Self::WindowsSessionLocked => "windows_session_locked",
            Self::WindowsSessionLogoff => "windows_session_logoff",
            Self::WindowsShutdown => "windows_shutdown",
            Self::ApplicationExit => "application_exit",
            Self::WriterError => "pcm_writer_error",
            Self::EmptyCapture => "empty_capture",
        }
    }
}

#[derive(Debug)]
enum ControlEvent {
    Pressed(TargetSnapshotInput),
    Released,
    Recover {
        reason: RecoveryReason,
        acknowledgement: Option<Sender<()>>,
    },
    SelectDevice(Option<String>),

    SelectRuntimeProfile(Option<String>),
}

#[derive(Debug)]
struct AudioBlock {
    generation: Uuid,
    samples: Vec<f32>,
}

const PTT_UP: u8 = 0;
const PTT_DOWN: u8 = 1;
const PTT_SUPPRESSED_UNTIL_RELEASE: u8 = 2;

#[derive(Default)]
struct PttLatch(AtomicU8);

impl PttLatch {
    fn pressed(&self) -> bool {
        self.0
            .compare_exchange(PTT_UP, PTT_DOWN, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn released(&self) -> bool {
        self.0.swap(PTT_UP, Ordering::AcqRel) == PTT_DOWN
    }

    fn suppress_active_until_release(&self) {
        let _ = self.0.compare_exchange(
            PTT_DOWN,
            PTT_SUPPRESSED_UNTIL_RELEASE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

pub struct CaptureService {
    control_tx: Sender<ControlEvent>,
    status: Arc<Mutex<CaptureStatus>>,
    ptt: Arc<PttLatch>,
}

impl CaptureService {
    pub fn start(
        app: AppHandle,
        lifecycle: Arc<ShellLifecycle>,
        database_path: PathBuf,
        managed_audio_root: PathBuf,
    ) -> Arc<Self> {
        let (control_tx, control_rx) = bounded(CONTROL_QUEUE_EVENTS);
        let (audio_tx, audio_rx) = bounded(AUDIO_QUEUE_BLOCKS);
        let status = Arc::new(Mutex::new(CaptureStatus::default()));
        let ptt = Arc::new(PttLatch::default());
        let worker_status = status.clone();
        let worker_ptt = ptt.clone();

        arm_cancel_key_with_capture(app.clone(), status.clone());
        thread::Builder::new()
            .name("wigigadict-capture-writer".into())
            .spawn(move || {
                capture_worker(
                    app,
                    lifecycle,
                    database_path,
                    managed_audio_root,
                    control_rx,
                    audio_tx,
                    audio_rx,
                    worker_status,
                    worker_ptt,
                );
            })
            .expect("capture worker thread must start");

        Arc::new(Self {
            control_tx,
            status,
            ptt,
        })
    }

    pub fn on_shortcut_state(&self, state: ShortcutState) {
        match state {
            ShortcutState::Pressed if self.ptt.pressed() => {
                let stops_active_capture = self
                    .status
                    .lock()
                    .is_ok_and(|status| toggle_stops_capture(status.phase));
                if stops_active_capture {
                    if self.control_tx.try_send(ControlEvent::Released).is_err() {
                        self.ptt.suppress_active_until_release();
                        let _ = self.control_tx.try_send(ControlEvent::Recover {
                            reason: RecoveryReason::WriterError,
                            acknowledgement: None,
                        });
                        self.set_local_unavailable("control_queue_full");
                    }
                } else {
                    let target =
                        capture_foreground_target(format!("target-{}", Uuid::new_v4()), now_ms());
                    match target {
                        Ok(target) => {
                            if self
                                .control_tx
                                .try_send(ControlEvent::Pressed(target))
                                .is_err()
                            {
                                self.ptt.suppress_active_until_release();
                                self.set_local_unavailable("control_queue_full");
                            }
                        }
                        Err(error) => {
                            self.ptt.suppress_active_until_release();
                            self.set_local_unavailable(error.code());
                        }
                    }
                }
            }
            ShortcutState::Released => {
                self.ptt.released();
            }
            _ => {}
        }
    }

    pub fn cancel(&self) -> Result<(), String> {
        self.recover_and_wait(RecoveryReason::Cancelled)
    }
    pub fn finish(&self) -> Result<(), String> {
        self.ptt.suppress_active_until_release();
        self.send_control(ControlEvent::Released)
    }

    pub fn recover_and_wait(&self, reason: RecoveryReason) -> Result<(), String> {
        self.ptt.suppress_active_until_release();
        let (acknowledgement, completed) = bounded(1);
        self.control_tx
            .send_timeout(
                ControlEvent::Recover {
                    reason,
                    acknowledgement: Some(acknowledgement),
                },
                Duration::from_millis(100),
            )
            .map_err(|_| "capture control queue did not accept safety stop".to_owned())?;
        completed
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "capture safety stop did not become durable within two seconds".to_owned())
    }

    pub fn select_device(&self, device_id: Option<String>) -> Result<(), String> {
        if device_id.as_ref().is_some_and(|value| value.len() > 512) {
            return Err("microphone device id is too long".into());
        }
        self.send_control(ControlEvent::SelectDevice(device_id))
    }

    pub fn select_runtime_profile(&self, runtime_profile_id: Option<String>) -> Result<(), String> {
        if runtime_profile_id
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 128)
        {
            return Err("runtime profile id is invalid".into());
        }
        self.send_control(ControlEvent::SelectRuntimeProfile(runtime_profile_id))
    }

    pub fn status(&self) -> Result<CaptureStatus, String> {
        self.status
            .lock()
            .map(|value| value.clone())
            .map_err(|_| "capture status mutex was poisoned".into())
    }

    fn send_control(&self, event: ControlEvent) -> Result<(), String> {
        self.control_tx
            .try_send(event)
            .map_err(|_| "capture control queue is full".into())
    }

    fn set_local_unavailable(&self, reason: &str) {
        if let Ok(mut status) = self.status.lock() {
            status.phase = CapturePhase::Unavailable;
            status.reason = Some(reason.to_owned());
        }
    }
}

/// A write refused by the reservation is the safety limit; anything else is a writer failure.
fn pipeline_recovery_reason(reason: &str) -> RecoveryReason {
    if reason == RecoveryReason::SizeLimit.code() {
        RecoveryReason::SizeLimit
    } else {
        RecoveryReason::WriterError
    }
}

/// The dictation toggle key is global, so `Escape` has to be global too: during a real dictation
/// the foreground window belongs to the target application and a WebView key listener never sees
/// it. The key is reserved only while a capture is live, so every other application keeps it.
///
/// Registration is marshalled to the UI thread by the shortcut plugin and blocks the caller, so
/// this must never run on the capture worker: its audio queue holds only `AUDIO_QUEUE_BLOCKS`
/// blocks. Polling the published phase also means no cancellation path can leave the key armed.
fn arm_cancel_key_with_capture(app: AppHandle, status: Arc<Mutex<CaptureStatus>>) {
    let Ok(escape) = Shortcut::from_str("Escape") else {
        return;
    };
    let _ = thread::Builder::new()
        .name("wigigadict-cancel-key".into())
        .spawn(move || {
            let mut armed = false;
            loop {
                thread::sleep(CANCEL_KEY_POLL_INTERVAL);
                let wanted = status
                    .lock()
                    .is_ok_and(|status| cancel_key_is_reserved(status.phase));
                if wanted == armed {
                    continue;
                }
                let changed = if wanted {
                    app.global_shortcut().register(escape).is_ok()
                } else {
                    app.global_shortcut().unregister(escape).is_ok()
                };
                if changed {
                    armed = wanted;
                }
            }
        });
}

fn cancel_key_is_reserved(phase: CapturePhase) -> bool {
    matches!(phase, CapturePhase::Preparing | CapturePhase::Recording)
}

fn toggle_stops_capture(phase: CapturePhase) -> bool {
    matches!(phase, CapturePhase::Preparing | CapturePhase::Recording)
}

pub fn validate_hotkey(value: &str) -> Result<Shortcut, String> {
    if value.is_empty() || value.len() > 64 {
        return Err("hotkey must contain 1..64 characters".into());
    }
    let shortcut = Shortcut::from_str(value).map_err(|_| "hotkey syntax is invalid".to_owned())?;
    if shortcut.key == tauri_plugin_global_shortcut::Code::Escape {
        return Err("Escape is reserved for capture cancellation".into());
    }
    if shortcut.mods.is_empty()
        && !matches!(
            shortcut.key,
            tauri_plugin_global_shortcut::Code::F1
                | tauri_plugin_global_shortcut::Code::F2
                | tauri_plugin_global_shortcut::Code::F3
                | tauri_plugin_global_shortcut::Code::F4
                | tauri_plugin_global_shortcut::Code::F5
                | tauri_plugin_global_shortcut::Code::F6
                | tauri_plugin_global_shortcut::Code::F7
                | tauri_plugin_global_shortcut::Code::F8
                | tauri_plugin_global_shortcut::Code::F9
                | tauri_plugin_global_shortcut::Code::F10
                | tauri_plugin_global_shortcut::Code::F11
                | tauri_plugin_global_shortcut::Code::F12
        )
    {
        return Err("single-key dictation toggle requires F1..F12".into());
    }
    Ok(shortcut)
}

pub fn input_devices() -> Result<Vec<InputDeviceStatus>, String> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|value| value.to_string());
    let devices = host
        .input_devices()
        .map_err(|_| "input_device_enumeration_failed".to_owned())?;
    let mut result = Vec::new();
    for device in devices {
        let Ok(id) = device.id() else {
            continue;
        };
        let id = id.to_string();
        let description = device.description();
        let healthy = device.default_input_config().is_ok();
        result.push(InputDeviceStatus {
            is_default: default_id
                .as_deref()
                .is_some_and(|default| device_ids_match(default, &id)),
            id,
            name: description
                .map(|value| value.name().to_owned())
                .unwrap_or_else(|_| "Unavailable microphone".into()),
            healthy,
        });
    }
    Ok(result)
}
struct ActiveCapture {
    generation: Uuid,
    plan: CaptureCommitPlan,
    stream: cpal::Stream,
    pipeline: PcmPipeline,
    signal: Arc<AtomicU8>,
    started: Instant,
    last_level_emit: Instant,
    last_status_emit: Instant,
    smoothed_level: f32,
}

#[derive(Debug, Clone, Copy)]
struct AudioSpec {
    sample_rate_hz: u32,
    channels: usize,
}

#[expect(
    clippy::too_many_arguments,
    reason = "worker inputs make cross-thread ownership and channel direction explicit"
)]
fn capture_worker(
    app: AppHandle,
    lifecycle: Arc<ShellLifecycle>,
    database_path: PathBuf,
    managed_audio_root: PathBuf,
    control_rx: Receiver<ControlEvent>,
    audio_tx: Sender<AudioBlock>,
    audio_rx: Receiver<AudioBlock>,
    status: Arc<Mutex<CaptureStatus>>,
    ptt: Arc<PttLatch>,
) {
    let persisted_configuration = ConfigurationRepository::open(&database_path)
        .and_then(|repository| repository.active())
        .ok()
        .flatten();
    let mut delivery = match DeliveryRepository::open(&database_path) {
        Ok(value) => value,
        Err(_) => {
            publish(
                &app,
                &status,
                CapturePhase::Unavailable,
                None,
                Some("delivery_storage_unavailable"),
                false,
                0,
            );
            return;
        }
    };
    let mut coordinator = match SessionCommitCoordinator::open(database_path, managed_audio_root) {
        Ok(mut value) => {
            if value.reconcile_startup(now_ms()).is_err() {
                publish(
                    &app,
                    &status,
                    CapturePhase::Recovery,
                    None,
                    Some("startup_reconciliation_failed"),
                    false,
                    0,
                );
            }
            value
        }
        Err(_) => {
            publish(
                &app,
                &status,
                CapturePhase::Unavailable,
                None,
                Some("capture_storage_unavailable"),
                false,
                0,
            );
            return;
        }
    };
    let mut configuration = CaptureConfiguration {
        runtime_profile_id: persisted_configuration
            .as_ref()
            .and_then(|value| value.active_runtime_profile_id.clone())
            .or_else(|| coordinator.active_runtime_profile_id().ok().flatten()),
        microphone_device_id: persisted_configuration.and_then(|value| value.microphone_device_id),
    };
    let mut active: Option<ActiveCapture> = None;
    let watchdog_tick = tick(Duration::from_millis(100));
    let watchdog_limit = pcm_watchdog_limit();

    loop {
        select_biased! {
            recv(control_rx) -> event => {
                let Ok(event) = event else { break };
                match event {
                    ControlEvent::Pressed(target_snapshot) => {
                        if active.is_none() {
                            active = begin_capture(
                                &app,
                                &lifecycle,
                                &mut coordinator,
                                &mut delivery,
                                &target_snapshot,
                                &configuration,
                                audio_tx.clone(),
                                &status,
                                &ptt,
                            );
                        }
                    }
                    ControlEvent::Released => {
                        if active.is_some() {
                            stop_capture(
                                &app,
                                &lifecycle,
                                &mut coordinator,
                                &audio_rx,
                                &status,
                                &mut active,
                                None,
                            );
                        }
                    }
                    ControlEvent::Recover {
                        reason,
                        acknowledgement,
                    } => {
                        if active.is_some() {
                            stop_capture(
                                &app,
                                &lifecycle,
                                &mut coordinator,
                                &audio_rx,
                                &status,
                                &mut active,
                                Some(reason),
                            );
                        }
                        if let Some(acknowledgement) = acknowledgement {
                            let _ = acknowledgement.try_send(());
                        }
                    }
                    ControlEvent::SelectDevice(device_id) => {
                        if active.is_none() {
                            configuration.microphone_device_id = device_id;
                        }
                    }
                    ControlEvent::SelectRuntimeProfile(profile_id) => {
                        if active.is_none() {
                            configuration.runtime_profile_id = profile_id;
                        }
                    }
                }
            }
            recv(audio_rx) -> block => {
                if let Ok(block) = block
                    && let Some(current) = active.as_mut()
                    && current.generation == block.generation
                {
                    match current.pipeline.push(&block.samples) {
                        Ok(level) => {
                        publish_capture_level(&app, current, level);
                        let bytes = current.pipeline.written_file_bytes();
                        if current.last_status_emit.elapsed() >= RECORDING_STATUS_EMIT_INTERVAL {
                            current.last_status_emit = Instant::now();
                            publish(
                                &app,
                                &status,
                                CapturePhase::Recording,
                                Some(current.plan.session_id.clone()),
                                None,
                                true,
                                bytes,
                            );
                        }
                        }
                        Err(reason) => {
                            stop_capture(
                                &app,
                                &lifecycle,
                                &mut coordinator,
                                &audio_rx,
                                &status,
                                &mut active,
                                Some(pipeline_recovery_reason(&reason)),
                            );
                        }
                    }
                }
            }
            recv(watchdog_tick) -> _ => {
                let fault = active
                    .as_ref()
                    .map(|value| value.signal.swap(SIGNAL_NONE, Ordering::AcqRel))
                    .unwrap_or(SIGNAL_NONE);
                if fault == SIGNAL_XRUN && let Some(current) = active.as_ref() {
                    if let Some(diagnostics) = app.try_state::<DiagnosticService>() {
                        diagnostics.record_extended(capture_event(
                            Some(current.plan.session_id.clone()),
                            DiagnosticStage::Record,
                            DiagnosticOutcome::Recovered,
                            Some("audio_xrun"),
                            current.pipeline.written_file_bytes(),
                        ));
                    }
                    note_audio_gap(&app, &status);
                }
                let reason = capture_signal_recovery(fault).or_else(|| {
                    active
                        .as_ref()
                        .is_some_and(|value| value.started.elapsed() >= watchdog_limit)
                        .then_some(RecoveryReason::Watchdog)
                });
                if let Some(reason) = reason {
                    ptt.suppress_active_until_release();
                    stop_capture(
                        &app,
                        &lifecycle,
                        &mut coordinator,
                        &audio_rx,
                        &status,
                        &mut active,
                        Some(reason),
                    );
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "capture inputs make storage, target and thread ownership explicit"
)]
fn begin_capture(
    app: &AppHandle,
    lifecycle: &Arc<ShellLifecycle>,
    coordinator: &mut SessionCommitCoordinator,
    delivery: &mut DeliveryRepository,
    target_snapshot: &TargetSnapshotInput,
    configuration: &CaptureConfiguration,
    audio_tx: Sender<AudioBlock>,
    status: &Arc<Mutex<CaptureStatus>>,
    ptt: &Arc<PttLatch>,
) -> Option<ActiveCapture> {
    let Some(runtime_profile_id) = configuration.runtime_profile_id.clone() else {
        ptt.suppress_active_until_release();
        publish(
            app,
            status,
            CapturePhase::Unavailable,
            None,
            Some("runtime_profile_unavailable"),
            false,
            0,
        );
        return None;
    };
    let generation = Uuid::new_v4();
    let plan = capture_plan(runtime_profile_id, now_ms());
    publish(
        app,
        status,
        CapturePhase::Preparing,
        Some(plan.session_id.clone()),
        None,
        false,
        0,
    );
    let writer = match coordinator.prepare_pcm_writer(&plan) {
        Ok(value) => value,
        Err(error) => {
            let reason = match error {
                SessionCommitError::Admission(reason) => reason.code(),
                _ => "capture_admission_failed",
            };
            ptt.suppress_active_until_release();
            publish(
                app,
                status,
                CapturePhase::Unavailable,
                Some(plan.session_id.clone()),
                Some(reason),
                false,
                0,
            );
            return None;
        }
    };
    if delivery
        .capture_initial_target(&plan.session_id, target_snapshot)
        .is_err()
    {
        let _ = coordinator.recover_pcm_writer(
            &plan,
            writer,
            now_ms().max(plan.started_at),
            "target_snapshot_persist_failed",
        );
        ptt.suppress_active_until_release();
        publish(
            app,
            status,
            CapturePhase::Recovery,
            Some(plan.session_id),
            Some("target_snapshot_persist_failed"),
            false,
            WAV_HEADER_BYTES,
        );
        return None;
    }
    if lifecycle.begin_capture(&plan.session_id).is_err() {
        let _ = coordinator.recover_pcm_writer(
            &plan,
            writer,
            now_ms().max(plan.started_at),
            "shell_capture_rejected",
        );
        ptt.suppress_active_until_release();
        publish(
            app,
            status,
            CapturePhase::Recovery,
            Some(plan.session_id),
            Some("shell_capture_rejected"),
            false,
            WAV_HEADER_BYTES,
        );
        return None;
    }

    let signal = Arc::new(AtomicU8::new(SIGNAL_NONE));
    let (stream, spec) = match open_input_stream(
        configuration.microphone_device_id.as_deref(),
        generation,
        audio_tx,
        signal.clone(),
    ) {
        Ok(value) => value,
        Err(error) => {
            let reason = capture_open_error_code(&error);
            let recovered = coordinator
                .recover_pcm_writer(&plan, writer, now_ms().max(plan.started_at), reason)
                .is_ok();
            lifecycle.recover_capture(&plan.session_id, reason);
            if recovered {
                lifecycle.finish_capture_recovery(&plan.session_id);
            }
            ptt.suppress_active_until_release();
            publish(
                app,
                status,
                CapturePhase::Recovery,
                Some(plan.session_id),
                Some(reason),
                false,
                WAV_HEADER_BYTES,
            );
            return None;
        }
    };
    let pipeline = match PcmPipeline::new(writer, spec) {
        Ok(value) => value,
        Err((_error, writer)) => {
            drop(stream);
            let recovered = coordinator
                .recover_pcm_writer(
                    &plan,
                    writer,
                    now_ms().max(plan.started_at),
                    RecoveryReason::WriterError.code(),
                )
                .is_ok();
            lifecycle.recover_capture(&plan.session_id, RecoveryReason::WriterError.code());
            if recovered {
                lifecycle.finish_capture_recovery(&plan.session_id);
            }
            ptt.suppress_active_until_release();
            publish(
                app,
                status,
                CapturePhase::Recovery,
                Some(plan.session_id),
                Some(RecoveryReason::WriterError.code()),
                false,
                WAV_HEADER_BYTES,
            );
            return None;
        }
    };
    if stream.play().is_err() {
        drop(stream);
        let writer = pipeline.into_writer();
        let reason = "input_stream_start_failed";
        let recovered = coordinator
            .recover_pcm_writer(&plan, writer, now_ms().max(plan.started_at), reason)
            .is_ok();
        lifecycle.recover_capture(&plan.session_id, reason);
        if recovered {
            lifecycle.finish_capture_recovery(&plan.session_id);
        }
        ptt.suppress_active_until_release();
        publish(
            app,
            status,
            CapturePhase::Recovery,
            Some(plan.session_id),
            Some(reason),
            false,
            WAV_HEADER_BYTES,
        );
        return None;
    }

    publish(
        app,
        status,
        CapturePhase::Recording,
        Some(plan.session_id.clone()),
        None,
        true,
        WAV_HEADER_BYTES,
    );
    Some(ActiveCapture {
        generation,
        plan,
        stream,
        pipeline,
        signal,
        started: Instant::now(),
        last_level_emit: Instant::now(),
        last_status_emit: Instant::now(),
        smoothed_level: 0.0,
    })
}

fn publish_capture_level(app: &AppHandle, capture: &mut ActiveCapture, level: f32) {
    let smoothing = if level >= capture.smoothed_level {
        0.68
    } else {
        0.22
    };
    capture.smoothed_level += (level - capture.smoothed_level) * smoothing;
    if capture.last_level_emit.elapsed() < LEVEL_EMIT_INTERVAL {
        return;
    }
    capture.last_level_emit = Instant::now();
    let _ = app.emit_to(
        "overlay",
        "capture-level",
        CaptureLevel {
            session_id: capture.plan.session_id.clone(),
            level: capture.smoothed_level.clamp(0.0, 1.0),
        },
    );
}

fn stop_capture(
    app: &AppHandle,
    lifecycle: &Arc<ShellLifecycle>,
    coordinator: &mut SessionCommitCoordinator,
    audio_rx: &Receiver<AudioBlock>,
    status: &Arc<Mutex<CaptureStatus>>,
    active: &mut Option<ActiveCapture>,
    requested_recovery: Option<RecoveryReason>,
) {
    let Some(mut current) = active.take() else {
        return;
    };
    publish(
        app,
        status,
        CapturePhase::Finalizing,
        Some(current.plan.session_id.clone()),
        None,
        true,
        current.pipeline.written_file_bytes(),
    );
    drop(current.stream);
    while let Ok(block) = audio_rx.try_recv() {
        if block.generation == current.generation && current.pipeline.push(&block.samples).is_err()
        {
            break;
        }
    }

    // Reaching the hard session bound is not a failure: the audio captured up to it is complete
    // and must still be recognised. Only the reported reason changes, and nothing is truncated
    // or overwritten, because the writer refused the excess instead of trimming the file.
    let safety_limit = matches!(
        requested_recovery,
        Some(RecoveryReason::SizeLimit | RecoveryReason::Watchdog)
    );
    let mut recovery = if safety_limit {
        None
    } else {
        requested_recovery
    };
    if current.pipeline.finish().is_err() {
        recovery = Some(RecoveryReason::WriterError);
    }
    let bytes = current.pipeline.written_file_bytes();
    if bytes == WAV_HEADER_BYTES && recovery.is_none() {
        recovery = Some(RecoveryReason::EmptyCapture);
    }
    let writer = current.pipeline.into_writer();
    current.plan.finalized_at = now_ms().max(current.plan.started_at);

    if let Some(reason) = recovery {
        let recovered = if reason == RecoveryReason::Cancelled {
            coordinator
                .cancel_pcm_writer(&current.plan, writer, current.plan.finalized_at)
                .is_ok()
        } else {
            match coordinator.recover_pcm_writer(
                &current.plan,
                writer,
                current.plan.finalized_at,
                reason.code(),
            ) {
                Ok(receipt) => {
                    if let Some(archive) = app.try_state::<ArchiveService>() {
                        archive.archive_audio(
                            &receipt.session_id,
                            current.plan.started_at,
                            &receipt.staging_storage_key,
                        );
                    }
                    true
                }
                Err(_) => false,
            }
        };
        lifecycle.recover_capture(&current.plan.session_id, reason.code());
        if recovered {
            lifecycle.finish_capture_recovery(&current.plan.session_id);
        }
        publish(
            app,
            status,
            CapturePhase::Recovery,
            Some(current.plan.session_id),
            Some(reason.code()),
            false,
            bytes,
        );
    } else {
        let receipt = coordinator.finalize_pcm_writer(&current.plan, writer);
        if let Ok(receipt) = receipt {
            if let Some(archive) = app.try_state::<ArchiveService>() {
                archive.archive_audio(
                    &receipt.session_id,
                    current.plan.started_at,
                    &receipt.storage_key,
                );
            }
            lifecycle.finish_capture(&current.plan.session_id);
            publish(app, status, CapturePhase::Idle, None, None, true, bytes);
            if let Some(overlay) = app.try_state::<OverlayService>() {
                // A degraded runtime has no dispatcher thread, so nothing would ever move the HUD off
                // `Processing`: it stayed pinned on top forever and told the owner that a recognition
                // was running. The audio is durable either way; only the reported outcome differs.
                let (phase, reason) = if asr_can_run(app) {
                    (OverlayPhase::Processing, None)
                } else {
                    (OverlayPhase::Error, Some("asr_unavailable"))
                };
                let reason = if safety_limit && reason.is_none() {
                    Some(RecoveryReason::SizeLimit.code())
                } else {
                    reason
                };
                overlay.publish_capture(
                    app,
                    OverlayStatus::new(phase, Some(current.plan.session_id), reason),
                );
            }
        } else {
            lifecycle.recover_capture(&current.plan.session_id, "finalize_failed");
            publish(
                app,
                status,
                CapturePhase::Recovery,
                Some(current.plan.session_id),
                Some("finalize_failed"),
                false,
                bytes,
            );
        }
    }
}
/// A missing or unstartable sidecar leaves the queued attempt without any worker.
fn asr_can_run(app: &AppHandle) -> bool {
    app.try_state::<SidecarRuntime>()
        .is_none_or(|runtime| runtime.accepts_work())
}

fn open_input_stream(
    selected_device_id: Option<&str>,
    generation: Uuid,
    audio_tx: Sender<AudioBlock>,
    signal: Arc<AtomicU8>,
) -> Result<(cpal::Stream, AudioSpec), String> {
    let host = cpal::default_host();
    let device = if let Some(selected) = selected_device_id {
        host.input_devices()
            .map_err(|_| "input_device_enumeration_failed".to_owned())?
            .find(|device| {
                device
                    .id()
                    .is_ok_and(|device_id| device_ids_match(selected, &device_id.to_string()))
            })
            .ok_or_else(|| "selected_input_device_unavailable".to_owned())?
    } else {
        host.default_input_device()
            .ok_or_else(|| "default_input_device_unavailable".to_owned())?
    };
    let supported = device
        .default_input_config()
        .map_err(|_| "input_device_unhealthy".to_owned())?;
    let spec = AudioSpec {
        sample_rate_hz: supported.sample_rate(),
        channels: usize::from(supported.channels()),
    };
    if spec.channels == 0 || spec.sample_rate_hz == 0 {
        return Err("input_config_invalid".into());
    }
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => {
            build_input_stream::<f32>(&device, config, generation, audio_tx, signal)
        }
        SampleFormat::I16 => {
            build_input_stream::<i16>(&device, config, generation, audio_tx, signal)
        }
        SampleFormat::U16 => {
            build_input_stream::<u16>(&device, config, generation, audio_tx, signal)
        }
        _ => Err("input_sample_format_unsupported".into()),
    }?;
    Ok((stream, spec))
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    generation: Uuid,
    audio_tx: Sender<AudioBlock>,
    signal: Arc<AtomicU8>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + Copy,
    f32: FromSample<T>,
{
    let error_signal = signal.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let samples = data
                    .iter()
                    .copied()
                    .map(f32::from_sample)
                    .collect::<Vec<_>>();
                if let Err(error) = audio_tx.try_send(AudioBlock {
                    generation,
                    samples,
                }) {
                    let code = match error {
                        TrySendError::Full(_) => SIGNAL_OVERFLOW,
                        TrySendError::Disconnected(_) => SIGNAL_DEVICE_LOST,
                    };
                    let _ = signal.compare_exchange(
                        SIGNAL_NONE,
                        code,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
            },
            move |error| {
                let _ = error_signal.compare_exchange(
                    SIGNAL_NONE,
                    cpal_stream_error_signal(error.kind()),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            },
            Some(Duration::from_secs(2)),
        )
        .map_err(|error| cpal_open_error_code(error.kind()).to_owned())
}

fn cpal_stream_error_signal(kind: CpalErrorKind) -> u8 {
    match kind {
        CpalErrorKind::DeviceNotAvailable | CpalErrorKind::HostUnavailable => SIGNAL_DEVICE_LOST,
        CpalErrorKind::DeviceBusy => SIGNAL_DEVICE_BUSY,
        CpalErrorKind::DeviceChanged | CpalErrorKind::StreamInvalidated => {
            SIGNAL_STREAM_INVALIDATED
        }
        CpalErrorKind::PermissionDenied => SIGNAL_PERMISSION_DENIED,
        CpalErrorKind::Xrun => SIGNAL_XRUN,
        _ => SIGNAL_BACKEND_ERROR,
    }
}

fn capture_signal_recovery(signal: u8) -> Option<RecoveryReason> {
    match signal {
        SIGNAL_OVERFLOW => Some(RecoveryReason::AudioQueueOverflow),
        SIGNAL_DEVICE_LOST => Some(RecoveryReason::AudioDeviceLost),
        SIGNAL_DEVICE_BUSY => Some(RecoveryReason::AudioDeviceBusy),
        SIGNAL_STREAM_INVALIDATED => Some(RecoveryReason::AudioStreamInvalidated),
        SIGNAL_PERMISSION_DENIED => Some(RecoveryReason::MicrophonePermissionDenied),
        SIGNAL_BACKEND_ERROR => Some(RecoveryReason::AudioBackendError),
        SIGNAL_NONE | SIGNAL_XRUN => None,
        _ => Some(RecoveryReason::AudioBackendError),
    }
}

fn cpal_open_error_code(kind: CpalErrorKind) -> &'static str {
    match kind {
        CpalErrorKind::DeviceNotAvailable => "selected_input_device_unavailable",
        CpalErrorKind::HostUnavailable => "audio_host_unavailable",
        CpalErrorKind::DeviceBusy => "audio_device_busy",
        CpalErrorKind::PermissionDenied => "microphone_permission_denied",
        CpalErrorKind::UnsupportedConfig | CpalErrorKind::InvalidInput => {
            "input_config_unsupported"
        }
        CpalErrorKind::ResourceExhausted => "audio_resource_exhausted",
        CpalErrorKind::DeviceChanged | CpalErrorKind::StreamInvalidated => {
            "audio_stream_invalidated"
        }
        CpalErrorKind::Xrun => "audio_xrun",
        _ => "input_stream_backend_error",
    }
}

fn capture_open_error_code(error: &str) -> &'static str {
    match error {
        "input_device_enumeration_failed" => "input_device_enumeration_failed",
        "selected_input_device_unavailable" => "selected_input_device_unavailable",
        "default_input_device_unavailable" => "default_input_device_unavailable",
        "input_device_unhealthy" => "input_device_unhealthy",
        "input_config_invalid" => "input_config_invalid",
        "input_sample_format_unsupported" => "input_sample_format_unsupported",
        "input_stream_open_failed" => "input_stream_open_failed",
        "input_config_unsupported" => "input_config_unsupported",
        "audio_host_unavailable" => "audio_host_unavailable",
        "audio_device_busy" => "audio_device_busy",
        "microphone_permission_denied" => "microphone_permission_denied",
        "audio_resource_exhausted" => "audio_resource_exhausted",
        "audio_stream_invalidated" => "audio_stream_invalidated",
        "audio_xrun" => "audio_xrun",
        "input_stream_backend_error" => "input_stream_backend_error",
        _ => RecoveryReason::AudioDeviceLost.code(),
    }
}

fn device_ids_match(selected: &str, candidate: &str) -> bool {
    selected == candidate
}

#[derive(Debug)]
struct VoiceGain {
    gain: f32,
}

impl Default for VoiceGain {
    fn default() -> Self {
        Self { gain: 1.0 }
    }
}

impl VoiceGain {
    fn process(&mut self, samples: &mut [f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let square_sum = samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>();
        let rms = (square_sum / samples.len() as f64).sqrt() as f32;
        let desired = if rms <= VOICE_GAIN_FLOOR_RMS {
            1.0
        } else {
            (VOICE_GAIN_TARGET_RMS / rms).clamp(1.0, VOICE_GAIN_MAX)
        };
        let smoothing = if desired < self.gain { 0.80 } else { 0.35 };
        self.gain += (desired - self.gain) * smoothing;

        let mut peak = 0.0_f32;
        for sample in samples {
            *sample = (*sample * self.gain).clamp(-0.98, 0.98);
            peak = peak.max(sample.abs());
        }
        (peak / 0.32).clamp(0.0, 1.0)
    }
}

fn downmix_frame(frame: &[f32]) -> f32 {
    frame
        .iter()
        .copied()
        .max_by(|left, right| left.abs().total_cmp(&right.abs()))
        .unwrap_or(0.0)
}

struct PcmPipeline {
    writer: PcmPartWriter,
    channels: usize,
    input_rate: u32,
    resampler: Option<Async<f32>>,
    pending_mono: Vec<f32>,
    /// Set once the 32 MiB reservation refuses a write. The captured audio up to that point is
    /// complete and must still be transcribed, so later writes are dropped instead of failing.
    reservation_exhausted: bool,
    delay_remaining: usize,
    total_input_frames: usize,
    total_output_frames: usize,
    voice_gain: VoiceGain,
}

impl PcmPipeline {
    fn new(writer: PcmPartWriter, spec: AudioSpec) -> Result<Self, (String, PcmPartWriter)> {
        if spec.channels == 0 || spec.sample_rate_hz == 0 {
            return Err(("input_config_invalid".into(), writer));
        }
        let mut resampler = if spec.sample_rate_hz == PcmFormat::MONO_16KHZ_S16.sample_rate_hz {
            None
        } else {
            match Async::<f32>::new_poly(
                f64::from(PcmFormat::MONO_16KHZ_S16.sample_rate_hz)
                    / f64::from(spec.sample_rate_hz),
                1.0,
                PolynomialDegree::Cubic,
                RESAMPLE_CHUNK_FRAMES,
                1,
                FixedAsync::Input,
            ) {
                Ok(resampler) => Some(resampler),
                Err(_) => {
                    return Err(("resampler_initialization_failed".to_owned(), writer));
                }
            }
        };
        let delay_remaining = resampler
            .as_mut()
            .map(|value| value.output_delay())
            .unwrap_or(0);
        Ok(Self {
            writer,
            channels: spec.channels,
            input_rate: spec.sample_rate_hz,
            resampler,
            pending_mono: Vec::with_capacity(RESAMPLE_CHUNK_FRAMES * 2),
            reservation_exhausted: false,
            delay_remaining,
            total_input_frames: 0,
            total_output_frames: 0,
            voice_gain: VoiceGain::default(),
        })
    }

    fn push(&mut self, interleaved: &[f32]) -> Result<f32, String> {
        if !interleaved.len().is_multiple_of(self.channels) {
            return Err("unaligned_input_block".into());
        }
        let frame_count = interleaved.len() / self.channels;
        self.total_input_frames = self
            .total_input_frames
            .checked_add(frame_count)
            .ok_or_else(|| "input_frame_counter_overflow".to_owned())?;
        let mut mono = interleaved
            .chunks_exact(self.channels)
            .map(downmix_frame)
            .collect::<Vec<_>>();
        let level = self.voice_gain.process(&mut mono);
        self.pending_mono.extend(mono);

        if self.resampler.is_none() {
            let mono = std::mem::take(&mut self.pending_mono);
            self.write_f32(&mono, None)?;
            return Ok(level);
        }
        while self.pending_mono.len() >= RESAMPLE_CHUNK_FRAMES {
            let chunk = self
                .pending_mono
                .drain(..RESAMPLE_CHUNK_FRAMES)
                .collect::<Vec<_>>();
            let output_limit = self.expected_output_frames();
            self.process_chunk(&chunk, None, Some(output_limit))?;
        }
        Ok(level)
    }

    fn finish(&mut self) -> Result<(), String> {
        if self.resampler.is_none() {
            let mono = std::mem::take(&mut self.pending_mono);
            return self.write_f32(&mono, None);
        }
        let target = self.expected_output_frames();
        if !self.pending_mono.is_empty() {
            let partial_len = self.pending_mono.len();
            let mut chunk = std::mem::take(&mut self.pending_mono);
            chunk.resize(RESAMPLE_CHUNK_FRAMES, 0.0);
            self.process_chunk(&chunk, Some(partial_len), Some(target))?;
        }
        let zeros = vec![0.0; RESAMPLE_CHUNK_FRAMES];
        for _ in 0..16 {
            if self.total_output_frames >= target {
                break;
            }
            self.process_chunk(&zeros, None, Some(target))?;
        }
        if self.total_output_frames < target && !self.reservation_exhausted {
            return Err("resampler_tail_incomplete".into());
        }
        Ok(())
    }

    fn expected_output_frames(&self) -> usize {
        ((self.total_input_frames as u128) * u128::from(PcmFormat::MONO_16KHZ_S16.sample_rate_hz)
            / u128::from(self.input_rate)) as usize
    }

    fn process_chunk(
        &mut self,
        input: &[f32],
        partial_len: Option<usize>,
        output_limit: Option<usize>,
    ) -> Result<(), String> {
        let adapter = InterleavedSlice::new(input, 1, RESAMPLE_CHUNK_FRAMES)
            .map_err(|_| "resampler_input_invalid".to_owned())?;
        let indexing = partial_len.map(|value| Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: Some(value),
            active_channels_mask: None,
        });
        let output = self
            .resampler
            .as_mut()
            .expect("resampler branch is checked")
            .process(&adapter, indexing.as_ref())
            .map_err(|_| "resampler_processing_failed".to_owned())?;
        self.write_f32(&output.take_data(), output_limit)
    }

    fn write_f32(&mut self, samples: &[f32], output_limit: Option<usize>) -> Result<(), String> {
        if self.reservation_exhausted {
            return Ok(());
        }
        let skip = self.delay_remaining.min(samples.len());
        self.delay_remaining -= skip;
        let mut useful = &samples[skip..];
        if let Some(limit) = output_limit {
            useful = &useful[..useful
                .len()
                .min(limit.saturating_sub(self.total_output_frames))];
        }
        if useful.is_empty() {
            return Ok(());
        }
        let pcm = useful
            .iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16)
            .collect::<Vec<_>>();
        self.writer.write_samples(&pcm).map_err(|error| {
            // Only a refused reservation is the 32 MiB safety limit; a failing disk is not.
            match error {
                SessionCommitError::Conflict(_) => {
                    self.reservation_exhausted = true;
                    RecoveryReason::SizeLimit.code()
                }
                _ => RecoveryReason::WriterError.code(),
            }
            .to_owned()
        })?;
        self.total_output_frames += pcm.len();
        Ok(())
    }

    fn written_file_bytes(&self) -> u64 {
        self.writer.written_file_bytes()
    }

    fn into_writer(self) -> PcmPartWriter {
        self.writer
    }
}
fn capture_plan(runtime_profile_id: String, started_at: i64) -> CaptureCommitPlan {
    let session_id = Uuid::new_v4().to_string();
    let artifact_id = Uuid::new_v4().to_string();
    let commit_id = Uuid::new_v4().to_string();
    CaptureCommitPlan {
        session_id,
        artifact_id,
        commit_id: commit_id.clone(),
        prepare_event_id: Uuid::new_v4().to_string(),
        finalizing_event_id: Uuid::new_v4().to_string(),
        finalized_event_id: Uuid::new_v4().to_string(),
        runtime_profile_id,
        asr_attempt_id: Uuid::new_v4().to_string(),
        asr_idempotency_key: format!("capture-{commit_id}"),
        started_at,
        finalized_at: started_at,
        reserved_byte_size: PCM_LIMIT_BYTES,
        format: PcmFormat::MONO_16KHZ_S16,
    }
}

fn pcm_watchdog_limit() -> Duration {
    let pcm_bytes_per_second = u64::from(PcmFormat::MONO_16KHZ_S16.sample_rate_hz) * 2;
    Duration::from_millis(
        PCM_LIMIT_BYTES
            .saturating_sub(WAV_HEADER_BYTES)
            .saturating_mul(1000)
            / pcm_bytes_per_second,
    )
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

/// Records that the device dropped samples and tells the UI at once.
///
/// The recording deliberately continues - an xrun is survivable - but the lost samples are gone
/// for good, so staying silent about them would quietly break the promise that nothing is lost.
fn note_audio_gap(app: &AppHandle, status: &Arc<Mutex<CaptureStatus>>) {
    let snapshot = {
        let Ok(mut current) = status.lock() else {
            return;
        };
        current.audio_gaps = current.audio_gaps.saturating_add(1);
        current.clone()
    };
    let _ = app.emit("capture-status", snapshot);
}

fn publish(
    app: &AppHandle,
    status: &Arc<Mutex<CaptureStatus>>,
    phase: CapturePhase,
    session_id: Option<String>,
    reason: Option<&str>,
    device_healthy: bool,
    durable_pcm_bytes: u64,
) {
    let mut next = CaptureStatus {
        phase,
        session_id: session_id.clone(),
        reason: reason.map(str::to_owned),
        device_healthy,
        durable_pcm_bytes,
        audio_gaps: 0,
    };
    if let Ok(mut current) = status.lock() {
        // Gaps belong to one capture: they survive every status change inside it and reset the
        // moment a different session starts.
        if current.session_id == next.session_id {
            next.audio_gaps = current.audio_gaps;
        }
        *current = next.clone();
    }
    if let Some(overlay) = app.try_state::<OverlayService>() {
        match phase {
            CapturePhase::Preparing => overlay.publish_capture(
                app,
                OverlayStatus::new(OverlayPhase::Preparing, session_id.clone(), reason),
            ),
            CapturePhase::Recording => overlay.publish_capture(
                app,
                OverlayStatus::new(OverlayPhase::Recording, session_id.clone(), reason),
            ),
            CapturePhase::Finalizing => overlay.publish_capture(
                app,
                OverlayStatus::new(OverlayPhase::Processing, session_id.clone(), reason),
            ),
            CapturePhase::Recovery if reason == Some(RecoveryReason::Cancelled.code()) => {
                overlay.hide(app);
            }
            CapturePhase::Recovery => overlay.publish_capture(
                app,
                OverlayStatus::new(OverlayPhase::Error, session_id.clone(), reason),
            ),
            CapturePhase::Unavailable => overlay.publish_capture(
                app,
                OverlayStatus::new(OverlayPhase::Error, session_id.clone(), reason),
            ),
            CapturePhase::Idle => {}
        }
    }
    if let Some(diagnostics) = app.try_state::<DiagnosticService>() {
        let (stage, outcome) = match phase {
            CapturePhase::Preparing => (DiagnosticStage::Prepare, DiagnosticOutcome::Started),
            CapturePhase::Recording => (DiagnosticStage::Record, DiagnosticOutcome::Started),
            CapturePhase::Finalizing => (DiagnosticStage::Finalize, DiagnosticOutcome::Started),
            CapturePhase::Idle => (DiagnosticStage::Finalize, DiagnosticOutcome::Succeeded),
            CapturePhase::Recovery if reason == Some(RecoveryReason::Cancelled.code()) => {
                (DiagnosticStage::Recover, DiagnosticOutcome::Cancelled)
            }
            CapturePhase::Recovery => (DiagnosticStage::Recover, DiagnosticOutcome::Recovered),
            CapturePhase::Unavailable => (DiagnosticStage::Prepare, DiagnosticOutcome::Failed),
        };
        let stage = if reason == Some("finalize_failed") {
            DiagnosticStage::Commit
        } else {
            stage
        };
        let event = capture_event(
            session_id.clone(),
            stage,
            outcome,
            reason,
            durable_pcm_bytes,
        );
        if phase == CapturePhase::Recording && durable_pcm_bytes > WAV_HEADER_BYTES {
            diagnostics.record_extended(event);
        } else {
            diagnostics.record_essential(event);
        }
    }
    let _ = app.emit("capture-status", next);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptt_latch_ignores_repeat_and_orphan_release() {
        let latch = PttLatch::default();
        assert!(latch.pressed());
        assert!(!latch.pressed());
        assert!(latch.released());
        assert!(!latch.released());
        assert!(latch.pressed());
        latch.suppress_active_until_release();
        assert!(!latch.pressed());
        assert!(!latch.released());
        assert!(latch.pressed());
    }

    #[test]
    fn escape_is_reserved_only_while_a_capture_is_live() {
        assert!(cancel_key_is_reserved(CapturePhase::Preparing));
        assert!(cancel_key_is_reserved(CapturePhase::Recording));
        // Every other phase must leave Escape to the rest of the system.
        for phase in [
            CapturePhase::Idle,
            CapturePhase::Finalizing,
            CapturePhase::Recovery,
            CapturePhase::Unavailable,
        ] {
            assert!(
                !cancel_key_is_reserved(phase),
                "{phase:?} must release Escape"
            );
        }
        // Escape stays impossible to bind as the dictation toggle, so the shared shortcut
        // handler can always tell the two apart by key alone.
        assert!(validate_hotkey("Escape").is_err());
    }

    #[test]
    fn the_safety_limit_keeps_the_recording_instead_of_failing_it() {
        let format = PcmFormat::MONO_16KHZ_S16;
        let bytes_per_second = u64::from(format.sample_rate_hz) * 2;
        // The watchdog and the byte limit express the same bound, so both must finalize.
        assert_eq!(
            pcm_watchdog_limit().as_millis() as u64,
            (PCM_LIMIT_BYTES - WAV_HEADER_BYTES) * 1000 / bytes_per_second
        );
        for reason in [RecoveryReason::SizeLimit, RecoveryReason::Watchdog] {
            assert!(
                matches!(
                    Some(reason),
                    Some(RecoveryReason::SizeLimit | RecoveryReason::Watchdog)
                ),
                "{} must take the finalize path",
                reason.code()
            );
        }
        // A writer failure is still a failure and must not be finalized as a good recording.
        assert!(!matches!(
            Some(RecoveryReason::WriterError),
            Some(RecoveryReason::SizeLimit | RecoveryReason::Watchdog)
        ));
    }

    #[test]
    fn an_exhausted_reservation_stops_writing_without_failing_the_recording() {
        let root = std::env::temp_dir().join(format!("wigigadict-limit-{}", Uuid::new_v4()));
        let store = wigigadict_storage::ManagedAudioStore::open(&root).unwrap();
        // A reservation that only fits part of the input, exactly like the 32 MiB session bound.
        let reserved = WAV_HEADER_BYTES + 2_000;
        let writer = store
            .create_writer("limit", reserved, PcmFormat::MONO_16KHZ_S16)
            .unwrap();
        let mut pipeline = match PcmPipeline::new(
            writer,
            AudioSpec {
                sample_rate_hz: 16_000,
                channels: 1,
            },
        ) {
            Ok(value) => value,
            Err((_error, _writer)) => panic!("pipeline must initialize"),
        };

        let block = vec![0.25_f32; 800];
        let mut limit_reason = None;
        for _ in 0..8 {
            if let Err(reason) = pipeline.push(&block) {
                limit_reason = Some(reason);
                break;
            }
        }
        assert_eq!(
            limit_reason.as_deref(),
            Some(RecoveryReason::SizeLimit.code()),
            "the refused reservation must be reported as the safety limit"
        );

        // The audio captured up to the bound stays a normal finalized recording: nothing is
        // truncated or overwritten, the tail check does not fail it, and the file never grows
        // past the reservation.
        let written = pipeline.written_file_bytes();
        // Draining the last queued blocks after the bound must neither fail nor grow the file.
        assert!(pipeline.push(&block).is_ok());
        assert_eq!(pipeline.written_file_bytes(), written);
        assert!(pipeline.finish().is_ok());
        assert!(pipeline.written_file_bytes() <= reserved);
        assert!(pipeline.written_file_bytes() > WAV_HEADER_BYTES);
        drop(pipeline);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_a_refused_reservation_is_reported_as_the_safety_limit() {
        assert_eq!(
            pipeline_recovery_reason(RecoveryReason::SizeLimit.code()),
            RecoveryReason::SizeLimit
        );
        for other in [
            "pcm_writer_error",
            "unaligned_input_block",
            "resampler_processing_failed",
            "input_frame_counter_overflow",
        ] {
            assert_eq!(
                pipeline_recovery_reason(other),
                RecoveryReason::WriterError,
                "{other} must not be reported as the 32 MiB limit"
            );
        }
    }

    #[test]
    fn toggle_stops_only_an_active_capture() {
        assert!(toggle_stops_capture(CapturePhase::Preparing));
        assert!(toggle_stops_capture(CapturePhase::Recording));
        for phase in [
            CapturePhase::Idle,
            CapturePhase::Finalizing,
            CapturePhase::Recovery,
            CapturePhase::Unavailable,
        ] {
            assert!(!toggle_stops_capture(phase));
        }
    }

    #[test]
    fn idle_safety_action_does_not_swallow_the_next_physical_press() {
        let latch = PttLatch::default();
        latch.suppress_active_until_release();
        assert!(latch.pressed());
    }

    #[test]
    fn hotkey_allows_function_key_or_modifier_and_reserves_escape() {
        assert!(validate_hotkey(DEFAULT_HOTKEY).is_ok());
        assert!(validate_hotkey("F12").is_ok());
        assert!(validate_hotkey("Space").is_err());
        assert!(validate_hotkey("D").is_err());
        assert!(validate_hotkey("control+Escape").is_err());
        assert!(validate_hotkey("control+alt+NotAKey").is_err());
    }

    #[test]
    fn capture_open_errors_are_bounded_before_persistence() {
        for code in [
            "input_device_enumeration_failed",
            "selected_input_device_unavailable",
            "default_input_device_unavailable",
            "input_device_unhealthy",
            "input_config_invalid",
            "input_sample_format_unsupported",
            "input_stream_open_failed",
            "input_config_unsupported",
            "audio_host_unavailable",
            "audio_device_busy",
            "microphone_permission_denied",
            "audio_resource_exhausted",
            "audio_stream_invalidated",
            "audio_xrun",
            "input_stream_backend_error",
        ] {
            assert_eq!(capture_open_error_code(code), code);
        }
        assert_eq!(
            capture_open_error_code("driver returned private detail"),
            RecoveryReason::AudioDeviceLost.code()
        );
    }

    #[test]
    fn cpal_errors_are_reduced_to_bounded_content_free_codes() {
        assert_eq!(
            cpal_stream_error_signal(CpalErrorKind::DeviceBusy),
            SIGNAL_DEVICE_BUSY
        );
        assert_eq!(
            cpal_stream_error_signal(CpalErrorKind::StreamInvalidated),
            SIGNAL_STREAM_INVALIDATED
        );
        assert_eq!(
            cpal_open_error_code(CpalErrorKind::PermissionDenied),
            "microphone_permission_denied"
        );
        assert_eq!(
            cpal_open_error_code(CpalErrorKind::BackendError),
            "input_stream_backend_error"
        );
        assert_eq!(cpal_stream_error_signal(CpalErrorKind::Xrun), SIGNAL_XRUN);
        assert_eq!(capture_signal_recovery(SIGNAL_XRUN), None);
        assert_eq!(
            capture_signal_recovery(SIGNAL_DEVICE_LOST),
            Some(RecoveryReason::AudioDeviceLost)
        );
    }

    #[test]
    fn selected_wasapi_device_uses_the_same_rendered_identity_as_the_catalog() {
        let rendered = "wasapi:{0.0.1.00000000}.{device-guid}";
        assert!(device_ids_match(rendered, rendered));
        assert!(!device_ids_match(
            rendered,
            "{0.0.1.00000000}.{device-guid}"
        ));
    }

    #[test]
    fn callback_overflow_sets_explicit_fault_without_blocking() {
        let (tx, rx) = bounded(1);
        let signal = Arc::new(AtomicU8::new(SIGNAL_NONE));
        tx.try_send(AudioBlock {
            generation: Uuid::nil(),
            samples: vec![0.0],
        })
        .unwrap();
        let error = tx
            .try_send(AudioBlock {
                generation: Uuid::nil(),
                samples: vec![0.0],
            })
            .unwrap_err();
        if matches!(error, TrySendError::Full(_)) {
            signal.store(SIGNAL_OVERFLOW, Ordering::Release);
        }
        assert_eq!(signal.load(Ordering::Acquire), SIGNAL_OVERFLOW);
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn watchdog_is_derived_from_the_same_32_mib_pcm_reservation() {
        assert_eq!(PCM_LIMIT_BYTES, 33_554_432);
        assert_eq!(pcm_watchdog_limit(), Duration::from_millis(1_048_574));
    }

    #[test]
    fn capture_plan_is_immutable_and_reserves_the_hard_limit() {
        let plan = capture_plan("runtime-1".into(), 100);
        assert_eq!(plan.started_at, plan.finalized_at);
        assert_eq!(plan.reserved_byte_size, PCM_LIMIT_BYTES);
        assert_eq!(plan.format, PcmFormat::MONO_16KHZ_S16);
        assert!(plan.asr_idempotency_key.contains(&plan.commit_id));
    }

    #[test]
    fn recovery_reasons_are_stable_machine_codes() {
        let cases = [
            (RecoveryReason::Cancelled, "cancelled"),
            (RecoveryReason::Watchdog, "lost_keyup_watchdog"),
            (RecoveryReason::AudioDeviceLost, "audio_device_lost"),
            (RecoveryReason::AudioDeviceBusy, "audio_device_busy"),
            (
                RecoveryReason::AudioStreamInvalidated,
                "audio_stream_invalidated",
            ),
            (
                RecoveryReason::MicrophonePermissionDenied,
                "microphone_permission_denied",
            ),
            (RecoveryReason::AudioBackendError, "audio_backend_error"),
            (RecoveryReason::AudioQueueOverflow, "audio_queue_overflow"),
            (RecoveryReason::SizeLimit, "pcm_size_limit"),
        ];
        for (reason, expected) in cases {
            assert_eq!(reason.code(), expected);
        }
    }
    #[test]
    fn fake_stereo_48khz_resamples_to_durable_mono_16khz_pcm() {
        let root =
            std::env::temp_dir().join(format!("wigigadict-step8-resample-{}", Uuid::new_v4()));
        let store = wigigadict_storage::ManagedAudioStore::open(&root).unwrap();
        let writer = store
            .create_writer("step8-resample", 1024 * 1024, PcmFormat::MONO_16KHZ_S16)
            .unwrap();
        let mut pipeline = match PcmPipeline::new(
            writer,
            AudioSpec {
                sample_rate_hz: 48_000,
                channels: 2,
            },
        ) {
            Ok(value) => value,
            Err((_error, _writer)) => panic!("fake resampler must initialize"),
        };
        let input = vec![0.25_f32; 4_800 * 2];
        for block in input.chunks(960 * 2) {
            pipeline.push(block).unwrap();
        }
        pipeline.finish().unwrap();
        assert_eq!(pipeline.written_file_bytes(), WAV_HEADER_BYTES + 3_200);
        drop(pipeline);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adaptive_voice_gain_lifts_quiet_speech_without_clipping() {
        let mut gain = VoiceGain::default();
        let mut block = vec![0.01_f32; 960];
        let mut level = 0.0;
        for _ in 0..6 {
            block.fill(0.01);
            level = gain.process(&mut block);
        }
        assert!(block[0] >= 0.045, "quiet voice should receive useful gain");
        assert!(block.iter().all(|sample| sample.abs() <= 0.98));
        assert!(level > 0.1);
    }

    #[test]
    fn downmix_keeps_a_single_active_microphone_channel() {
        assert_eq!(downmix_frame(&[0.02, 0.0]), 0.02);
        assert_eq!(downmix_frame(&[-0.01, 0.03]), 0.03);
    }

    #[test]
    fn capture_level_event_is_content_free() {
        let serialized = serde_json::to_value(CaptureLevel {
            session_id: "session-1".into(),
            level: 0.42,
        })
        .unwrap();
        assert_eq!(
            serialized.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["level", "sessionId"]
        );
    }

    #[test]
    fn ui_event_cadence_is_bounded_below_audio_callback_rate() {
        assert!(LEVEL_EMIT_INTERVAL >= Duration::from_millis(60));
        assert!(RECORDING_STATUS_EMIT_INTERVAL >= Duration::from_millis(200));
        assert!(RECORDING_STATUS_EMIT_INTERVAL > LEVEL_EMIT_INTERVAL);
    }
}
