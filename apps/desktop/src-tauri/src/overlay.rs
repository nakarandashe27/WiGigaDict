use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindowLongPtrW, HWND_TOPMOST,
    SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, WS_BORDER, WS_CAPTION,
    WS_DLGFRAME, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_NOACTIVATE, WS_EX_STATICEDGE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
    WS_SYSMENU, WS_THICKFRAME,
};

const OVERLAY_LABEL: &str = "overlay";
const TERMINAL_VISIBILITY: Duration = Duration::from_millis(3_200);
const WORK_AREA_MARGIN: i32 = 48;
/// Logical size of the overlay window. It is a tight box around the HUD pill plus the few
/// pixels its own CSS shadow needs, so no native surface is visible around the pill.
const STATUS_WIDTH: i32 = 176;
const STATUS_HEIGHT: i32 = 56;
const DEFAULT_DPI: u32 = 96;
#[cfg(debug_assertions)]
const DEV_CYCLE_LEVEL_INTERVAL: Duration = Duration::from_millis(66);
#[cfg(debug_assertions)]
const DEV_CYCLE_LEVELS: &[f32] = &[
    0.05, 0.08, 0.14, 0.24, 0.42, 0.66, 0.38, 0.18, 0.09, 0.31, 0.58, 0.27, 0.11, 0.06,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPhase {
    Preparing,
    Recording,
    Processing,
    Delivered,
    Uncertain,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayStatus {
    pub phase: OverlayPhase,
    pub session_id: Option<String>,
    pub reason: Option<String>,
}

impl OverlayStatus {
    pub fn new(phase: OverlayPhase, session_id: Option<String>, reason: Option<&str>) -> Self {
        Self {
            phase,
            session_id,
            reason: reason.map(str::to_owned),
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            OverlayPhase::Delivered | OverlayPhase::Uncertain | OverlayPhase::Error
        )
    }
}

pub struct OverlayService {
    generation: Arc<AtomicU64>,
    current: Arc<Mutex<Option<OverlayStatus>>>,
    presentation: Arc<Mutex<()>>,
    /// Resolved once, on the UI thread, in [`OverlayService::configure`].
    ///
    /// `WebviewWindow::hwnd()` is a runtime getter: it posts a message to the UI thread and waits
    /// for the answer. `publish` runs on the capture worker and on the ASR thread, so calling it
    /// there deadlocked against any synchronous command that was itself waiting for the capture
    /// worker — every cancellation and every safety stop spent the full two second timeout and
    /// then reported a failure for work that had actually succeeded. The overlay window is
    /// created once and never recreated, so its handle is stable for the process lifetime.
    hwnd: Arc<AtomicIsize>,
}

impl OverlayService {
    pub fn new() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(0)),
            current: Arc::new(Mutex::new(None)),
            presentation: Arc::new(Mutex::new(())),
            hwnd: Arc::new(AtomicIsize::new(0)),
        }
    }

    pub fn configure(&self, app: &AppHandle) -> Result<(), String> {
        let overlay = app
            .get_webview_window(OVERLAY_LABEL)
            .ok_or_else(|| "configured overlay window is missing".to_owned())?;
        overlay
            .set_focusable(false)
            .map_err(|_| "overlay could not be made non-focusable".to_owned())?;
        let hwnd = overlay
            .hwnd()
            .map_err(|_| "overlay native handle is unavailable".to_owned())?;
        self.hwnd.store(hwnd.0 as isize, Ordering::SeqCst);
        apply_overlay_frame(hwnd);
        hide_window(hwnd);
        Ok(())
    }

    /// The cached handle, or `None` before `configure` resolved it.
    fn native_window(&self) -> Option<HWND> {
        let raw = self.hwnd.load(Ordering::SeqCst);
        (raw != 0).then_some(HWND(raw as *mut std::ffi::c_void))
    }

    pub fn publish_capture(&self, app: &AppHandle, next: OverlayStatus) {
        self.publish(app, next, false);
    }

    pub fn publish_pipeline(&self, app: &AppHandle, next: OverlayStatus) {
        self.publish(app, next, true);
    }

    fn publish(&self, app: &AppHandle, next: OverlayStatus, require_current_session: bool) {
        let Ok(_presentation) = self.presentation.lock() else {
            return;
        };
        let accepted = if let Ok(mut current) = self.current.lock() {
            if !should_present(current.as_ref(), &next, require_current_session) {
                false
            } else {
                *current = Some(next.clone());
                true
            }
        } else {
            false
        };
        if !accepted {
            return;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = app.emit_to(OVERLAY_LABEL, "overlay-status", next.clone());
        let _ = app.emit_to("main", "overlay-status", next.clone());
        if let Some(hwnd) = self.native_window() {
            apply_overlay_frame(hwnd);
            show_near_foreground(hwnd);
        }
        if next.is_terminal() {
            let current_generation = self.generation.clone();
            let current = self.current.clone();
            let presentation = self.presentation.clone();
            let handle = self.hwnd.clone();
            thread::spawn(move || {
                thread::sleep(TERMINAL_VISIBILITY);
                let Ok(_presentation) = presentation.lock() else {
                    return;
                };
                if current_generation.load(Ordering::SeqCst) == generation {
                    if let Ok(mut current) = current.lock() {
                        *current = None;
                    }
                    let raw = handle.load(Ordering::SeqCst);
                    if raw != 0 {
                        hide_window(HWND(raw as *mut std::ffi::c_void));
                    }
                }
            });
        }
    }

    pub fn hide(&self, _app: &AppHandle) {
        let Ok(_presentation) = self.presentation.lock() else {
            return;
        };
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut current) = self.current.lock() {
            *current = None;
        }
        if let Some(hwnd) = self.native_window() {
            hide_window(hwnd);
        }
    }
}

