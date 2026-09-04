use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GlobalFree, HANDLE, HWND, LPARAM, LRESULT, WPARAM},
    Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    },
    System::{
        DataExchange::{
            CloseClipboard, CountClipboardFormats, EmptyClipboard, GetClipboardData,
            IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
        },
        LibraryLoader::GetModuleHandleW,
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        SystemInformation::OSVERSIONINFOW,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
            SendInput, SetFocus, VK_CONTROL, VK_F8,
        },
        WindowsAndMessaging::{
            CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
            ES_AUTOHSCROLL, ES_MULTILINE, EnumWindows, GUITHREADINFO, GWL_EXSTYLE, GetClassNameW,
            GetForegroundWindow, GetGUIThreadInfo, GetWindowLongPtrW, GetWindowTextW,
            GetWindowThreadProcessId, HHOOK, IsWindow, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
            PM_REMOVE, PeekMessageW, RegisterClassW, SW_HIDE, SW_RESTORE, SW_SHOW,
            SW_SHOWNOACTIVATE, SetForegroundWindow, SetWindowsHookExW, ShowWindow,
            TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WNDCLASSW,
            WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW,
            WS_VISIBLE,
        },
    },
};

use crate::{
    DeliveryStatus, EvidenceLevel, FailureCode, HotkeyState, HotkeyTransition, InsertionMethod,
    MethodEvidence,
};

