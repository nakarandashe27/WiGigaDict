use crate::insertion::{InsertionFailure, InsertionPlatform, TransportAttempt};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::ptr::null_mut;
use std::thread;
use std::time::Duration;
use wigigadict_storage::{DeliveryMethod, IntegrityLevel, TargetSnapshot, TargetSnapshotInput};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GlobalFree, HANDLE, HWND},
    Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    },
    System::{
        DataExchange::{
            CloseClipboard, CountClipboardFormats, EmptyClipboard, GetClipboardData,
            IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        SystemInformation::OSVERSIONINFOW,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, SendInput, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
            VK_SPACE,
        },
        WindowsAndMessaging::{
            GUITHREADINFO, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
            GetWindowThreadProcessId, IsWindow,
        },
    },
};

const MEDIUM_INTEGRITY_RID: u32 = 0x2000;
const CF_UNICODETEXT: u32 = 13;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeTarget {
    hwnd: HWND,
    process_id: u32,
    thread_id: u32,
    process_identity: String,
    window_class: String,
    control_class: String,
    integrity_rid: u32,
    os_build: u32,
}

pub struct WindowsInsertionPlatform;

pub fn capture_target(
    snapshot_id: String,
    captured_at: i64,
) -> Result<TargetSnapshotInput, InsertionFailure> {
    let native = capture_native()?;
    Ok(TargetSnapshotInput {
        snapshot_id,
        process_identity: native.process_identity,
        process_id: native.process_id,
        thread_id: native.thread_id,
        window_handle: format!("0x{:016X}", native.hwnd as usize as u64),
        window_class: native.window_class,
        control_class: native.control_class,
        process_version: "unknown".into(),
        integrity_level: integrity_level(native.integrity_rid),
        integrity_rid: native.integrity_rid,
        os_build: native.os_build,
        captured_at,
    })
}

impl InsertionPlatform for WindowsInsertionPlatform {
    fn revalidate(&mut self, target: &TargetSnapshot) -> Result<String, InsertionFailure> {
        let expected_hwnd = parse_hwnd(&target.window_handle)?;
        if expected_hwnd.is_null() || unsafe { IsWindow(expected_hwnd) } == 0 {
            return Err(InsertionFailure::TargetMissing);
        }
        if unsafe { GetForegroundWindow() } != expected_hwnd {
            return Err(InsertionFailure::FocusChanged);
        }
        let current = capture_native()?;
        if current.hwnd != expected_hwnd
            || current.process_id != target.process_id
            || current.thread_id != target.thread_id
            || !current
                .process_identity
                .eq_ignore_ascii_case(&target.process_identity)
            || current.window_class != target.window_class
            || current.control_class != target.control_class
            || current.integrity_rid != target.integrity_rid
            || current.os_build != target.os_build
        {
            return Err(InsertionFailure::FocusChanged);
        }
        if current.os_build == 0 || current.os_build >= 22_000 {
            return Err(InsertionFailure::UnsupportedWindowsVersion);
        }
        let caller_integrity =
            current_process_integrity().map_err(|_| InsertionFailure::Win32CallFailed)?;
        if current.integrity_rid > caller_integrity || current.integrity_rid > MEDIUM_INTEGRITY_RID
        {
            return Err(InsertionFailure::ElevatedTarget);
        }
        Ok(target.window_handle.clone())
    }

    fn insert(
        &mut self,
        method: DeliveryMethod,
        text: &str,
        target: &TargetSnapshot,
    ) -> TransportAttempt {
        match method {
            DeliveryMethod::Unicode => send_unicode(text),
            DeliveryMethod::SendInput => send_virtual_keys(text),
            DeliveryMethod::Clipboard => clipboard_paste(text, target),
        }
    }
}

fn capture_native() -> Result<NativeTarget, InsertionFailure> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
        return Err(InsertionFailure::TargetMissing);
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    if thread_id == 0 || process_id == 0 {
        return Err(InsertionFailure::Win32CallFailed);
    }
    let process = ProcessHandle::open(process_id).map_err(|_| InsertionFailure::TargetMissing)?;
    let process_identity = process
        .name()
        .map_err(|_| InsertionFailure::Win32CallFailed)?;
    let integrity_rid = process
        .integrity_rid()
        .map_err(|_| InsertionFailure::Win32CallFailed)?;
    Ok(NativeTarget {
        hwnd,
        process_id,
        thread_id,
        process_identity,
        window_class: class_name(hwnd)?,
        control_class: focused_control_class(thread_id)?,
        integrity_rid,
        os_build: os_build(),
    })
}

fn class_name(hwnd: HWND) -> Result<String, InsertionFailure> {
    let mut class = vec![0_u16; 256];
    let length = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    if length == 0 {
        return Err(InsertionFailure::Win32CallFailed);
    }
    Ok(String::from_utf16_lossy(&class[..length as usize]))
}