#[cfg(debug_assertions)]
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DevCaptureLevel {
    session_id: &'static str,
    level: f32,
}

#[cfg(debug_assertions)]
pub fn start_dev_cycle(app: AppHandle) -> Result<(), String> {
    thread::Builder::new()
        .name("wigigadict-overlay-dev-cycle".into())
        .spawn(move || {
            const SESSION_ID: &str = "dev-overlay-cycle";
            loop {
                if app.get_webview_window(OVERLAY_LABEL).is_none() {
                    return;
                }
                let Some(overlay) = app.try_state::<OverlayService>() else {
                    return;
                };
                overlay.publish_capture(
                    &app,
                    OverlayStatus::new(OverlayPhase::Preparing, Some(SESSION_ID.into()), None),
                );
                thread::sleep(Duration::from_millis(220));
                overlay.publish_capture(
                    &app,
                    OverlayStatus::new(OverlayPhase::Recording, Some(SESSION_ID.into()), None),
                );
                for &level in DEV_CYCLE_LEVELS {
                    let _ = app.emit_to(
                        OVERLAY_LABEL,
                        "capture-level",
                        DevCaptureLevel {
                            session_id: SESSION_ID,
                            level,
                        },
                    );
                    thread::sleep(DEV_CYCLE_LEVEL_INTERVAL);
                }
                overlay.publish_capture(
                    &app,
                    OverlayStatus::new(OverlayPhase::Processing, Some(SESSION_ID.into()), None),
                );
                thread::sleep(Duration::from_millis(800));
            }
        })
        .map(|_| ())
        .map_err(|_| "dev overlay cycle thread could not start".to_owned())
}
fn pipeline_owns_current(current: Option<&OverlayStatus>, next: &OverlayStatus) -> bool {
    next.session_id.is_some() && current.is_some_and(|value| value.session_id == next.session_id)
}

fn should_present(
    current: Option<&OverlayStatus>,
    next: &OverlayStatus,
    require_current_session: bool,
) -> bool {
    (!require_current_session || pipeline_owns_current(current, next)) && current != Some(next)
}

fn scaled(logical: i32, dpi: u32) -> i32 {
    ((i64::from(logical) * i64::from(dpi) + i64::from(DEFAULT_DPI / 2)) / i64::from(DEFAULT_DPI))
        as i32
}