const MARKER: &str = "WiGigaDict_M0_Привет";
const MEDIUM_INTEGRITY_RID: u32 = 0x2000;
const FIXTURE_CLASS: &str = "WiGigaDictM0Win32SpikeFixture";

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentEvidence {
    pub os_family: String,
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub architecture: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetEvidence {
    pub hwnd: u64,
    pub process_id: u32,
    pub thread_id: u32,
    pub process_name: String,
    pub window_class: String,
    pub focused_control_class: String,
    pub integrity_rid: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetSnapshot {
    hwnd: HWND,
    process_id: u32,
    thread_id: u32,
    process_name: String,
    window_class: String,
    focused_control_class: String,
    integrity_rid: u32,
}

impl TargetSnapshot {
    fn evidence(&self) -> TargetEvidence {
        TargetEvidence {
            hwnd: self.hwnd as usize as u64,
            process_id: self.process_id,
            thread_id: self.thread_id,
            process_name: self.process_name.clone(),
            window_class: self.window_class.clone(),
            focused_control_class: self.focused_control_class.clone(),
            integrity_rid: self.integrity_rid,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckEvidence {
    pub id: String,
    pub passed: bool,
    pub status: DeliveryStatus,
    pub level: EvidenceLevel,
    pub failure: Option<FailureCode>,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AutomatedMatrixReport {
    pub schema_version: u32,
    pub harness_version: String,
    pub environment: EnvironmentEvidence,
    pub fixture_target: TargetEvidence,
    pub checks: Vec<CheckEvidence>,
    pub content_logged: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExternalProbeReport {
    pub schema_version: u32,
    pub harness_version: String,
    pub surface: String,
    pub environment: EnvironmentEvidence,
    pub target: TargetEvidence,
    pub overlay_cycles: u32,
    pub overlay_focus_steals: u32,
    pub insertion: MethodEvidence,
    pub target_retained_after_input: bool,
    pub observer_required: bool,
    pub observer_kind: String,
    pub observer_acknowledged: bool,
    pub passed: bool,
    pub content_logged: bool,
}

impl ExternalProbeReport {
    #[must_use]
    pub fn all_required_passed(&self) -> bool {
        self.passed && !self.content_logged
    }
}

impl AutomatedMatrixReport {
    #[must_use]
    pub fn all_required_passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed) && !self.content_logged
    }

    pub fn write_json(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(self)?))?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HookEvent {
    is_down: bool,
    injected: bool,
}

static HOOK_EVENTS: OnceLock<Mutex<Vec<HookEvent>>> = OnceLock::new();

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam as u32, WM_KEYDOWN | WM_KEYUP) {
        // SAFETY: Windows supplies a valid KBDLLHOOKSTRUCT for low-level keyboard messages.
        let data = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if data.vkCode == u32::from(VK_F8) {
            let events = HOOK_EVENTS.get_or_init(|| Mutex::new(Vec::new()));
            if let Ok(mut events) = events.lock() {
                events.push(HookEvent {
                    is_down: wparam as u32 == WM_KEYDOWN,
                    injected: data.flags & LLKHF_INJECTED != 0,
                });
            }
        }
    }

    // SAFETY: forwarding is required by the WH_KEYBOARD_LL hook contract.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn fixture_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the default procedure receives the original valid window-procedure arguments.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

struct HookGuard(HHOOK);

impl Drop for HookGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this process owns the hook handle.
            unsafe { UnhookWindowsHookEx(self.0) };
        }
    }
}

struct WindowFixture {
    parent: HWND,
    edit: HWND,
    overlay: HWND,
}

impl WindowFixture {
    fn create() -> Result<Self, String> {
        let fixture_class = wide(FIXTURE_CLASS);
        let edit_class = wide("EDIT");
        let target_title = wide("WiGigaDict M0 target fixture");
        let overlay_title = wide("WiGigaDict M0 no-activate overlay");
        // SAFETY: null requests the current executable module.
        let instance = unsafe { GetModuleHandleW(null()) };
        if instance.is_null() {
            return Err(last_error("GetModuleHandleW"));
        }
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(fixture_window_proc),
            hInstance: instance,
            lpszClassName: fixture_class.as_ptr(),
            ..Default::default()
        };
        // SAFETY: class fields point to code and data alive for this process.
        if unsafe { RegisterClassW(&window_class) } == 0 {
            return Err(last_error("RegisterClassW"));
        }

        // SAFETY: all class/title pointers remain alive for each call; parent handles are valid.
        let parent = unsafe {
            CreateWindowExW(
                0,
                fixture_class.as_ptr(),
                target_title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                640,
                240,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if parent.is_null() {
            return Err(last_error("CreateWindowExW(target)"));
        }

        // SAFETY: parent is a live window owned by this process.
        let edit = unsafe {
            CreateWindowExW(
                0,
                edit_class.as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | ES_MULTILINE as u32 | ES_AUTOHSCROLL as u32,
                16,
                16,
                590,
                150,
                parent,
                null_mut(),
                instance,
                null(),
            )
        };
        if edit.is_null() {
            // SAFETY: parent was created above.
            unsafe { DestroyWindow(parent) };
            return Err(last_error("CreateWindowExW(edit)"));
        }

        // SAFETY: built-in class and pointers are valid for this call.
        let overlay = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                fixture_class.as_ptr(),
                overlay_title.as_ptr(),
                WS_VISIBLE,
                180,
                180,
                360,
                80,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if overlay.is_null() {
            // SAFETY: both handles were created above.
            unsafe {
                DestroyWindow(edit);
                DestroyWindow(parent);
            }
            return Err(last_error("CreateWindowExW(overlay)"));
        }
        // SAFETY: handles are live and owned by this process.
        unsafe {
            ShowWindow(overlay, SW_HIDE);
            ShowWindow(parent, SW_SHOW);
            SetForegroundWindow(parent);
            SetFocus(edit);
        }
        pump_messages();

        Ok(Self {
            parent,
            edit,
            overlay,
        })
    }

    fn clear_edit(&self) {
        let empty = wide("");
        // SAFETY: edit is live for the fixture lifetime.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(self.edit, empty.as_ptr());
            SetForegroundWindow(self.parent);
            SetFocus(self.edit);
        }
        pump_messages();
    }

    fn target_acked(&self) -> bool {
        pump_messages();
        let mut buffer = vec![0_u16; 256];
        // SAFETY: edit is live and buffer is writable.
        let length = unsafe { GetWindowTextW(self.edit, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize]) == MARKER
    }
}

impl Drop for WindowFixture {
    fn drop(&mut self) {
        // SAFETY: duplicate destruction is avoided; child destruction with parent is harmlessly
        // handled by Windows, so only top-level owned handles are destroyed explicitly.
        unsafe {
            if !self.overlay.is_null() && IsWindow(self.overlay) != 0 {
                DestroyWindow(self.overlay);
            }
            if !self.parent.is_null() && IsWindow(self.parent) != 0 {
                DestroyWindow(self.parent);
            }
        }
    }
}

struct ExternalOverlay(HWND);

impl ExternalOverlay {
    fn create() -> Result<Self, String> {
        let fixture_class = wide(FIXTURE_CLASS);
        let overlay_title = wide("WiGigaDict M0 external no-activate overlay");
        // SAFETY: null requests the current executable module.
        let instance = unsafe { GetModuleHandleW(null()) };
        if instance.is_null() {
            return Err(last_error("GetModuleHandleW"));
        }
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(fixture_window_proc),
            hInstance: instance,
            lpszClassName: fixture_class.as_ptr(),
            ..Default::default()
        };
        // SAFETY: class fields point to code and data alive for this process.
        if unsafe { RegisterClassW(&window_class) } == 0 {
            return Err(last_error("RegisterClassW(external overlay)"));
        }
        // SAFETY: class and title pointers remain valid for the call; the window starts hidden.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                fixture_class.as_ptr(),
                overlay_title.as_ptr(),
                0,
                180,
                180,
                360,
                80,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if hwnd.is_null() {
            return Err(last_error("CreateWindowExW(external overlay)"));
        }
        Ok(Self(hwnd))
    }
}

impl Drop for ExternalOverlay {
    fn drop(&mut self) {
        // SAFETY: this process owns the overlay HWND.
        unsafe {
            if !self.0.is_null() && IsWindow(self.0) != 0 {
                DestroyWindow(self.0);
            }
        }
    }
}