fn focused_control_class(thread_id: u32) -> Result<String, InsertionFailure> {
    let mut info: GUITHREADINFO = unsafe { zeroed() };
    info.cbSize = size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(thread_id, &mut info) } == 0 {
        return Err(InsertionFailure::Win32CallFailed);
    }
    if info.hwndFocus.is_null() {
        return Ok("none".into());
    }
    class_name(info.hwndFocus)
}

fn keyboard_state_safe() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .all(|key| unsafe { GetAsyncKeyState(i32::from(*key)) } >= 0)
}

fn send_unicode(text: &str) -> TransportAttempt {
    if !keyboard_state_safe() {
        let mut result = TransportAttempt::zero(InsertionFailure::KeyboardStateUnsafe);
        result.keyboard_state_safe = Some(false);
        return result;
    }
    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    send_inputs(inputs, Some(true), None, None)
}

fn send_virtual_keys(text: &str) -> TransportAttempt {
    if !keyboard_state_safe() {
        let mut result = TransportAttempt::zero(InsertionFailure::KeyboardStateUnsafe);
        result.keyboard_state_safe = Some(false);
        return result;
    }
    let Some(inputs) = virtual_key_inputs(text) else {
        let mut result = TransportAttempt::zero(InsertionFailure::UnsupportedCharacter);
        result.keyboard_state_safe = Some(true);
        return result;
    };
    send_inputs(inputs, Some(true), None, None)
}

fn virtual_key_inputs(text: &str) -> Option<Vec<INPUT>> {
    let mut inputs = Vec::new();
    for character in text.chars() {
        let (key, shift) = match character {
            'a'..='z' => (character.to_ascii_uppercase() as u16, false),
            'A'..='Z' => (character as u16, true),
            '0'..='9' => (character as u16, false),
            ' ' => (VK_SPACE, false),
            _ => return None,
        };
        if shift {
            inputs.push(key_input(VK_SHIFT, 0, 0));
        }
        inputs.push(key_input(key, 0, 0));
        inputs.push(key_input(key, 0, KEYEVENTF_KEYUP));
        if shift {
            inputs.push(key_input(VK_SHIFT, 0, KEYEVENTF_KEYUP));
        }
    }
    Some(inputs)
}