/// Physical placement of the HUD on the monitor that currently owns the foreground window.
fn overlay_placement(work_area: (i32, i32, i32, i32), dpi: u32) -> (i32, i32, i32, i32) {
    let (left, _top, right, bottom) = work_area;
    let width = scaled(STATUS_WIDTH, dpi);
    let height = scaled(STATUS_HEIGHT, dpi);
    let x = left + (right - left - width) / 2;
    let y = bottom - height - scaled(WORK_AREA_MARGIN, dpi);
    (x, y, width, height)
}

/// Removes every native frame element from the overlay HWND.
///
/// `tao` keeps `WS_CAPTION | WS_SYSMENU` on the real window even for `decorations: false` and only
/// strips them when it calls `AdjustWindowRectEx`. That mismatch makes DWM paint a rectangular
/// caption with a system close button around the HUD, and it makes every `WM_DPICHANGED` shrink the
/// window by one caption/frame. Clearing the frame styles for real fixes both.
/// Every frame style that would make Windows paint a caption, a border or a system close button.
const FRAME_STYLES: u32 = WS_CAPTION.0
    | WS_BORDER.0
    | WS_DLGFRAME.0
    | WS_SYSMENU.0
    | WS_THICKFRAME.0
    | WS_MINIMIZEBOX.0
    | WS_MAXIMIZEBOX.0;
const EDGE_STYLES: u32 =
    WS_EX_WINDOWEDGE.0 | WS_EX_CLIENTEDGE.0 | WS_EX_DLGMODALFRAME.0 | WS_EX_STATICEDGE.0;

fn overlay_window_style(current: u32) -> u32 {
    (current & !FRAME_STYLES) | WS_POPUP.0
}

fn overlay_extended_style(current: u32) -> u32 {
    (current & !EDGE_STYLES) | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0
}

fn apply_overlay_frame(hwnd: HWND) {
    // SAFETY: hwnd belongs to the configured live Tauri overlay window.
    unsafe {
        let current_style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let current_extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let style = overlay_window_style(current_style);
        let extended = overlay_extended_style(current_extended);
        // Reading a style never leaves this thread, but writing one sends WM_STYLECHANGING and
        // WM_STYLECHANGED to the owning thread and waits for it. `publish` runs on the capture
        // worker and on the ASR thread, so an unconditional write deadlocked against any
        // synchronous command that was waiting for the capture worker: every cancellation and
        // every safety stop burned the full two second acknowledgement timeout and then reported
        // a failure for work that had already succeeded. Measured at 2013 ms inside this call.
        //
        // The frame is still verified on every show, exactly as ADR-007 requires; it is only
        // written when `tao` has actually put a caption back.
        if style == current_style && extended == current_extended {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended as isize);

        // Without this DWM still draws a rectangular drop shadow around the transparent window.
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            std::ptr::from_ref(&policy).cast(),
            std::mem::size_of_val(&policy) as u32,
        );

        // SWP_ASYNCWINDOWPOS is not cosmetic here. `publish` runs on the capture worker and on
        // the ASR thread, and a synchronous cross-thread SetWindowPos waits for the UI thread to
        // process the frame change. When the UI thread was itself blocked inside a synchronous
        // command (every `cancel_capture`, every safety stop) that wait deadlocked until the two
        // second acknowledgement timeout, and the owner was told a cancellation had failed after
        // it had actually succeeded. It also put the audio drain behind the UI thread.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_ASYNCWINDOWPOS
                | SWP_FRAMECHANGED
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOACTIVATE,
        );
    }
}

fn monitor_dpi(monitor: HMONITOR) -> u32 {
    let mut dpi_x = DEFAULT_DPI;
    let mut dpi_y = DEFAULT_DPI;
    // SAFETY: monitor comes from MonitorFromWindow and both outputs are writable.
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err()
        || dpi_x == 0
    {
        return DEFAULT_DPI;
    }
    dpi_x
}