pub fn run_external_probe(
    surface: &str,
    expected_process: &str,
    activation_title: Option<&str>,
    observer_title_marker: Option<&str>,
    prepare_vscode_editor: bool,
) -> Result<ExternalProbeReport, Box<dyn std::error::Error>> {
    let mut snapshot = match activation_title {
        Some(title) => activate_expected_window(expected_process, title)?,
        None => wait_for_expected_foreground(expected_process, Duration::from_secs(60))?,
    };
    if prepare_vscode_editor {
        focus_vscode_editor()?;
        thread::sleep(Duration::from_millis(100));
        snapshot = capture_foreground_target()?;
        if !snapshot.process_name.eq_ignore_ascii_case(expected_process) {
            return Err(
                "VS Code focus preparation changed the target process; no marker was injected"
                    .into(),
            );
        }
    }
    let caller_integrity = current_process_integrity()?;
    validate_integrity(caller_integrity, snapshot.integrity_rid)
        .map_err(|failure| format!("target integrity rejected: {failure:?}"))?;
    let overlay = ExternalOverlay::create()?;
    let mut overlay_focus_steals = 0_u32;
    let title_before_input = window_title(snapshot.hwnd)?;

    for _ in 0..100 {
        // SAFETY: the overlay HWND is live and foreground lookup is read-only.
        unsafe {
            let _ = ShowWindow(overlay.0, SW_HIDE);
            let before = GetForegroundWindow();
            let _ = ShowWindow(overlay.0, SW_SHOWNOACTIVATE);
            thread::sleep(Duration::from_millis(2));
            let after = GetForegroundWindow();
            if before != snapshot.hwnd || after != before {
                overlay_focus_steals += 1;
            }
        }
    }
    // SAFETY: the overlay HWND is live.
    unsafe {
        let _ = ShowWindow(overlay.0, SW_HIDE);
    }

    let insertion = if overlay_focus_steals == 0 && revalidate_target(&snapshot).is_ok() {
        send_unicode(MARKER)
    } else {
        MethodEvidence::failed(InsertionMethod::UnicodePacket, FailureCode::FocusChanged)
    };
    pump_messages();
    thread::sleep(Duration::from_millis(100));
    let target_retained_after_input = revalidate_target(&snapshot).is_ok();
    let observer_required = observer_title_marker.is_some() || prepare_vscode_editor;
    let mut title_after_input = window_title(snapshot.hwnd)?;
    if observer_required {
        let observer_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < observer_deadline {
            let acknowledged = observer_title_marker.map_or_else(
                || prepare_vscode_editor && title_after_input != title_before_input,
                |marker| title_after_input.contains(marker),
            );
            if acknowledged {
                break;
            }
            thread::sleep(Duration::from_millis(50));
            title_after_input = window_title(snapshot.hwnd)?;
        }
    }
    let (observer_kind, observer_acknowledged) = if let Some(marker) = observer_title_marker {
        ("window_title_marker", title_after_input.contains(marker))
    } else if prepare_vscode_editor {
        (
            "window_title_changed",
            title_after_input != title_before_input,
        )
    } else {
        ("not_requested", false)
    };
    let passed = overlay_focus_steals == 0
        && target_retained_after_input
        && insertion.expected_input_units == insertion.accepted_input_units
        && insertion.level == EvidenceLevel::TransportOnly
        && insertion.status == DeliveryStatus::Uncertain
        && (!observer_required || observer_acknowledged);

    Ok(ExternalProbeReport {
        schema_version: 1,
        harness_version: env!("CARGO_PKG_VERSION").into(),
        surface: surface.into(),
        environment: environment_evidence(),
        target: snapshot.evidence(),
        overlay_cycles: 100,
        overlay_focus_steals,
        insertion,
        target_retained_after_input,
        observer_required,
        observer_kind: observer_kind.into(),
        observer_acknowledged,
        passed,
        content_logged: false,
    })
}

fn focus_vscode_editor() -> Result<(), String> {
    let inputs = [
        key_input(VK_CONTROL, 0, 0),
        key_input(0x31, 0, 0),
        key_input(0x31, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];
    // SAFETY: input slice is valid for the duration of the call.
    let accepted = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if accepted != inputs.len() as u32 {
        return Err(
            "VS Code editor focus shortcut was not fully accepted; marker was not injected".into(),
        );
    }
    Ok(())
}

fn window_title(hwnd: HWND) -> Result<String, String> {
    let mut title = vec![0_u16; 512];
    // SAFETY: hwnd is live and title is a writable buffer.
    let length = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
    if length == 0 {
        return Ok(String::new());
    }
    Ok(String::from_utf16_lossy(&title[..length as usize]))
}

fn activate_expected_window(
    expected_process: &str,
    title_prefix: &str,
) -> Result<TargetSnapshot, String> {
    let mut search = WindowSearch {
        title_prefix,
        hwnd: null_mut(),
        matches: 0,
    };
    // SAFETY: the callback receives a pointer to search for the duration of this synchronous call.
    unsafe {
        let _ = EnumWindows(
            Some(find_window_by_title_prefix),
            &mut search as *mut WindowSearch<'_> as LPARAM,
        );
    }
    let hwnd = search.hwnd;
    if search.matches == 0 || hwnd.is_null() {
        return Err("disposable target window prefix was not found; no input was injected".into());
    }
    if search.matches != 1 {
        return Err("disposable target window prefix was ambiguous; no input was injected".into());
    }

    // SAFETY: hwnd was returned by FindWindowW.
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        // SAFETY: foreground lookup is read-only.
        if unsafe { GetForegroundWindow() } == hwnd {
            let snapshot = capture_foreground_target()?;
            if snapshot.hwnd != hwnd
                || !snapshot.process_name.eq_ignore_ascii_case(expected_process)
            {
                return Err(
                    "activated window did not match the expected process; no input was injected"
                        .into(),
                );
            }
            return Ok(snapshot);
        }
        thread::sleep(Duration::from_millis(50));
    }

    Err(
        "Windows denied foreground activation for the disposable target; no input was injected"
            .into(),
    )
}