fn send_inputs(
    inputs: Vec<INPUT>,
    keyboard_state_safe: Option<bool>,
    clipboard_set: Option<bool>,
    clipboard_restored: Option<bool>,
) -> TransportAttempt {
    let expected = inputs.len() as u32;
    let accepted = unsafe { SendInput(expected, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    TransportAttempt {
        expected_units: expected,
        accepted_units: accepted,
        target_acknowledged: false,
        keyboard_state_safe,
        clipboard_set,
        clipboard_restored,
        failure: transport_failure(expected, accepted),
    }
}

fn clipboard_paste(text: &str, _target: &TargetSnapshot) -> TransportAttempt {
    if !keyboard_state_safe() {
        let mut result = TransportAttempt::zero(InsertionFailure::KeyboardStateUnsafe);
        result.keyboard_state_safe = Some(false);
        result.clipboard_set = Some(false);
        return result;
    }
    let owner = null_mut();
    if unsafe { OpenClipboard(owner) } == 0 {
        let mut result = TransportAttempt::zero(InsertionFailure::ClipboardBusy);
        result.keyboard_state_safe = Some(true);
        result.clipboard_set = Some(false);
        return result;
    }
    let previous = match read_clipboard_unicode() {
        Ok(value) => value,
        Err(()) => {
            unsafe { CloseClipboard() };
            let mut result = TransportAttempt::zero(InsertionFailure::ClipboardRestoreFailed);
            result.keyboard_state_safe = Some(true);
            result.clipboard_set = Some(false);
            result.clipboard_restored = Some(false);
            return result;
        }
    };
    if set_clipboard_unicode(text).is_err() {
        let restored = restore_clipboard(&previous);
        unsafe { CloseClipboard() };
        let mut result = TransportAttempt::zero(if restored {
            InsertionFailure::Win32CallFailed
        } else {
            InsertionFailure::ClipboardRestoreFailed
        });
        result.keyboard_state_safe = Some(true);
        result.clipboard_set = Some(false);
        result.clipboard_restored = Some(restored);
        return result;
    }
    unsafe { CloseClipboard() };

    let inputs = vec![
        key_input(VK_CONTROL, 0, 0),
        key_input(b'V' as u16, 0, 0),
        key_input(b'V' as u16, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];
    let mut attempt = send_inputs(inputs, Some(true), Some(true), None);
    thread::sleep(Duration::from_millis(100));
    let restored = if unsafe { OpenClipboard(owner) } == 0 {
        false
    } else {
        let restored = restore_clipboard(&previous);
        unsafe { CloseClipboard() };
        restored
    };
    attempt.clipboard_restored = Some(restored);
    if !restored {
        attempt.failure = Some(InsertionFailure::ClipboardRestoreFailed);
    }
    attempt
}

enum ClipboardContent {
    Empty,
    Unicode(String),
}

fn read_clipboard_unicode() -> Result<ClipboardContent, ()> {
    let count = unsafe { CountClipboardFormats() };
    if count == 0 {
        return Ok(ClipboardContent::Empty);
    }
    if count != 1 || unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) } == 0 {
        return Err(());
    }
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if handle.is_null() {
        return Err(());
    }
    let units = unsafe { GlobalSize(handle) } / size_of::<u16>();
    let data = unsafe { GlobalLock(handle) } as *const u16;
    if data.is_null() {
        return Err(());
    }
    let slice = unsafe { std::slice::from_raw_parts(data, units) };
    let length = slice
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(slice.len());
    let value = String::from_utf16_lossy(&slice[..length]);
    unsafe { GlobalUnlock(handle) };
    Ok(ClipboardContent::Unicode(value))
}

fn set_clipboard_unicode(text: &str) -> Result<(), ()> {
    let mut encoded: Vec<u16> = text.encode_utf16().collect();
    encoded.push(0);
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
    if unsafe { SetClipboardData(CF_UNICODETEXT, allocation) }.is_null() {
        unsafe { GlobalFree(allocation) };
        return Err(());
    }
    Ok(())
}

fn restore_clipboard(previous: &ClipboardContent) -> bool {
    match previous {
        ClipboardContent::Empty => (unsafe { EmptyClipboard() }) != 0,
        ClipboardContent::Unicode(value) => set_clipboard_unicode(value).is_ok(),
    }
}

fn transport_failure(expected: u32, accepted: u32) -> Option<InsertionFailure> {
    if accepted == expected {
        None
    } else if accepted == 0 {
        Some(InsertionFailure::Win32CallFailed)
    } else {
        Some(InsertionFailure::InputPartiallyAccepted)
    }
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

fn parse_hwnd(value: &str) -> Result<HWND, InsertionFailure> {
    let raw = value
        .strip_prefix("0x")
        .and_then(|value| usize::from_str_radix(value, 16).ok())
        .ok_or(InsertionFailure::TargetMissing)?;
    Ok(raw as HWND)
}

fn integrity_level(rid: u32) -> IntegrityLevel {
    match rid {
        0x0000..=0x0fff => IntegrityLevel::Untrusted,
        0x1000..=0x1fff => IntegrityLevel::Low,
        0x2000..=0x2fff => IntegrityLevel::Medium,
        0x3000..=0x3fff => IntegrityLevel::High,
        0x4000.. => IntegrityLevel::System,
    }
}

fn os_build() -> u32 {
    let mut version: OSVERSIONINFOW = unsafe { zeroed() };
    version.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    if unsafe { RtlGetVersion(&mut version) } == 0 {
        version.dwBuildNumber
    } else {
        0
    }
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(process_id: u32) -> Result<Self, ()> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if handle.is_null() {
            Err(())
        } else {
            Ok(Self(handle))
        }
    }

    fn name(&self) -> Result<String, ()> {
        let mut path = vec![0_u16; 32_768];
        let mut length = path.len() as u32;
        if unsafe { QueryFullProcessImageNameW(self.0, 0, path.as_mut_ptr(), &mut length) } == 0 {
            return Err(());
        }
        let full = String::from_utf16_lossy(&path[..length as usize]);
        Ok(Path::new(&full)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_owned())
    }

    fn integrity_rid(&self) -> Result<u32, ()> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(self.0, TOKEN_QUERY, &mut token) } == 0 {
            return Err(());
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

fn current_process_integrity() -> Result<u32, ()> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(());
    }
    let token = OwnedHandle(token);
    token_integrity_rid(token.0)
}

fn token_integrity_rid(token: HANDLE) -> Result<u32, ()> {
    let mut size = 0_u32;
    unsafe {
        GetTokenInformation(token, TokenIntegrityLevel, null_mut(), 0, &mut size);
    }
    if size == 0 {
        return Err(());
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
        return Err(());
    }
    let label = unsafe { &*(buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    let count_ptr = unsafe { GetSidSubAuthorityCount(label.Label.Sid) };
    if count_ptr.is_null() {
        return Err(());
    }
    let count = unsafe { *count_ptr };
    if count == 0 {
        return Err(());
    }
    let rid_ptr = unsafe { GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)) };
    if rid_ptr.is_null() {
        return Err(());
    }
    Ok(unsafe { *rid_ptr })
}
