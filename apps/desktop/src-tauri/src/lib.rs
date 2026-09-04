// MSVC reports creation of the required cdylib import library on linker stdout.
// Rust 1.97 surfaces that informational line as a warning.
#![allow(linker_messages)]

#[cfg(windows)]
mod archive;
mod asr_service;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod diagnostics;
#[cfg(all(test, windows))]
mod golden_flow_tests;
#[cfg(windows)]
mod insertion;
mod ipc;
#[cfg(all(windows, debug_assertions))]
mod m0_win32_spike;
#[cfg(windows)]
mod models;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod recovery;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod shell_lifecycle;
mod version;
#[cfg(windows)]
mod windows_insertion;

use asr_service::SidecarRuntime;
#[cfg(windows)]
use capture::{CaptureService, CaptureStatus, DEFAULT_HOTKEY, RecoveryReason};
use serde::Serialize;
#[cfg(windows)]
use shell_lifecycle::{
    ShellBootstrap, ShellEvent, ShellLifecycle, ShellLifecycleError, ShellStatus,
};
use tauri::Manager;

#[cfg(windows)]
use tauri_plugin_global_shortcut::Shortcut;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    state: &'static str,
    protocol: String,
    sidecar: String,
    detail: String,
}

#[cfg(windows)]
struct HotkeyBinding(std::sync::Mutex<Shortcut>);

#[cfg(windows)]
#[tauri::command]
fn runtime_status(
    window: tauri::WebviewWindow,
    runtime: tauri::State<'_, SidecarRuntime>,
) -> Result<RuntimeStatus, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    Ok(runtime.status())
}

#[cfg(windows)]
#[tauri::command]
fn shell_status(
    window: tauri::WebviewWindow,
    lifecycle: tauri::State<'_, std::sync::Arc<ShellLifecycle>>,
) -> Result<ShellStatus, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    lifecycle.status().map_err(|error| error.to_string())
}

#[cfg(windows)]
#[tauri::command]
fn capture_status(
    window: tauri::WebviewWindow,
    capture: tauri::State<'_, std::sync::Arc<CaptureService>>,
) -> Result<CaptureStatus, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    capture.status()
}

#[cfg(windows)]
#[tauri::command]
fn cancel_capture(
    window: tauri::WebviewWindow,
    capture: tauri::State<'_, std::sync::Arc<CaptureService>>,
) -> Result<(), String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    capture.cancel()
}

#[cfg(windows)]
#[tauri::command]
fn overlay_cancel_capture(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    capture: tauri::State<'_, std::sync::Arc<CaptureService>>,
) -> Result<(), String> {
    shell_lifecycle::authorize_overlay_window(&window).map_err(|error| error.to_string())?;
    if let Some(overlay) = app.try_state::<overlay::OverlayService>() {
        overlay.hide(&app);
    }
    capture.cancel()
}

#[cfg(windows)]
#[tauri::command]
fn overlay_finish_capture(
    window: tauri::WebviewWindow,
    capture: tauri::State<'_, std::sync::Arc<CaptureService>>,
) -> Result<(), String> {
    shell_lifecycle::authorize_overlay_window(&window).map_err(|error| error.to_string())?;
    capture.finish()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(windows)]
    run_windows();

    #[cfg(not(windows))]
    compile_error!("WiGigaDict personal MVP supports only Windows");
}