struct WindowSearch<'a> {
    title_prefix: &'a str,
    hwnd: HWND,
    matches: u32,
}

unsafe extern "system" fn find_window_by_title_prefix(hwnd: HWND, context: LPARAM) -> i32 {
    // SAFETY: context points to the WindowSearch owned by activate_expected_window.
    let search = unsafe { &mut *(context as *mut WindowSearch<'_>) };
    let mut title = vec![0_u16; 512];
    // SAFETY: hwnd is supplied by EnumWindows and title is a writable buffer.
    let length = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
    if length > 0
        && String::from_utf16_lossy(&title[..length as usize]).starts_with(search.title_prefix)
    {
        search.matches += 1;
        if search.matches == 1 {
            search.hwnd = hwnd;
        }
    }
    1
}

fn wait_for_expected_foreground(
    expected_process: &str,
    timeout: Duration,
) -> Result<TargetSnapshot, String> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if let Ok(snapshot) = capture_foreground_target()
            && snapshot.process_name.eq_ignore_ascii_case(expected_process)
        {
            return Ok(snapshot);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "expected foreground process {expected_process} was not observed within 60 seconds; no input was injected"
    ))
}

pub fn run_automated_matrix() -> Result<AutomatedMatrixReport, Box<dyn std::error::Error>> {
    let fixture = WindowFixture::create()?;
    wait_for_foreground(fixture.parent, Duration::from_secs(60))?;
    let snapshot = capture_foreground_target()?;
    if snapshot.hwnd != fixture.parent {
        return Err("fixture did not become foreground; refusing to inject input".into());
    }
    let mut checks = Vec::new();

    checks.push(check_hotkey_hook()?);
    checks.push(check_overlay_no_activate(&fixture));

    fixture.clear_edit();
    revalidate_target(&snapshot).map_err(|failure| {
        format!("target revalidation failed before Unicode input: {failure:?}")
    })?;
    let unicode = send_unicode(MARKER);
    pump_messages();
    let unicode = MethodEvidence::from_transport(
        unicode.method,
        unicode.expected_input_units,
        unicode.accepted_input_units,
        fixture.target_acked(),
        false,
    );
    checks.push(check_from_method("unicode_packet_target_ack", unicode));

    fixture.clear_edit();
    revalidate_target(&snapshot).map_err(|failure| {
        format!("target revalidation failed before virtual-key input: {failure:?}")
    })?;
    let virtual_key = send_virtual_key_codes(&[b'A' as u16]);
    checks.push(CheckEvidence {
        id: "virtual_key_transport".into(),
        passed: virtual_key.accepted_input_units == virtual_key.expected_input_units
            && virtual_key.status == DeliveryStatus::Uncertain
            && virtual_key.level == EvidenceLevel::TransportOnly,
        status: virtual_key.status,
        level: virtual_key.level,
        failure: virtual_key.failure,
        detail: "raw virtual-key SendInput accepted every unit but remains transport_only without acknowledgement"
            .into(),
    });

    checks.push(check_focus_change(&snapshot, &fixture));
    checks.push(check_missing_target(&snapshot));
    checks.push(check_integrity_policy(&snapshot));
    checks.push(check_partial_input_policy());
    checks.push(check_clipboard_busy(&fixture));
    checks.push(check_clipboard_restore_policy());

    Ok(AutomatedMatrixReport {
        schema_version: 1,
        harness_version: env!("CARGO_PKG_VERSION").into(),
        environment: environment_evidence(),
        fixture_target: snapshot.evidence(),
        checks,
        content_logged: false,
    })
}

fn wait_for_foreground(hwnd: HWND, timeout: Duration) -> Result<(), String> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        pump_messages();
        // SAFETY: read-only comparison against a live fixture HWND.
        if unsafe { GetForegroundWindow() } == hwnd {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("interactive fixture was not foreground within 60 seconds; no input was injected".into())
}

