//! Test-only M0 acceptance path for the real Tauri/WRY overlay window.
//!
//! This module is inert unless the executable receives `--m0-overlay-spike`.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, SW_HIDE, SW_SHOWNOACTIVATE,
    SetWindowLongPtrW, ShowWindow, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

const OVERLAY_LABEL: &str = "m0-overlay-spike";
static ARMED: AtomicBool = AtomicBool::new(false);
static RAN: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
struct OverlayReport {
    schema_version: u32,
    shell_version: &'static str,
    window_kind: &'static str,
    cycles: u32,
    target_mismatches: u32,
    focus_steals: u32,
    required_styles_present: bool,
    passed: bool,
    content_logged: bool,
}

/// Only reachable from a debug build: [`crate::m0_overlay_spike_requested`] gates it out of any
/// shipped binary, because this harness replaces the entire shell.
#[cfg(debug_assertions)]
pub fn requested() -> bool {
    std::env::args().any(|argument| argument == "--m0-overlay-spike")
        || std::env::current_exe()
            .ok()
            .and_then(|path| path.file_stem().map(|stem| stem.to_owned()))
            .is_some_and(|stem| stem == "wigigadict-overlay-spike")
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("index.html".into()))
        .title("WiGigaDict M0 Tauri no-activate overlay")
        .inner_size(360.0, 80.0)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .visible(false)
        .build()?;
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(1));
        ARMED.store(true, Ordering::SeqCst);
        thread::sleep(Duration::from_secs(9));
        if !RAN.load(Ordering::SeqCst) {
            std::process::exit(2);
        }
    });
    Ok(())
}

pub fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != "main"
        || !matches!(event, WindowEvent::Focused(true))
        || !ARMED.load(Ordering::SeqCst)
        || RAN.swap(true, Ordering::SeqCst)
    {
        return;
    }

    let result = run(window);
    let mut exit_code = if result.as_ref().is_ok_and(|report| report.passed) {
        0
    } else {
        1
    };
    if let Ok(report) = result
        && let Some(path) = report_path()
        && write_report(&path, &report).is_err()
    {
        exit_code = 1;
    }
    std::process::exit(exit_code);
}

fn run(main: &tauri::Window) -> Result<OverlayReport, Box<dyn std::error::Error>> {
    let app = main.app_handle();
    let overlay = app
        .get_webview_window(OVERLAY_LABEL)
        .ok_or("Tauri overlay window is missing")?;
    let main_hwnd = main.hwnd()?;
    let overlay_hwnd = overlay.hwnd()?;
    wait_for_stable_foreground(main_hwnd)?;
    let mut target_mismatches = 0_u32;
    let mut focus_steals = 0_u32;

    // SAFETY: the HWND belongs to the live Tauri overlay on the UI thread.
    unsafe {
        let current = GetWindowLongPtrW(overlay_hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            overlay_hwnd,
            GWL_EXSTYLE,
            (current | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize,
        );
    }

    for _ in 0..100 {
        // SAFETY: both HWND values belong to live Tauri windows for the duration of the run.
        unsafe {
            let _ = ShowWindow(overlay_hwnd, SW_HIDE);
            let before = GetForegroundWindow();
            let _ = ShowWindow(overlay_hwnd, SW_SHOWNOACTIVATE);
            thread::sleep(Duration::from_millis(2));
            let after = GetForegroundWindow();
            if before != main_hwnd {
                target_mismatches += 1;
            }
            if after != before {
                focus_steals += 1;
            }
        }
    }
    // SAFETY: style lookup and hide operate on the live overlay HWND.
    let styles = unsafe {
        let _ = ShowWindow(overlay_hwnd, SW_HIDE);
        GetWindowLongPtrW(overlay_hwnd, GWL_EXSTYLE) as u32
    };
    let required = WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0;
    let required_styles_present = styles & required == required;

    Ok(OverlayReport {
        schema_version: 1,
        shell_version: env!("CARGO_PKG_VERSION"),
        window_kind: "tauri_webview2",
        cycles: 100,
        target_mismatches,
        focus_steals,
        required_styles_present,
        passed: target_mismatches == 0 && focus_steals == 0 && required_styles_present,
        content_logged: false,
    })
}

fn wait_for_stable_foreground(
    expected: windows::Win32::Foundation::HWND,
) -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let required_stability = Duration::from_millis(250);
    let mut stable_since = None;

    while Instant::now() < deadline {
        // SAFETY: foreground lookup is read-only.
        if unsafe { GetForegroundWindow() } == expected {
            let since = stable_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= required_stability {
                return Ok(());
            }
        } else {
            stable_since = None;
        }
        thread::sleep(Duration::from_millis(10));
    }

    Err("Tauri main window did not remain foreground for 250 ms; overlay cycles were not run")
}

fn report_path() -> Option<PathBuf> {
    if let Some(path) = std::env::args().find_map(|argument| {
        argument
            .strip_prefix("--m0-overlay-report=")
            .map(PathBuf::from)
    }) {
        return Some(path);
    }
    let executable = std::env::current_exe().ok()?;
    let repository = executable.parent()?.parent()?.parent()?;
    Some(repository.join(format!(
        "artifacts/win32-spike/tauri-overlay-{}.json",
        std::process::id()
    )))
}

fn write_report(path: &PathBuf, report: &OverlayReport) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.part");
    std::fs::write(
        &temporary,
        format!("{}\n", serde_json::to_string_pretty(report)?),
    )?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