/// The M0 Win32 spike harness is a development tool: it replaces the whole shell with a bare
/// overlay and no tray, so a shipped binary must not expose it.
#[cfg(windows)]
fn m0_overlay_spike_requested() -> bool {
    #[cfg(debug_assertions)]
    {
        m0_win32_spike::requested()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[cfg(windows)]
fn dev_overlay_cycle_requested() -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::args().any(|argument| argument == "--dev-overlay-cycle")
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Appends one content-free line to `%LOCALAPPDATA%\WiGigaDict\logs\startup.log`.
///
/// A release build is a `windows` subsystem binary, so `eprintln!` goes nowhere. Every startup
/// path that ends the process now leaves a readable trace instead of vanishing.
#[cfg(windows)]
fn log_startup(message: &str) {
    eprintln!("WiGigaDict startup: {message}");
    let Ok(local) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let directory = std::path::Path::new(&local).join("WiGigaDict").join("logs");
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("startup.log"))
    {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or_default();
        let _ = writeln!(file, "{stamp} {message}");
    }
}

#[cfg(windows)]
fn run_windows() {
    let ShellBootstrap {
        lifecycle,
        instance_guard,
    } = match shell_lifecycle::bootstrap() {
        Ok(bootstrap) => bootstrap,
        Err(ShellLifecycleError::AlreadyRunning) => {
            // Never exit silently on a second launch: raise the window of the live instance.
            log_startup("second_instance_activating_existing_window");
            shell_lifecycle::request_existing_instance_activation();
            return;
        }
        Err(error) => {
            log_startup(&format!("bootstrap_rejected: {error}"));
            return;
        }
    };

    let overlay_spike = m0_overlay_spike_requested();
    let dev_overlay_cycle = dev_overlay_cycle_requested();
    let started_with_windows = std::env::args().any(|argument| argument == "--startup");
    let (configured_hotkey, configured_startup, configured_diagnostics) =
        if overlay_spike || dev_overlay_cycle {
            (DEFAULT_HOTKEY.to_owned(), false, false)
        } else {
            let paths = lifecycle.capture_paths();
            let configuration = match settings::initialize_configuration(&paths.database) {
                Ok(configuration) => configuration,
                Err(error) => {
                    log_startup(&format!("configuration_rejected: {error}"));
                    return;
                }
            };
            (
                configuration.hotkey_binding,
                configuration.startup_enabled,
                configuration.diagnostic_mode,
            )
        };
    let configured_shortcut = match capture::validate_hotkey(&configured_hotkey) {
        Ok(shortcut) => shortcut,
        Err(error) => {
            log_startup(&format!("hotkey_rejected: {error}"));
            return;
        }
    };
    let setup_lifecycle = lifecycle.clone();
    let window_lifecycle = lifecycle.clone();
    let run_lifecycle = lifecycle.clone();
    let capture_slot =
        std::sync::Arc::new(std::sync::OnceLock::<std::sync::Arc<CaptureService>>::new());
    let handler_slot = capture_slot.clone();
    let run_capture_slot = capture_slot.clone();

    let shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcut(configured_hotkey.as_str())
        .expect("validated dictation toggle hotkey must parse")
        .with_handler(move |_app, shortcut, event| {
            let Some(capture) = handler_slot.get() else {
                return;
            };
            // One handler serves every registered shortcut, so it has to tell them apart.
            // `Escape` is reserved for cancellation and is never a valid dictation binding.
            if shortcut.key == tauri_plugin_global_shortcut::Code::Escape {
                if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                    let _ = capture.cancel();
                }
                return;
            }
            capture.on_shortcut_state(event.state);
        })
        .build();

    let builder = tauri::Builder::default()
        .plugin(shortcut_plugin)
        .manage(instance_guard)
        .manage(lifecycle)
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            shell_status,
            capture_status,
            cancel_capture,
            overlay_cancel_capture,
            overlay_finish_capture,
            diagnostics::diagnostic_status,
            diagnostics::diagnostic_prepare,
            diagnostics::diagnostic_export,
            settings::settings_get,
            settings::settings_update,
            settings::archive_directory_pick,
            models::models_list,
            models::models_install_start,
            models::models_install_pause,
            models::models_install_cancel,
            models::models_import_local,
            models::models_activate,
            models::models_remove,
            recovery::recovery_list,
            recovery::recovery_retry,
            recovery::recovery_record_copy,
            recovery::recovery_resolve,
            recovery::recovery_set_pinned,
            recovery::recovery_delete
        ])
        .setup(move |app| {
            #[cfg(debug_assertions)]
            if overlay_spike {
                m0_win32_spike::setup(app)?;
                return Ok(());
            }

            let overlay = overlay::OverlayService::new();
            overlay.configure(app.handle())?;
            app.manage(overlay);
            #[cfg(debug_assertions)]
            if dev_overlay_cycle {
                let main = app
                    .get_webview_window("main")
                    .ok_or("configured main window is missing")?;
                main.hide()?;
                overlay::start_dev_cycle(app.handle().clone())?;
                return Ok(());
            }
            shell_lifecycle::setup_tray(app, setup_lifecycle.clone())?;
            let main = app
                .get_webview_window("main")
                .ok_or("configured main window is missing")?;
            if started_with_windows {
                main.hide()?;
            }

            let paths = setup_lifecycle.capture_paths();
            settings::reconcile_configured_startup(configured_startup)?;
            app.manage(settings::SettingsService::new(
                paths.database.clone(),
                paths.audio_root.clone(),
            ));
            let archive = archive::ArchiveService::new(&paths.database, &paths.audio_root);
            // Missing/corrupt history must not make the shell unavailable. Backfill is
            // idempotent and retries remaining owner-visible copies on every start.
            let _ = archive.backfill_current();
            app.manage(archive);
            app.manage(diagnostics::DiagnosticService::new(
                &paths.audio_root,
                configured_diagnostics,
            )?);
            // Resources sit next to the executable in both the dev build and the installer.
            let catalog_dir = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
                .unwrap_or_else(|| paths.audio_root.clone());
            app.manage(models::ModelService::new(
                &paths.database,
                &paths.audio_root,
                &catalog_dir,
            ));
            let recovery = recovery::RecoveryService::new(&paths.database, &paths.audio_root);
            recovery.startup_maintenance()?;
            app.manage(recovery);
            let capture = CaptureService::start(
                app.handle().clone(),
                setup_lifecycle.clone(),
                paths.database.clone(),
                paths.audio_root.clone(),
            );
            if capture_slot.set(capture.clone()).is_err() {
                return Err("capture service was initialized twice".into());
            }
            app.manage(capture.clone());
            app.manage(HotkeyBinding(std::sync::Mutex::new(configured_shortcut)));

            let safety_capture = capture.clone();
            shell_lifecycle::install_session_notifications(
                &main,
                setup_lifecycle.clone(),
                std::sync::Arc::new(move |event| {
                    if let Some(reason) = recovery_reason(event) {
                        let _ = safety_capture.recover_and_wait(reason);
                    }
                }),
            )?;

            // The single-instance guard and lifecycle boundary are established before any
            // component that owns durable writers or child processes. A missing sidecar degrades
            // the ASR runtime instead of failing setup, which used to kill the whole shell.
            let runtime =
                SidecarRuntime::start(app.handle().clone(), paths.database, paths.audio_root);
            app.manage(runtime);
            Ok(())
        })
        .on_window_event(move |window, event| {
            #[cfg(debug_assertions)]
            if overlay_spike {
                m0_win32_spike::on_window_event(window, event);
                return;
            }
            if window.label() == "main"
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
                && !window_lifecycle.is_shutting_down()
            {
                api.prevent_close();
                let _ = window.hide();
            }
        });

    // Tauri creates every configured window before it runs the setup hook, so a panic here left
    // the main window on screen for a moment and then killed the process without any trace.
    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(error) => {
            log_startup(&format!("setup_failed: {error}"));
            std::process::exit(1);
        }
    };
    app.run(move |_app, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            if let Some(capture) = run_capture_slot.get() {
                let _ = capture.recover_and_wait(RecoveryReason::ApplicationExit);
            }
            let _ = run_lifecycle.on_event(ShellEvent::AppExit);
        }
    });
}

#[cfg(windows)]
fn recovery_reason(event: ShellEvent) -> Option<RecoveryReason> {
    match event {
        ShellEvent::SessionLock => Some(RecoveryReason::WindowsSessionLocked),
        ShellEvent::SessionLogoff => Some(RecoveryReason::WindowsSessionLogoff),
        ShellEvent::QueryEndSession | ShellEvent::EndSession => {
            Some(RecoveryReason::WindowsShutdown)
        }
        ShellEvent::AppExit => Some(RecoveryReason::ApplicationExit),
        ShellEvent::SessionUnlock | ShellEvent::EndSessionCancelled => None,
    }
}