fn check_hotkey_hook() -> Result<CheckEvidence, String> {
    if let Ok(mut events) = HOOK_EVENTS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        events.clear();
    }
    // SAFETY: callback has static lifetime and the current process module is valid.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            GetModuleHandleW(null()),
            0,
        )
    };
    if hook.is_null() {
        return Err(last_error("SetWindowsHookExW"));
    }
    let _guard = HookGuard(hook);
    let inputs = [
        key_input(VK_F8, 0, 0),
        key_input(VK_F8, 0, 0),
        key_input(VK_F8, 0, KEYEVENTF_KEYUP),
    ];
    // SAFETY: input slice is valid for the duration of the call.
    let accepted = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    pump_messages();
    let events = HOOK_EVENTS
        .get()
        .and_then(|events| events.lock().ok().map(|events| events.clone()))
        .unwrap_or_default();
    let mut state = HotkeyState::default();
    let transitions: Vec<_> = events
        .iter()
        .map(|event| state.observe(event.is_down))
        .collect();
    let passed = accepted == inputs.len() as u32
        && transitions
            == [
                HotkeyTransition::Down,
                HotkeyTransition::RepeatIgnored,
                HotkeyTransition::Up,
            ]
        && events.iter().all(|event| event.injected);

    Ok(CheckEvidence {
        id: "hotkey_down_repeat_up".into(),
        passed,
        status: if passed { DeliveryStatus::Delivered } else { DeliveryStatus::Uncertain },
        level: if passed { EvidenceLevel::TargetAck } else { EvidenceLevel::None },
        failure: (!passed).then_some(FailureCode::Win32CallFailed),
        detail: "WH_KEYBOARD_LL observed one down, ignored repeat, and one up; injected flag was retained"
            .into(),
    })
}

fn check_overlay_no_activate(fixture: &WindowFixture) -> CheckEvidence {
    let mut retained_focus = true;
    for _ in 0..100 {
        // SAFETY: fixture handles are live.
        unsafe {
            ShowWindow(fixture.overlay, SW_HIDE);
            SetForegroundWindow(fixture.parent);
            SetFocus(fixture.edit);
        }
        pump_messages();
        // SAFETY: foreground query is read-only and overlay is a live fixture window.
        let before = unsafe { GetForegroundWindow() };
        unsafe { ShowWindow(fixture.overlay, SW_SHOWNOACTIVATE) };
        pump_messages();
        let after = unsafe { GetForegroundWindow() };
        retained_focus &= before == fixture.parent && after == before;
    }
    // SAFETY: window style lookup is read-only.
    let ex_style = unsafe { GetWindowLongPtrW(fixture.overlay, GWL_EXSTYLE) } as u32;
    let required = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
    let passed = retained_focus && ex_style & required == required;

    CheckEvidence {
        id: "overlay_ws_ex_noactivate".into(),
        passed,
        status: if passed { DeliveryStatus::Delivered } else { DeliveryStatus::Uncertain },
        level: if passed { EvidenceLevel::TargetAck } else { EvidenceLevel::None },
        failure: (!passed).then_some(FailureCode::FocusChanged),
        detail: "foreground HWND remained identical across 100 SW_SHOWNOACTIVATE cycles and required extended styles were present"
            .into(),
    }
}

fn check_focus_change(snapshot: &TargetSnapshot, fixture: &WindowFixture) -> CheckEvidence {
    let other_title = wide("WiGigaDict M0 changed target");
    let class = wide(FIXTURE_CLASS);
    // SAFETY: built-in class and static pointers are valid.
    let other = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            other_title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            760,
            100,
            320,
            180,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    if other.is_null() {
        return failed_check("foreground_change_uncertain", FailureCode::Win32CallFailed);
    }
    // SAFETY: other is live.
    unsafe { SetForegroundWindow(other) };
    pump_messages();
    let failure = revalidate_target(snapshot).expect_err("foreground must differ");
    // SAFETY: restore the fixture and destroy the temporary top-level window.
    unsafe {
        DestroyWindow(other);
        SetForegroundWindow(fixture.parent);
        SetFocus(fixture.edit);
    }
    pump_messages();

    CheckEvidence {
        id: "foreground_change_uncertain".into(),
        passed: failure == FailureCode::FocusChanged,
        status: DeliveryStatus::Uncertain,
        level: EvidenceLevel::None,
        failure: Some(failure),
        detail: "immutable target revalidation rejected a different foreground HWND before input"
            .into(),
    }
}

fn check_missing_target(snapshot: &TargetSnapshot) -> CheckEvidence {
    let mut missing = snapshot.clone();
    missing.hwnd = null_mut();
    let failure = revalidate_target(&missing).expect_err("null HWND must be missing");
    CheckEvidence {
        id: "missing_hwnd_uncertain".into(),
        passed: failure == FailureCode::TargetMissing,
        status: DeliveryStatus::Uncertain,
        level: EvidenceLevel::None,
        failure: Some(failure),
        detail: "destroyed or null target is rejected before input".into(),
    }
}

fn check_integrity_policy(snapshot: &TargetSnapshot) -> CheckEvidence {
    let failure = validate_integrity(snapshot.integrity_rid, snapshot.integrity_rid + 0x1000)
        .expect_err("higher integrity target must be rejected");
    CheckEvidence {
        id: "elevated_target_policy_uncertain".into(),
        passed: failure == FailureCode::ElevatedTarget,
        status: DeliveryStatus::Uncertain,
        level: EvidenceLevel::None,
        failure: Some(failure),
        detail: "a target above the caller integrity RID is rejected; no UAC helper exists".into(),
    }
}