fn show_near_foreground(hwnd: HWND) {
    // SAFETY: all calls inspect live desktop windows and reposition the live overlay without
    // activation. The size is recomputed from the target monitor DPI instead of being carried
    // over, so moving between 100/125/150% monitors cannot accumulate drift.
    unsafe {
        let foreground = GetForegroundWindow();
        let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
        let mut monitor_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            return;
        }
        let work = monitor_info.rcWork;
        let (x, y, width, height) = overlay_placement(
            (work.left, work.top, work.right, work.bottom),
            monitor_dpi(monitor),
        );
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

fn hide_window(hwnd: HWND) {
    // SAFETY: hwnd belongs to the configured live Tauri overlay window. `ShowWindow` sends
    // `WM_SHOWWINDOW` to the owning thread and would block the caller for the same reason
    // `apply_overlay_frame` would; the asynchronous request carries no such dependency.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_ASYNCWINDOWPOS
                | SWP_HIDEWINDOW
                | SWP_NOACTIVATE
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_evidence_terminal_phases_auto_hide() {
        for phase in [
            OverlayPhase::Preparing,
            OverlayPhase::Recording,
            OverlayPhase::Processing,
        ] {
            assert!(!OverlayStatus::new(phase, None, None).is_terminal());
        }
        for phase in [
            OverlayPhase::Delivered,
            OverlayPhase::Uncertain,
            OverlayPhase::Error,
        ] {
            assert!(OverlayStatus::new(phase, None, None).is_terminal());
        }
    }

    #[test]
    fn overlay_status_never_contains_transcript_content() {
        let serialized = serde_json::to_value(OverlayStatus::new(
            OverlayPhase::Uncertain,
            Some("session-1".into()),
            Some("delivery_unconfirmed"),
        ))
        .unwrap();
        assert_eq!(
            serialized.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["phase", "reason", "sessionId"]
        );
    }

    #[test]
    fn pipeline_events_cannot_reopen_or_replace_an_owned_overlay() {
        let session_one =
            OverlayStatus::new(OverlayPhase::Recording, Some("session-1".into()), None);
        let same = OverlayStatus::new(OverlayPhase::Processing, Some("session-1".into()), None);
        let stale = OverlayStatus::new(OverlayPhase::Processing, Some("session-old".into()), None);

        assert!(!pipeline_owns_current(None, &same));
        assert!(pipeline_owns_current(Some(&session_one), &same));
        assert!(!pipeline_owns_current(Some(&session_one), &stale));
    }

    #[test]
    fn repeated_status_does_not_reemit_or_reposition_the_native_window() {
        let recording = OverlayStatus::new(OverlayPhase::Recording, Some("session-1".into()), None);
        let processing =
            OverlayStatus::new(OverlayPhase::Processing, Some("session-1".into()), None);

        assert!(!should_present(Some(&recording), &recording, false));
        assert!(should_present(Some(&recording), &processing, true));
    }

    #[test]
    fn overlay_native_window_uses_one_phase_invariant_size() {
        assert_eq!((STATUS_WIDTH, STATUS_HEIGHT), (176, 56));
    }

    #[test]
    fn placement_is_derived_from_the_target_monitor_dpi_and_never_drifts() {
        // A 1920x1040 work area at 100%, 125% and 150%.
        let work = (0, 0, 1920, 1040);
        let (x, y, width, height) = overlay_placement(work, 96);
        assert_eq!((width, height), (176, 56));
        assert_eq!(x, (1920 - 176) / 2);
        assert_eq!(y, 1040 - 56 - 48);

        assert_eq!(overlay_placement(work, 120).2, 220);
        assert_eq!(overlay_placement(work, 120).3, 70);
        assert_eq!(overlay_placement(work, 144).2, 264);
        assert_eq!(overlay_placement(work, 144).3, 84);

        // Repeating the computation for the same monitor is idempotent: the size is always
        // recomputed from the logical constants, never from the current window rect.
        assert_eq!(overlay_placement(work, 144), overlay_placement(work, 144));
    }

    #[test]
    fn every_native_overlay_call_is_asynchronous_for_the_owning_thread() {
        // `publish` runs on the capture worker and on the ASR thread. A synchronous cross-thread
        // SetWindowPos waits for the UI thread, which deadlocked every cancellation against the
        // two second acknowledgement timeout and put the audio drain behind the UI thread.
        let source = include_str!("overlay.rs");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("test module marker")
            .0;
        let call_sites = production
            .match_indices("SetWindowPos(")
            .collect::<Vec<_>>();
        assert_eq!(call_sites.len(), 4, "a new call site must be reviewed here");
        for (offset, _) in call_sites {
            let arguments = &production[offset..production.len().min(offset + 400)];
            assert!(
                arguments.contains("SWP_ASYNCWINDOWPOS"),
                "SetWindowPos at byte {offset} must pass SWP_ASYNCWINDOWPOS"
            );
        }
        assert!(
            !production.contains("ShowWindow("),
            "ShowWindow sends WM_SHOWWINDOW and blocks on the owning thread"
        );
    }

    #[test]
    fn an_already_correct_frame_is_never_rewritten() {
        // Writing a style blocks on the owning thread, so the frame must only be written when
        // `tao` has actually put a caption back. Both helpers are idempotent, which is what lets
        // `apply_overlay_frame` compare first and skip the cross-thread write.
        let settled_style = overlay_window_style(0x14CB_0000);
        let settled_extended = overlay_extended_style(0x0804_0198);
        assert_eq!(overlay_window_style(settled_style), settled_style);
        assert_eq!(overlay_extended_style(settled_extended), settled_extended);
        // A caption that came back must still be detected as a difference.
        assert_ne!(
            overlay_window_style(settled_style | WS_CAPTION.0),
            settled_style | WS_CAPTION.0
        );

        let production = include_str!("overlay.rs")
            .split_once("#[cfg(test)]")
            .expect("test module marker")
            .0;
        let guard = production
            .split_once("fn apply_overlay_frame")
            .expect("frame helper")
            .1;
        let write = guard.find("SetWindowLongPtrW").expect("style write");
        let skip = guard.find("return;").expect("early return");
        assert!(
            skip < write,
            "apply_overlay_frame must return before writing an unchanged style"
        );
    }

    #[test]
    fn overlay_frame_removes_every_caption_and_system_button_style() {
        // The style tao actually leaves on an undecorated window (observed 0x14CB0000).
        let observed = 0x14CB_0000_u32;
        let stripped = overlay_window_style(observed);
        for framed in [
            WS_CAPTION.0,
            WS_BORDER.0,
            WS_DLGFRAME.0,
            WS_SYSMENU.0,
            WS_THICKFRAME.0,
            WS_MINIMIZEBOX.0,
            WS_MAXIMIZEBOX.0,
        ] {
            assert_eq!(stripped & framed, 0, "frame style {framed:#x} survived");
        }
        assert_eq!(stripped & WS_POPUP.0, WS_POPUP.0);
        // Visibility and clipping flags must survive untouched.
        assert_eq!(stripped & 0x1000_0000, 0x1000_0000);
        assert_eq!(
            overlay_window_style(stripped),
            stripped,
            "must be idempotent"
        );
    }

    #[test]
    fn overlay_extended_style_keeps_the_hud_unfocusable_and_edgeless() {
        let observed = 0x0804_0198_u32;
        let applied = overlay_extended_style(observed);
        assert_eq!(applied & WS_EX_NOACTIVATE.0, WS_EX_NOACTIVATE.0);
        assert_eq!(applied & WS_EX_TOOLWINDOW.0, WS_EX_TOOLWINDOW.0);
        assert_eq!(applied & WS_EX_TOPMOST.0, WS_EX_TOPMOST.0);
        assert_eq!(applied & EDGE_STYLES, 0);
        assert_eq!(
            overlay_extended_style(applied),
            applied,
            "must be idempotent"
        );
    }

    #[test]
    fn placement_follows_the_monitor_origin_offset() {
        let secondary = (2560, 211, 4480, 1251);
        let (x, y, width, height) = overlay_placement(secondary, 96);
        assert_eq!((width, height), (176, 56));
        assert_eq!(x, 2560 + (1920 - 176) / 2);
        assert_eq!(y, 1251 - 56 - 48);
    }
}