fn check_partial_input_policy() -> CheckEvidence {
    let evidence =
        MethodEvidence::from_transport(InsertionMethod::UnicodePacket, 8, 6, false, false);
    CheckEvidence {
        id: "partial_sendinput_uncertain".into(),
        passed: evidence.status == DeliveryStatus::Uncertain
            && evidence.failure == Some(FailureCode::InputPartiallyAccepted)
            && !evidence.may_fallback(),
        status: evidence.status,
        level: evidence.level,
        failure: evidence.failure,
        detail: "partial SendInput is uncertain and cannot trigger an automatic fallback duplicate"
            .into(),
    }
}

fn check_clipboard_busy(fixture: &WindowFixture) -> CheckEvidence {
    let (opened_tx, opened_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = thread::spawn(move || {
        // SAFETY: the worker deliberately owns the clipboard until the test releases it.
        let opened = unsafe { OpenClipboard(null_mut()) } != 0;
        let _ = opened_tx.send(opened);
        if opened {
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
            // SAFETY: this worker owns the open clipboard.
            unsafe { CloseClipboard() };
        }
    });
    let opened = opened_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or(false);
    if !opened {
        let _ = release_tx.send(());
        let _ = holder.join();
        return failed_check("clipboard_busy_uncertain", FailureCode::Win32CallFailed);
    }
    let evidence = clipboard_paste(MARKER, fixture.parent, false);
    let _ = release_tx.send(());
    let _ = holder.join();
    CheckEvidence {
        id: "clipboard_busy_uncertain".into(),
        passed: evidence.status == DeliveryStatus::Uncertain
            && evidence.failure == Some(FailureCode::ClipboardBusy),
        status: evidence.status,
        level: evidence.level,
        failure: evidence.failure,
        detail: "busy clipboard fails closed without input or retry loop".into(),
    }
}

fn check_clipboard_restore_policy() -> CheckEvidence {
    let evidence = MethodEvidence::failed(
        InsertionMethod::ClipboardPaste,
        FailureCode::ClipboardRestoreFailed,
    );
    CheckEvidence {
        id: "clipboard_restore_failure_uncertain".into(),
        passed: evidence.status == DeliveryStatus::Uncertain
            && evidence.level == EvidenceLevel::None
            && !evidence.may_fallback(),
        status: evidence.status,
        level: evidence.level,
        failure: evidence.failure,
        detail: "failed clipboard restoration remains an explicit terminal uncertain result".into(),
    }
}

fn check_from_method(id: &str, evidence: MethodEvidence) -> CheckEvidence {
    CheckEvidence {
        id: id.into(),
        passed: evidence.status == DeliveryStatus::Delivered
            && evidence.level == EvidenceLevel::TargetAck,
        status: evidence.status,
        level: evidence.level,
        failure: evidence.failure,
        detail: "same-process standard EDIT fixture acknowledged the exact deterministic marker"
            .into(),
    }
}

fn failed_check(id: &str, failure: FailureCode) -> CheckEvidence {
    CheckEvidence {
        id: id.into(),
        passed: false,
        status: DeliveryStatus::Uncertain,
        level: EvidenceLevel::None,
        failure: Some(failure),
        detail: "Win32 fixture setup failed".into(),
    }
}

fn capture_foreground_target() -> Result<TargetSnapshot, String> {
    // SAFETY: foreground lookup is read-only.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
        return Err("foreground target is missing".into());
    }
    let mut process_id = 0;
    // SAFETY: hwnd is live and process_id is writable.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    if thread_id == 0 || process_id == 0 {
        return Err(last_error("GetWindowThreadProcessId"));
    }
    let process = ProcessHandle::open(process_id)?;
    let process_name = process.name()?;
    let integrity_rid = process.integrity_rid()?;

    let mut class = vec![0_u16; 256];
    // SAFETY: hwnd is live and class buffer is writable.
    let class_len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    if class_len == 0 {
        return Err(last_error("GetClassNameW"));
    }
    let focused_control_class = focused_control_class(thread_id)?;

    Ok(TargetSnapshot {
        hwnd,
        process_id,
        thread_id,
        process_name,
        window_class: String::from_utf16_lossy(&class[..class_len as usize]),
        focused_control_class,
        integrity_rid,
    })
}

fn focused_control_class(thread_id: u32) -> Result<String, String> {
    // SAFETY: the structure is initialized with its documented size and is writable.
    let mut info: GUITHREADINFO = unsafe { zeroed() };
    info.cbSize = size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(thread_id, &mut info) } == 0 {
        return Err(last_error("GetGUIThreadInfo"));
    }
    if info.hwndFocus.is_null() {
        return Ok("none".into());
    }
    let mut class = vec![0_u16; 256];
    // SAFETY: hwndFocus came from GetGUIThreadInfo and the class buffer is writable.
    let class_len =
        unsafe { GetClassNameW(info.hwndFocus, class.as_mut_ptr(), class.len() as i32) };
    if class_len == 0 {
        return Err(last_error("GetClassNameW(focused control)"));
    }
    Ok(String::from_utf16_lossy(&class[..class_len as usize]))
}

fn revalidate_target(snapshot: &TargetSnapshot) -> Result<(), FailureCode> {
    if snapshot.hwnd.is_null() || unsafe { IsWindow(snapshot.hwnd) } == 0 {
        return Err(FailureCode::TargetMissing);
    }
    if unsafe { GetForegroundWindow() } != snapshot.hwnd {
        return Err(FailureCode::FocusChanged);
    }
    let current = capture_foreground_target().map_err(|_| FailureCode::TargetMissing)?;
    if current != *snapshot {
        return Err(FailureCode::FocusChanged);
    }
    let caller_integrity = current_process_integrity().map_err(|_| FailureCode::Win32CallFailed)?;
    validate_integrity(caller_integrity, current.integrity_rid)
}

fn validate_integrity(caller_rid: u32, target_rid: u32) -> Result<(), FailureCode> {
    if target_rid > caller_rid || target_rid > MEDIUM_INTEGRITY_RID {
        Err(FailureCode::ElevatedTarget)
    } else {
        Ok(())
    }
}

fn send_unicode(text: &str) -> MethodEvidence {
    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    // SAFETY: input slice is valid for the duration of the call.
    let accepted = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    MethodEvidence::from_transport(
        InsertionMethod::UnicodePacket,
        inputs.len() as u32,
        accepted,
        false,
        false,
    )
}

fn send_virtual_key_codes(keys: &[u16]) -> MethodEvidence {
    let mut inputs = Vec::new();
    for vk in keys {
        inputs.push(key_input(*vk, 0, 0));
        inputs.push(key_input(*vk, 0, KEYEVENTF_KEYUP));
    }
    // SAFETY: input slice is valid for the duration of the call.
    let accepted = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    MethodEvidence::from_transport(
        InsertionMethod::VirtualKey,
        inputs.len() as u32,
        accepted,
        false,
        false,
    )
}

fn clipboard_paste(text: &str, owner: HWND, force_restore_failure: bool) -> MethodEvidence {
    // SAFETY: owner is a live fixture HWND. No retry loop is intentional.
    if unsafe { OpenClipboard(owner) } == 0 {
        return MethodEvidence::failed(InsertionMethod::ClipboardPaste, FailureCode::ClipboardBusy);
    }
    let previous = read_clipboard_unicode();
    let Ok(previous) = previous else {
        // SAFETY: this function owns the open clipboard.
        unsafe { CloseClipboard() };
        return MethodEvidence::failed(
            InsertionMethod::ClipboardPaste,
            FailureCode::ClipboardRestoreFailed,
        );
    };
    let set_result = set_clipboard_unicode(text);
    // SAFETY: this function owns the open clipboard.
    unsafe { CloseClipboard() };
    if set_result.is_err() {
        return MethodEvidence::failed(
            InsertionMethod::ClipboardPaste,
            FailureCode::Win32CallFailed,
        );
    }

    let paste_inputs = [
        key_input(VK_CONTROL, 0, 0),
        key_input(b'V' as u16, 0, 0),
        key_input(b'V' as u16, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];
    // SAFETY: input slice is valid for the duration of the call.
    let accepted = unsafe {
        SendInput(
            paste_inputs.len() as u32,
            paste_inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    pump_messages();

    if force_restore_failure || unsafe { OpenClipboard(owner) } == 0 {
        return MethodEvidence::failed(
            InsertionMethod::ClipboardPaste,
            FailureCode::ClipboardRestoreFailed,
        );
    }
    let restored = match previous {
        Some(previous) => set_clipboard_unicode(&previous).is_ok(),
        None => (unsafe { EmptyClipboard() }) != 0,
    };
    // SAFETY: this function owns the open clipboard.
    unsafe { CloseClipboard() };
    if !restored {
        return MethodEvidence::failed(
            InsertionMethod::ClipboardPaste,
            FailureCode::ClipboardRestoreFailed,
        );
    }

    MethodEvidence::from_transport(
        InsertionMethod::ClipboardPaste,
        paste_inputs.len() as u32,
        accepted,
        false,
        false,
    )
}

fn read_clipboard_unicode() -> Result<Option<String>, ()> {
    // This safe spike refuses to flatten non-text clipboard formats. A production fallback must
    // use a full format-preserving strategy or fail closed in the same way.
    let format_count = unsafe { CountClipboardFormats() };
    if format_count == 0 {
        return Ok(None);
    }
    if format_count != 1 || unsafe { IsClipboardFormatAvailable(13) } == 0 {
        return Err(());
    }
    // SAFETY: clipboard is open and CF_UNICODETEXT is available.
    let handle = unsafe { GetClipboardData(13) };
    if handle.is_null() {
        return Err(());
    }
    let size = unsafe { GlobalSize(handle) } / size_of::<u16>();
    let data = unsafe { GlobalLock(handle) } as *const u16;
    if data.is_null() {
        return Err(());
    }
    let slice = unsafe { std::slice::from_raw_parts(data, size) };
    let length = slice
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(slice.len());
    let value = String::from_utf16_lossy(&slice[..length]);
    unsafe { GlobalUnlock(handle) };
    Ok(Some(value))
}

fn set_clipboard_unicode(text: &str) -> Result<(), ()> {
    let mut encoded: Vec<u16> = text.encode_utf16().collect();
    encoded.push(0);
    // SAFETY: clipboard is open and allocation size is non-zero.
    let allocation = unsafe { GlobalAlloc(GMEM_MOVEABLE, encoded.len() * size_of::<u16>()) };
    if allocation.is_null() {
        return Err(());
    }
    let destination = unsafe { GlobalLock(allocation) } as *mut u16;
    if destination.is_null() {
        unsafe { GlobalFree(allocation) };
        return Err(());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(encoded.as_ptr(), destination, encoded.len());
        GlobalUnlock(allocation);
    }
    if unsafe { EmptyClipboard() } == 0 {
        unsafe { GlobalFree(allocation) };
        return Err(());
    }
    // Ownership transfers to the system only on success.
    if unsafe { SetClipboardData(13, allocation) }.is_null() {
        unsafe { GlobalFree(allocation) };
        return Err(());
    }
    Ok(())
}

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn pump_messages() {
    for _ in 0..20 {
        let mut had_message = false;
        loop {
            // SAFETY: msg is writable and null HWND selects current-thread messages.
            let mut msg: MSG = unsafe { zeroed() };
            if unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } == 0 {
                break;
            }
            had_message = true;
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !had_message {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(process_id: u32) -> Result<Self, String> {
        // SAFETY: process_id came from GetWindowThreadProcessId.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if handle.is_null() {
            Err(last_error("OpenProcess"))
        } else {
            Ok(Self(handle))
        }
    }

    fn name(&self) -> Result<String, String> {
        let mut path = vec![0_u16; 32_768];
        let mut length = path.len() as u32;
        // SAFETY: process handle is live and path buffer is writable.
        if unsafe { QueryFullProcessImageNameW(self.0, 0, path.as_mut_ptr(), &mut length) } == 0 {
            return Err(last_error("QueryFullProcessImageNameW"));
        }
        let full = String::from_utf16_lossy(&path[..length as usize]);
        Ok(std::path::Path::new(&full)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_owned())
    }

    fn integrity_rid(&self) -> Result<u32, String> {
        let mut token = null_mut();
        // SAFETY: process handle is live and token pointer is writable.
        if unsafe { OpenProcessToken(self.0, TOKEN_QUERY, &mut token) } == 0 {
            return Err(last_error("OpenProcessToken"));
        }
        let token = OwnedHandle(token);
        token_integrity_rid(token.0)
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn current_process_integrity() -> Result<u32, String> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("OpenProcessToken(current)"));
    }
    let token = OwnedHandle(token);
    token_integrity_rid(token.0)
}

fn token_integrity_rid(token: HANDLE) -> Result<u32, String> {
    let mut size = 0_u32;
    unsafe {
        GetTokenInformation(token, TokenIntegrityLevel, null_mut(), 0, &mut size);
    }
    if size == 0 {
        return Err(last_error("GetTokenInformation(size)"));
    }
    let mut buffer = vec![0_u8; size as usize];
    if unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast::<c_void>(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(last_error("GetTokenInformation(value)"));
    }
    let label = unsafe { &*(buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    let count_ptr = unsafe { GetSidSubAuthorityCount(label.Label.Sid) };
    if count_ptr.is_null() {
        return Err(last_error("GetSidSubAuthorityCount"));
    }
    let count = unsafe { *count_ptr };
    if count == 0 {
        return Err("integrity SID has no sub-authorities".into());
    }
    let rid_ptr = unsafe { GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)) };
    if rid_ptr.is_null() {
        return Err(last_error("GetSidSubAuthority"));
    }
    Ok(unsafe { *rid_ptr })
}

fn environment_evidence() -> EnvironmentEvidence {
    // SAFETY: RtlGetVersion writes the initialized, correctly-sized structure.
    let mut version: OSVERSIONINFOW = unsafe { zeroed() };
    version.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    let status = unsafe { RtlGetVersion(&mut version) };
    if status != 0 {
        return EnvironmentEvidence {
            os_family: "Windows (version unavailable)".into(),
            major: 0,
            minor: 0,
            build: 0,
            architecture: std::env::consts::ARCH.into(),
        };
    }
    EnvironmentEvidence {
        os_family: if version.dwBuildNumber >= 22_000 {
            "Windows 11"
        } else {
            "Windows 10"
        }
        .into(),
        major: version.dwMajorVersion,
        minor: version.dwMinorVersion,
        build: version.dwBuildNumber,
        architecture: std::env::consts::ARCH.into(),
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error(operation: &str) -> String {
    format!("{operation} failed: {}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop and explicit fixture focus"]
    fn automated_matrix_passes_on_windows() {
        let report = run_automated_matrix().expect("Win32 matrix should run");
        assert!(report.all_required_passed(), "{:#?}", report.checks);
        assert!(!report.content_logged);
    }
}
