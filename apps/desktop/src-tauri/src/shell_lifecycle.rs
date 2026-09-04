//! Windows-only safety boundary for the long-lived Tauri shell.

use std::collections::hash_map::DefaultHasher;
use std::ffi::c_void;
use std::fs;
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr::{NonNull, null_mut};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use uuid::Uuid;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HANDLE, HLOCAL, HWND, LPARAM,
    LRESULT, WPARAM,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, GetAce,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SetFileSecurityW,
    TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::System::SystemServices::{
    ACCESS_ALLOWED_ACE_TYPE, ACCESS_ALLOWED_CALLBACK_ACE_TYPE,
    ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE, ACCESS_ALLOWED_OBJECT_ACE_TYPE,
};
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, OpenMutexW, OpenProcessToken, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, FOLDERID_LocalAppData, KF_FLAG_DEFAULT, RemoveWindowSubclass,
    SHGetKnownFolderPath, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, PostMessageW, RegisterWindowMessageW, WM_ENDSESSION, WM_NCDESTROY,
    WM_QUERYENDSESSION, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, WTS_SESSION_LOGOFF,
    WTS_SESSION_UNLOCK,
};
use windows::core::{PCWSTR, PWSTR};

const PRODUCT_DIRECTORY: &str = "WiGigaDict";
/// Broadcast message a second launch uses to ask the live instance to show its main window.
const ACTIVATION_MESSAGE: PCWSTR = windows::core::w!("WiGigaDict.ActivateMainWindow");
/// Exists only once the live shell can answer [`ACTIVATION_MESSAGE`]. Session-scoped like the
/// broadcast itself, so it needs no managed-root plumbing.
const ACTIVATION_READY_OBJECT: PCWSTR = windows::core::w!(r"Local\WiGigaDict.ActivationReady");
const ACTIVATION_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const ACTIVATION_READY_POLL: std::time::Duration = std::time::Duration::from_millis(50);
const SESSION_SUBCLASS_ID: usize = 0x5749_4749;
const MAIN_WINDOW_LABEL: &str = "main";
const OVERLAY_WINDOW_LABEL: &str = "overlay";

#[derive(Debug, thiserror::Error)]
pub enum ShellLifecycleError {
    #[error("WiGigaDict must run without elevation")]
    Elevated,
    #[error("another WiGigaDict shell already owns this user session")]
    AlreadyRunning,
    #[error("managed root must stay below the current user's LocalAppData")]
    OutsideManagedRoot,
    #[error("managed path contains a reparse point: {0}")]
    ReparsePoint(PathBuf),
    #[error("managed root DACL is missing or null")]
    MissingDacl,
    #[error("managed root grants access outside owner/SYSTEM/Administrators")]
    BroadAcl,
    #[error("shell is not accepting a new capture")]
    CaptureRejected,
    #[error("invalid caller window: {0}")]
    InvalidCaller(String),
    #[error("lifecycle state mutex was poisoned")]
    Poisoned,
    #[error("{operation} failed with Win32 error {code}")]
    Win32Code { operation: &'static str, code: u32 },
    #[error("{operation} failed: {source}")]
    Win32 {
        operation: &'static str,
        #[source]
        source: windows::core::Error,
    },
    #[error("{operation} failed: {source}")]
    Utf16 {
        operation: &'static str,
        #[source]
        source: std::string::FromUtf16Error,
    },
    #[error("{operation} failed: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
}

type ShellResult<T> = Result<T, ShellLifecycleError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellPhase {
    Ready,
    SessionLocked,
    ShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEvent {
    SessionLock,
    SessionUnlock,
    SessionLogoff,
    QueryEndSession,
    EndSession,
    /// `WM_ENDSESSION` with `wParam = FALSE`: the shutdown Windows asked about was abandoned.
    EndSessionCancelled,
    AppExit,
}

impl ShellEvent {
    fn recovery_reason(self) -> &'static str {
        match self {
            Self::SessionLock => "windows_session_locked",
            Self::SessionLogoff => "windows_session_logoff",
            Self::QueryEndSession => "windows_query_end_session",
            Self::EndSession => "windows_end_session",
            Self::AppExit => "application_exit",
            Self::SessionUnlock => "windows_session_unlocked",
            Self::EndSessionCancelled => "windows_end_session_cancelled",
        }
    }

    /// The session continues after this event, so the shell must become reachable again.
    pub fn resumes_session(self) -> bool {
        matches!(self, Self::SessionUnlock | Self::EndSessionCancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CaptureState {
    Idle,
    Recording {
        session_id: String,
    },
    Recovery {
        session_id: String,
        reason: &'static str,
    },
}

#[derive(Debug)]
struct LifecycleState {
    phase: ShellPhase,
    accepting_new_work: bool,
    capture: CaptureState,
    /// Set by `WM_QUERYENDSESSION` and cleared by the answer. Only a shutdown that Windows asked
    /// about may be taken back; `EndSession`, `SessionLogoff` and `AppExit` stay terminal.
    end_session_query_pending: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellStatus {
    generation_id: String,
    phase: ShellPhase,
    accepting_new_work: bool,
    active_capture_state: &'static str,
    recovery_reason: Option<&'static str>,
    managed_root_ready: bool,
    elevated: bool,
}

#[derive(Debug)]
pub struct ShellLifecycle {
    generation_id: String,
    managed_paths: ManagedPaths,
    state: Mutex<LifecycleState>,
}

impl ShellLifecycle {
    fn new(managed_paths: ManagedPaths) -> Self {
        Self {
            generation_id: Uuid::new_v4().to_string(),
            managed_paths,
            state: Mutex::new(LifecycleState {
                phase: ShellPhase::Ready,
                accepting_new_work: true,
                capture: CaptureState::Idle,
                end_session_query_pending: false,
            }),
        }
    }

    pub fn status(&self) -> ShellResult<ShellStatus> {
        let state = self
            .state
            .lock()
            .map_err(|_| ShellLifecycleError::Poisoned)?;
        let (active_capture_state, recovery_reason) = match &state.capture {
            CaptureState::Idle => ("idle", None),
            CaptureState::Recording { .. } => ("recording", None),
            CaptureState::Recovery { reason, .. } => ("recovery", Some(*reason)),
        };
        Ok(ShellStatus {
            generation_id: self.generation_id.clone(),
            phase: state.phase,
            accepting_new_work: state.accepting_new_work,
            active_capture_state,
            recovery_reason,
            managed_root_ready: self.managed_paths.root.is_dir(),
            elevated: false,
        })
    }

    pub fn on_event(&self, event: ShellEvent) -> ShellResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ShellLifecycleError::Poisoned)?;
        if event == ShellEvent::SessionUnlock {
            if state.phase == ShellPhase::SessionLocked {
                state.phase = ShellPhase::Ready;
                state.accepting_new_work = true;
            }
            return Ok(());
        }
        if event == ShellEvent::EndSessionCancelled {
            // A cancelled shutdown used to leave the shell in ShuttingDown forever: the window
            // could never be shown again, every hotkey press was rejected and the process stayed
            // alive as an invisible instance holding the mutex.
            if state.end_session_query_pending {
                state.end_session_query_pending = false;
                state.phase = ShellPhase::Ready;
                state.accepting_new_work = true;
            }
            return Ok(());
        }

        state.end_session_query_pending = event == ShellEvent::QueryEndSession;
        state.accepting_new_work = false;
        state.phase = if event == ShellEvent::SessionLock {
            ShellPhase::SessionLocked
        } else {
            ShellPhase::ShuttingDown
        };
        if let CaptureState::Recording { session_id } = &state.capture {
            state.capture = CaptureState::Recovery {
                session_id: session_id.clone(),
                reason: event.recovery_reason(),
            };
        }
        Ok(())
    }

    pub fn is_shutting_down(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.phase == ShellPhase::ShuttingDown)
            .unwrap_or(true)
    }

    pub fn capture_paths(&self) -> CapturePaths {
        CapturePaths {
            database: self
                .managed_paths
                .root
                .join("storage")
                .join("wigigadict.sqlite3"),
            audio_root: self.managed_paths.root.clone(),
        }
    }

    pub fn begin_capture(&self, session_id: &str) -> ShellResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ShellLifecycleError::Poisoned)?;
        if !state.accepting_new_work || state.capture != CaptureState::Idle {
            return Err(ShellLifecycleError::CaptureRejected);
        }
        state.capture = CaptureState::Recording {
            session_id: session_id.to_owned(),
        };
        Ok(())
    }
    pub fn finish_capture(&self, session_id: &str) {
        if let Ok(mut state) = self.state.lock()
            && matches!(
                &state.capture,
                CaptureState::Recording { session_id: active } if active == session_id
            )
        {
            state.capture = CaptureState::Idle;
        }
    }

    pub fn finish_capture_recovery(&self, session_id: &str) {
        if let Ok(mut state) = self.state.lock()
            && matches!(
                &state.capture,
                CaptureState::Recovery { session_id: active, .. } if active == session_id
            )
        {
            state.capture = CaptureState::Idle;
        }
    }

    pub fn recover_capture(&self, session_id: &str, reason: &'static str) {
        if let Ok(mut state) = self.state.lock() {
            match &state.capture {
                CaptureState::Recording { session_id: active }
                | CaptureState::Recovery {
                    session_id: active, ..
                } if active == session_id => {
                    state.capture = CaptureState::Recovery {
                        session_id: session_id.to_owned(),
                        reason,
                    };
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapturePaths {
    pub database: PathBuf,
    pub audio_root: PathBuf,
}

#[derive(Debug)]
struct ManagedPaths {
    root: PathBuf,
}

impl ManagedPaths {
    fn prepare() -> ShellResult<Self> {
        let local_app_data = local_app_data()?;
        let root = local_app_data.join(PRODUCT_DIRECTORY);
        validate_managed_root(&local_app_data, &root)?;
        reject_reparse_if_present(&root)?;
        fs::create_dir_all(&root).map_err(|source| ShellLifecycleError::Io {
            operation: "create managed root",
            source,
        })?;

        let current_user_sid = current_user_sid_string()?;
        harden_managed_acl(&root, &current_user_sid)?;

        let canonical_base =
            local_app_data
                .canonicalize()
                .map_err(|source| ShellLifecycleError::Io {
                    operation: "canonicalize LocalAppData",
                    source,
                })?;
        let canonical_root = root
            .canonicalize()
            .map_err(|source| ShellLifecycleError::Io {
                operation: "canonicalize managed root",
                source,
            })?;
        validate_managed_root(&canonical_base, &canonical_root)?;
        reject_reparse_if_present(&canonical_root)?;
        ensure_restricted_acl(&canonical_root, &current_user_sid)?;

        // `installed`/`staging` hold downloaded third-party weights. They already inherit the
        // protected root DACL, so this adds no permission - it asserts on every start that
        // nobody re-broke inheritance and widened them. ModelManager creates them lazily and
        // never inspects ACLs.
        for directory in [
            "storage",
            "audio",
            "logs",
            "quarantine",
            "installed",
            "staging",
        ] {
            let path = canonical_root.join(directory);
            reject_reparse_if_present(&path)?;
            fs::create_dir_all(&path).map_err(|source| ShellLifecycleError::Io {
                operation: "create managed subdirectory",
                source,
            })?;
            reject_reparse_if_present(&path)?;
            ensure_restricted_acl(&path, &current_user_sid)?;
        }

        Ok(Self {
            root: canonical_root,
        })
    }
}

fn validate_managed_root(local_app_data: &Path, root: &Path) -> ShellResult<()> {
    let relative = root
        .strip_prefix(local_app_data)
        .map_err(|_| ShellLifecycleError::OutsideManagedRoot)?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(ShellLifecycleError::OutsideManagedRoot);
    }
    Ok(())
}

fn reject_reparse_if_present(path: &Path) -> ShellResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ShellLifecycleError::Io {
                operation: "read managed path metadata",
                source,
            });
        }
    };
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(ShellLifecycleError::ReparsePoint(path.to_owned()));
    }
    Ok(())
}

fn local_app_data() -> ShellResult<PathBuf> {
    // SAFETY: SHGetKnownFolderPath allocates the returned null-terminated string. It is copied
    // before the matching CoTaskMemFree call below.
    let value = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }
        .map_err(|source| ShellLifecycleError::Win32 {
            operation: "resolve LocalAppData",
            source,
        })?;
    // SAFETY: value is the valid PWSTR returned by SHGetKnownFolderPath.
    let result = unsafe { value.to_string() };
    // SAFETY: SHGetKnownFolderPath documents CoTaskMemFree for this allocation.
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    result
        .map(PathBuf::from)
        .map_err(|source| ShellLifecycleError::Utf16 {
            operation: "decode LocalAppData",
            source,
        })
}

struct LocalSecurityDescriptor(NonNull<c_void>);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: GetNamedSecurityInfoW allocated this descriptor with LocalAlloc.
        unsafe {
            let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(self.0.as_ptr())));
        }
    }
}

fn current_user_sid_string() -> ShellResult<String> {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle; token points to writable storage.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.map_err(
        |source| ShellLifecycleError::Win32 {
            operation: "open process token for SID",
            source,
        },
    )?;
    let _token_guard = TokenHandle(token);
    let mut required = 0_u32;
    // SAFETY: a zero-size probe is the documented way to obtain the TOKEN_USER buffer size.
    let _ = unsafe {
        windows::Win32::Security::GetTokenInformation(token, TokenUser, None, 0, &mut required)
    };
    if required == 0 {
        return Err(ShellLifecycleError::Win32 {
            operation: "size current user SID",
            source: windows::core::Error::from_win32(),
        });
    }
    let mut buffer = vec![0_u8; required as usize];
    // SAFETY: buffer has exactly the byte length reported by GetTokenInformation.
    unsafe {
        windows::Win32::Security::GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            required,
            &mut required,
        )
    }
    .map_err(|source| ShellLifecycleError::Win32 {
        operation: "read current user SID",
        source,
    })?;
    // SAFETY: a successful TokenUser query starts with a valid TOKEN_USER structure.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    sid_to_string(user.User.Sid)
}

fn sid_to_string(sid: PSID) -> ShellResult<String> {
    let mut value = PWSTR::null();
    // SAFETY: sid comes from a live token or ACE and value is a valid output pointer.
    unsafe { ConvertSidToStringSidW(sid, &mut value) }.map_err(|source| {
        ShellLifecycleError::Win32 {
            operation: "format Windows SID",
            source,
        }
    })?;
    // SAFETY: ConvertSidToStringSidW returned a valid null-terminated string.
    let result = unsafe { value.to_string() };
    // SAFETY: ConvertSidToStringSidW allocates with LocalAlloc.
    unsafe {
        let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(value.0.cast())));
    }
    result.map_err(|source| ShellLifecycleError::Utf16 {
        operation: "decode Windows SID",
        source,
    })
}

fn harden_managed_acl(path: &Path, current_user_sid: &str) -> ShellResult<()> {
    let sddl = format!("D:P(A;OICI;FA;;;{current_user_sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
    let sddl = wide(sddl.as_ref());
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: SDDL is null-terminated and descriptor is a valid output pointer.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|source| ShellLifecycleError::Win32 {
        operation: "build managed root DACL",
        source,
    })?;
    let descriptor_pointer = NonNull::new(descriptor.0).ok_or(ShellLifecycleError::MissingDacl)?;
    let _descriptor_guard = LocalSecurityDescriptor(descriptor_pointer);
    let path = wide(path.as_os_str());
    // SAFETY: path and descriptor remain valid for the duration of SetFileSecurityW.
    unsafe {
        SetFileSecurityW(
            PCWSTR(path.as_ptr()),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    }
    .ok()
    .map_err(|source| ShellLifecycleError::Win32 {
        operation: "apply managed root DACL",
        source,
    })
}

fn is_allowed_managed_sid(sid: &str, current_user_sid: &str) -> bool {
    sid == current_user_sid || sid == "S-1-5-18" || sid == "S-1-5-32-544"
}

fn is_unexpected_allow_ace_type(ace_type: u8) -> bool {
    matches!(
        u32::from(ace_type),
        ACCESS_ALLOWED_OBJECT_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE
    )
}

fn ensure_restricted_acl(path: &Path, current_user_sid: &str) -> ShellResult<()> {
    let wide = wide(path.as_os_str());
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: output pointers are valid for the duration of the call; path is null-terminated.
    let code = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            &mut descriptor,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(ShellLifecycleError::Win32Code {
            operation: "read managed root DACL",
            code: code.0,
        });
    }
    let descriptor = NonNull::new(descriptor.0).ok_or(ShellLifecycleError::MissingDacl)?;
    let _descriptor_guard = LocalSecurityDescriptor(descriptor);
    let dacl = NonNull::new(dacl).ok_or(ShellLifecycleError::MissingDacl)?;

    // SAFETY: dacl points inside the live security descriptor guarded above.
    let ace_count = unsafe { dacl.as_ref().AceCount };
    for index in 0..u32::from(ace_count) {
        let mut raw_ace: *mut c_void = null_mut();
        // SAFETY: index is bounded by AceCount and raw_ace is a valid output pointer.
        unsafe { GetAce(dacl.as_ptr(), index, &mut raw_ace) }.map_err(|source| {
            ShellLifecycleError::Win32 {
                operation: "read managed root ACE",
                source,
            }
        })?;
        let ace = NonNull::new(raw_ace.cast::<ACCESS_ALLOWED_ACE>())
            .ok_or(ShellLifecycleError::MissingDacl)?;
        // SAFETY: every ACE starts with ACE_HEADER; ACCESS_ALLOWED_ACE is used only for its type.
        let ace_type = unsafe { ace.as_ref().Header.AceType };
        if ace_type != ACCESS_ALLOWED_ACE_TYPE as u8 {
            if is_unexpected_allow_ace_type(ace_type) {
                return Err(ShellLifecycleError::BroadAcl);
            }
            continue;
        }
        // SAFETY: SidStart is the first byte of the variable-length SID in ACCESS_ALLOWED_ACE.
        let sid =
            unsafe { PSID(std::ptr::addr_of_mut!((*ace.as_ptr()).SidStart).cast::<c_void>()) };
        let sid = sid_to_string(sid)?;
        if !is_allowed_managed_sid(&sid, current_user_sid) {
            return Err(ShellLifecycleError::BroadAcl);
        }
    }
    Ok(())
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[derive(Debug)]
pub struct SingleInstanceGuard {
    handle: usize,
}

impl SingleInstanceGuard {
    fn acquire(managed_root: &Path) -> ShellResult<Self> {
        let mut hasher = DefaultHasher::new();
        managed_root
            .to_string_lossy()
            .to_lowercase()
            .hash(&mut hasher);
        let name = format!("Local\\WiGigaDict-{:016x}", hasher.finish());
        let name = wide(name.as_ref());
        // SAFETY: name is null-terminated and no SECURITY_ATTRIBUTES pointer is supplied.
        let handle =
            unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.map_err(|source| {
                ShellLifecycleError::Win32 {
                    operation: "create single-instance mutex",
                    source,
                }
            })?;
        // SAFETY: GetLastError is read immediately after CreateMutexW as required by its contract.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: handle was returned by CreateMutexW and is not retained on this branch.
            unsafe { CloseHandle(handle) }.ok();
            return Err(ShellLifecycleError::AlreadyRunning);
        }
        Ok(Self {
            handle: handle.0 as usize,
        })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns the live mutex handle returned by CreateMutexW.
        unsafe {
            let _ = CloseHandle(HANDLE(self.handle as *mut c_void));
        }
    }
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns the process token handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn ensure_not_elevated() -> ShellResult<()> {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle; token points to writable storage.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.map_err(
        |source| ShellLifecycleError::Win32 {
            operation: "open process token",
            source,
        },
    )?;
    let _token_guard = TokenHandle(token);
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0_u32;
    // SAFETY: buffer and length match TOKEN_ELEVATION exactly.
    unsafe {
        windows::Win32::Security::GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    }
    .map_err(|source| ShellLifecycleError::Win32 {
        operation: "read process elevation",
        source,
    })?;
    enforce_non_elevated(elevation.TokenIsElevated != 0)
}

fn enforce_non_elevated(is_elevated: bool) -> ShellResult<()> {
    if is_elevated {
        Err(ShellLifecycleError::Elevated)
    } else {
        Ok(())
    }
}

#[derive(Debug)]
pub struct ShellBootstrap {
    pub lifecycle: Arc<ShellLifecycle>,
    pub instance_guard: SingleInstanceGuard,
}

/// Registered (system-unique) id of the activation broadcast. Registering the same name in every
/// process yields the same id, so only WiGigaDict shells react to it.
fn activation_message_id() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    // SAFETY: ACTIVATION_MESSAGE is a static null-terminated wide string.
    *ID.get_or_init(|| unsafe { RegisterWindowMessageW(ACTIVATION_MESSAGE) })
}

/// Asks the shell that already owns this user session to show its main window.
///
/// A second launch used to exit silently, which looked exactly like a crash: the user double
/// clicked the app and nothing happened.
pub fn request_existing_instance_activation() {
    let message = activation_message_id();
    if message == 0 {
        return;
    }
    // The first instance takes the single-instance mutex long before it creates its window and
    // subclasses it. A broadcast posted inside that gap reaches nobody and is not queued, so a
    // launch fired right after the previous one silently did nothing. Wait for the live shell to
    // publish its readiness object first; a stalled shell only costs this deadline.
    wait_for_named_object(ACTIVATION_READY_OBJECT, ACTIVATION_READY_TIMEOUT);
    // SAFETY: posting a registered message id to top-level windows is the documented single
    // instance handshake; no pointer is passed in wparam/lparam.
    unsafe {
        let _ = PostMessageW(Some(HWND_BROADCAST), message, WPARAM(0), LPARAM(0));
    }
}

fn wait_for_named_object(name: PCWSTR, timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        // SAFETY: name is a null-terminated wide string that outlives this call.
        let opened = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, name) };
        if let Ok(handle) = opened {
            // SAFETY: handle was returned by OpenMutexW and is owned here.
            unsafe { CloseHandle(handle) }.ok();
            return;
        }
        if std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(ACTIVATION_READY_POLL);
    }
}

/// Publishes the readiness object once this process can answer an activation broadcast.
///
/// The handle intentionally lives until the process exits: the object must disappear exactly when
/// this shell does, so a later launch never waits on a dead instance.
fn publish_activation_readiness() {
    static READY: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let _ = READY.get_or_init(|| {
        // SAFETY: the object name is a static null-terminated wide string and no security
        // attributes are supplied.
        unsafe { CreateMutexW(None, false, ACTIVATION_READY_OBJECT) }
            .map(|handle| handle.0 as usize)
            .unwrap_or_default()
    });
}

pub fn bootstrap() -> ShellResult<ShellBootstrap> {
    ensure_not_elevated()?;
    let managed_paths = ManagedPaths::prepare()?;
    let instance_guard = SingleInstanceGuard::acquire(&managed_paths.root)?;
    Ok(ShellBootstrap {
        lifecycle: Arc::new(ShellLifecycle::new(managed_paths)),
        instance_guard,
    })
}

use windows::Win32::UI::WindowsAndMessaging::{
    WTS_CONSOLE_CONNECT, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT, WTS_REMOTE_DISCONNECT,
};

struct SessionNotificationContext {
    lifecycle: Arc<ShellLifecycle>,
    app: AppHandle,
    capture_safety: Arc<dyn Fn(ShellEvent) + Send + Sync>,
}

pub fn install_session_notifications(
    window: &WebviewWindow,
    lifecycle: Arc<ShellLifecycle>,
    capture_safety: Arc<dyn Fn(ShellEvent) + Send + Sync>,
) -> Result<(), Box<dyn std::error::Error>> {
    let hwnd = window.hwnd()?;
    // SAFETY: hwnd belongs to the live main Tauri window.
    unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }?;
    let context = Box::new(SessionNotificationContext {
        lifecycle,
        app: window.app_handle().clone(),
        capture_safety,
    });
    let raw_context = Box::into_raw(context) as usize;
    // SAFETY: hwnd is live, callback has the documented ABI, and raw_context stays allocated
    // until WM_NCDESTROY removes the subclass.
    let installed = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(session_subclass_proc),
            SESSION_SUBCLASS_ID,
            raw_context,
        )
    };
    if !installed.as_bool() {
        // SAFETY: registration succeeded above and raw_context has not been shared elsewhere.
        unsafe {
            let _ = WTSUnRegisterSessionNotification(hwnd);
            drop(Box::from_raw(
                raw_context as *mut SessionNotificationContext,
            ));
        }
        return Err(windows::core::Error::from_win32().into());
    }
    publish_activation_readiness();
    Ok(())
}

unsafe extern "system" fn session_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let context = unsafe { (reference_data as *const SessionNotificationContext).as_ref() };
    if let Some(context) = context {
        match message {
            WM_WTSSESSION_CHANGE => {
                let event = session_event_from_wparam(wparam.0 as u32);
                if let Some(event) = event {
                    publish_system_event(context, event);
                }
            }
            WM_QUERYENDSESSION => {
                publish_system_event(context, ShellEvent::QueryEndSession);
            }
            WM_ENDSESSION if wparam.0 != 0 => {
                publish_system_event(context, ShellEvent::EndSession);
            }
            WM_ENDSESSION => {
                publish_system_event(context, ShellEvent::EndSessionCancelled);
            }
            WM_NCDESTROY => {
                // SAFETY: this window owns both registrations and this is their terminal message.
                let _ = unsafe { WTSUnRegisterSessionNotification(hwnd) };
                // SAFETY: callback and id match the successful SetWindowSubclass call.
                let _ =
                    unsafe { RemoveWindowSubclass(hwnd, Some(session_subclass_proc), subclass_id) };
                // SAFETY: reference_data originated from Box::into_raw and is consumed once here.
                unsafe {
                    drop(Box::from_raw(
                        reference_data as *mut SessionNotificationContext,
                    ));
                }
            }
            _ if message != 0 && message == activation_message_id() => {
                show_main_section(&context.app, &context.lifecycle, "dictation");
            }
            _ => {}
        }
    }
    // SAFETY: forwarding unhandled messages preserves the window's existing subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn session_event_from_wparam(value: u32) -> Option<ShellEvent> {
    match value {
        WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => {
            Some(ShellEvent::SessionLock)
        }
        WTS_SESSION_UNLOCK | WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
            Some(ShellEvent::SessionUnlock)
        }
        WTS_SESSION_LOGOFF => Some(ShellEvent::SessionLogoff),
        _ => None,
    }
}

fn publish_system_event(context: &SessionNotificationContext, event: ShellEvent) {
    if !event.resumes_session() {
        (context.capture_safety)(event);
    }
    if context.lifecycle.on_event(event).is_err() {
        return;
    }
    if !event.resumes_session() {
        if let Some(main) = context.app.get_webview_window(MAIN_WINDOW_LABEL) {
            let _ = main.hide();
        }
        if let Some(overlay) = context.app.get_webview_window(OVERLAY_WINDOW_LABEL) {
            let _ = overlay.hide();
        }
    }
    if let Ok(status) = context.lifecycle.status() {
        let _ = context.app.emit("shell://lifecycle", status);
    }
}

pub fn setup_tray(app: &tauri::App, lifecycle: Arc<ShellLifecycle>) -> tauri::Result<()> {
    let dictation =
        tauri::menu::MenuItem::with_id(app, "show_dictation", "Диктовка", true, None::<&str>)?;
    let history =
        tauri::menu::MenuItem::with_id(app, "show_history", "История", true, None::<&str>)?;
    let settings =
        tauri::menu::MenuItem::with_id(app, "show_settings", "Настройки", true, None::<&str>)?;
    let quit = tauri::menu::MenuItem::with_id(app, "quit_shell", "Выйти", true, None::<&str>)?;
    let menu = tauri::menu::Menu::with_items(app, &[&dictation, &history, &settings, &quit])?;
    let mut builder = tauri::tray::TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip("WiGigaDict · локальная диктовка")
        .show_menu_on_left_click(true);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show_dictation" => show_main_section(app, &lifecycle, "dictation"),
            "show_history" => show_main_section(app, &lifecycle, "history"),
            "show_settings" => show_main_section(app, &lifecycle, "settings"),
            "quit_shell" => {
                let _ = lifecycle.on_event(ShellEvent::AppExit);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn show_main_section(app: &AppHandle, lifecycle: &ShellLifecycle, section: &'static str) {
    let can_show = lifecycle
        .status()
        .is_ok_and(|status| status.phase == ShellPhase::Ready);
    if can_show && let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = main.show();
        let _ = main.set_focus();
        let _ = app.emit_to(MAIN_WINDOW_LABEL, "shell-navigate", section);
    }
}
pub fn authorize_main_window(window: &WebviewWindow) -> ShellResult<()> {
    authorize_main_label(window.label())
}

pub fn authorize_overlay_window(window: &WebviewWindow) -> ShellResult<()> {
    authorize_overlay_label(window.label())
}

fn authorize_main_label(label: &str) -> ShellResult<()> {
    if label == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err(ShellLifecycleError::InvalidCaller(label.to_owned()))
    }
}

fn authorize_overlay_label(label: &str) -> ShellResult<()> {
    if label == OVERLAY_WINDOW_LABEL {
        Ok(())
    } else {
        Err(ShellLifecycleError::InvalidCaller(label.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn test_lifecycle() -> ShellLifecycle {
        ShellLifecycle::new(ManagedPaths {
            root: std::env::current_dir().expect("current directory"),
        })
    }

    #[test]
    fn acl_validator_rejects_every_noncanonical_allow_ace_type() {
        assert!(is_unexpected_allow_ace_type(
            ACCESS_ALLOWED_OBJECT_ACE_TYPE as u8
        ));
        assert!(is_unexpected_allow_ace_type(
            ACCESS_ALLOWED_CALLBACK_ACE_TYPE as u8
        ));
        assert!(is_unexpected_allow_ace_type(
            ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE as u8
        ));
        assert!(!is_unexpected_allow_ace_type(1));
    }

    #[test]
    fn current_test_process_is_non_elevated() {
        ensure_not_elevated().expect("quality tests must run non-elevated");
    }

    #[test]
    fn elevation_policy_fails_closed() {
        assert!(enforce_non_elevated(false).is_ok());
        assert!(matches!(
            enforce_non_elevated(true),
            Err(ShellLifecycleError::Elevated)
        ));
    }

    #[test]
    fn activation_waits_for_a_live_shell_and_still_honours_its_deadline() {
        // No such object exists: the wait must end at the deadline instead of hanging or
        // posting a broadcast into the startup gap that nobody can answer yet.
        let absent = windows::core::w!(r"Local\WiGigaDict.ActivationReady.AbsentTestObject");
        let started = std::time::Instant::now();
        wait_for_named_object(absent, std::time::Duration::from_millis(200));
        let waited = started.elapsed();
        assert!(
            waited >= std::time::Duration::from_millis(150),
            "an absent shell must be waited for, waited {waited:?}"
        );
        assert!(
            waited < std::time::Duration::from_secs(3),
            "the deadline must be honoured, waited {waited:?}"
        );

        // A published object ends the wait immediately.
        let present = windows::core::w!(r"Local\WiGigaDict.ActivationReady.PresentTestObject");
        // SAFETY: the object name is a static null-terminated wide string.
        let handle = unsafe { CreateMutexW(None, false, present) }.expect("test readiness object");
        let started = std::time::Instant::now();
        wait_for_named_object(present, std::time::Duration::from_secs(5));
        let waited = started.elapsed();
        // SAFETY: handle was returned by CreateMutexW and is owned here.
        unsafe { CloseHandle(handle) }.ok();
        assert!(
            waited < std::time::Duration::from_millis(500),
            "a ready shell must not be waited for, waited {waited:?}"
        );
    }

    #[test]
    fn cancelled_shutdown_makes_the_shell_reachable_again() {
        let lifecycle = test_lifecycle();
        lifecycle
            .on_event(ShellEvent::QueryEndSession)
            .expect("query end session");
        let blocked = lifecycle.status().expect("status");
        assert_eq!(blocked.phase, ShellPhase::ShuttingDown);
        assert!(!blocked.accepting_new_work);
        assert!(lifecycle.begin_capture("session-blocked").is_err());

        lifecycle
            .on_event(ShellEvent::EndSessionCancelled)
            .expect("cancelled end session");
        let resumed = lifecycle.status().expect("status");
        assert_eq!(resumed.phase, ShellPhase::Ready);
        assert!(resumed.accepting_new_work);
        assert!(lifecycle.begin_capture("session-after-cancel").is_ok());
    }

    #[test]
    fn a_confirmed_or_explicit_exit_is_never_taken_back() {
        for terminal in [
            ShellEvent::EndSession,
            ShellEvent::SessionLogoff,
            ShellEvent::AppExit,
        ] {
            let lifecycle = test_lifecycle();
            lifecycle.on_event(terminal).expect("terminal event");
            lifecycle
                .on_event(ShellEvent::EndSessionCancelled)
                .expect("cancellation after a terminal event");
            let state = lifecycle.status().expect("status");
            assert_eq!(state.phase, ShellPhase::ShuttingDown);
            assert!(!state.accepting_new_work);
        }
    }

    #[test]
    fn an_unlock_never_resurrects_a_shutting_down_shell() {
        let lifecycle = test_lifecycle();
        lifecycle
            .on_event(ShellEvent::QueryEndSession)
            .expect("query end session");
        lifecycle
            .on_event(ShellEvent::SessionUnlock)
            .expect("unlock");
        assert_eq!(
            lifecycle.status().expect("status").phase,
            ShellPhase::ShuttingDown
        );
    }

    #[test]
    fn every_session_disconnect_has_a_matching_reconnect() {
        for (away, back) in [
            (WTS_SESSION_LOCK, WTS_SESSION_UNLOCK),
            (WTS_CONSOLE_DISCONNECT, WTS_CONSOLE_CONNECT),
            (WTS_REMOTE_DISCONNECT, WTS_REMOTE_CONNECT),
        ] {
            let lifecycle = test_lifecycle();
            let away = session_event_from_wparam(away).expect("disconnect event");
            let back = session_event_from_wparam(back).expect("reconnect event");
            lifecycle.on_event(away).expect("disconnect");
            assert_eq!(
                lifecycle.status().expect("status").phase,
                ShellPhase::SessionLocked
            );
            lifecycle.on_event(back).expect("reconnect");
            assert_eq!(lifecycle.status().expect("status").phase, ShellPhase::Ready);
        }
    }

    #[test]
    fn native_session_messages_map_only_to_bounded_lifecycle_events() {
        assert_eq!(
            session_event_from_wparam(WTS_SESSION_LOCK),
            Some(ShellEvent::SessionLock)
        );
        assert_eq!(
            session_event_from_wparam(WTS_CONSOLE_DISCONNECT),
            Some(ShellEvent::SessionLock)
        );
        assert_eq!(
            session_event_from_wparam(WTS_REMOTE_DISCONNECT),
            Some(ShellEvent::SessionLock)
        );
        assert_eq!(
            session_event_from_wparam(WTS_SESSION_UNLOCK),
            Some(ShellEvent::SessionUnlock)
        );
        assert_eq!(
            session_event_from_wparam(WTS_SESSION_LOGOFF),
            Some(ShellEvent::SessionLogoff)
        );
        assert_eq!(session_event_from_wparam(u32::MAX), None);
    }

    #[test]
    fn managed_root_must_be_a_strict_local_app_data_child() {
        let base = PathBuf::from(r"C:\Users\owner\AppData\Local");
        assert!(validate_managed_root(&base, &base.join("WiGigaDict")).is_ok());
        assert!(matches!(
            validate_managed_root(&base, &base),
            Err(ShellLifecycleError::OutsideManagedRoot)
        ));
        assert!(matches!(
            validate_managed_root(&base, Path::new(r"C:\ProgramData\WiGigaDict")),
            Err(ShellLifecycleError::OutsideManagedRoot)
        ));
    }

    struct ManagedAclFixture {
        root: PathBuf,
    }

    impl Drop for ManagedAclFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir(self.root.join("child"));
            let _ = fs::remove_dir(&self.root);
        }
    }

    #[test]
    fn managed_acl_removes_inherited_groups_and_propagates_to_children() {
        let local = local_app_data().expect("LocalAppData");
        let root = local.join(format!("WiGigaDict-acl-test-{}", Uuid::new_v4()));
        validate_managed_root(&local, &root).expect("test root containment");
        fs::create_dir(&root).expect("create ACL fixture");
        let _fixture = ManagedAclFixture { root: root.clone() };
        let current_sid = current_user_sid_string().expect("current user SID");
        harden_managed_acl(&root, &current_sid).expect("harden fixture ACL");
        ensure_restricted_acl(&root, &current_sid).expect("restricted root ACL");
        let child = root.join("child");
        fs::create_dir(&child).expect("create inherited child");
        ensure_restricted_acl(&child, &current_sid).expect("restricted inherited child ACL");
        assert!(is_allowed_managed_sid(&current_sid, &current_sid));
        assert!(is_allowed_managed_sid("S-1-5-18", &current_sid));
        assert!(is_allowed_managed_sid("S-1-5-32-544", &current_sid));
        assert!(!is_allowed_managed_sid("S-1-5-11", &current_sid));
    }

    #[test]
    fn startup_generation_is_unique_per_process_generation() {
        let first = test_lifecycle().status().expect("first status");
        let second = test_lifecycle().status().expect("second status");
        assert_ne!(first.generation_id, second.generation_id);
        assert_eq!(first.phase, ShellPhase::Ready);
    }

    #[test]
    fn lock_moves_active_capture_to_recovery_and_unlock_never_resumes_it() {
        let lifecycle = test_lifecycle();
        lifecycle.begin_capture("session-1").expect("begin capture");
        lifecycle
            .on_event(ShellEvent::SessionLock)
            .expect("lock event");
        let locked = lifecycle.status().expect("locked status");
        assert_eq!(locked.phase, ShellPhase::SessionLocked);
        assert!(!locked.accepting_new_work);
        assert_eq!(locked.active_capture_state, "recovery");
        assert_eq!(locked.recovery_reason, Some("windows_session_locked"));

        lifecycle
            .on_event(ShellEvent::SessionUnlock)
            .expect("unlock event");
        let unlocked = lifecycle.status().expect("unlocked status");
        assert_eq!(unlocked.phase, ShellPhase::Ready);
        assert!(unlocked.accepting_new_work);
        assert_eq!(unlocked.active_capture_state, "recovery");
    }

    #[test]
    fn durable_capture_recovery_releases_the_capture_slot() {
        let lifecycle = test_lifecycle();
        lifecycle.begin_capture("session-1").expect("begin capture");
        lifecycle.recover_capture("session-1", "audio_device_lost");
        lifecycle.finish_capture_recovery("different-session");
        assert_eq!(
            lifecycle
                .status()
                .expect("mismatched status")
                .active_capture_state,
            "recovery"
        );

        lifecycle.finish_capture_recovery("session-1");
        assert_eq!(
            lifecycle
                .status()
                .expect("settled status")
                .active_capture_state,
            "idle"
        );
        lifecycle
            .begin_capture("session-2")
            .expect("capture slot released after durable recovery");
    }

    #[test]
    fn repeated_safety_events_are_idempotent_and_shutdown_dominates_unlock() {
        let lifecycle = test_lifecycle();
        lifecycle.begin_capture("session-2").expect("begin capture");
        lifecycle
            .on_event(ShellEvent::SessionLock)
            .expect("first lock");
        lifecycle
            .on_event(ShellEvent::SessionLock)
            .expect("duplicate lock");
        assert_eq!(
            lifecycle.status().expect("status").recovery_reason,
            Some("windows_session_locked")
        );
        lifecycle
            .on_event(ShellEvent::QueryEndSession)
            .expect("shutdown query");
        lifecycle
            .on_event(ShellEvent::SessionUnlock)
            .expect("late unlock");
        let status = lifecycle.status().expect("shutdown status");
        assert_eq!(status.phase, ShellPhase::ShuttingDown);
        assert!(!status.accepting_new_work);
        assert!(lifecycle.is_shutting_down());
    }

    #[test]
    fn second_instance_cannot_acquire_the_same_writer_generation_mutex() {
        let root = PathBuf::from(format!(
            r"C:\Users\owner\AppData\Local\WiGigaDict-test-{}",
            Uuid::new_v4()
        ));
        let first = SingleInstanceGuard::acquire(&root).expect("first mutex");
        let second = SingleInstanceGuard::acquire(&root);
        assert!(matches!(second, Err(ShellLifecycleError::AlreadyRunning)));
        drop(first);
        let third = SingleInstanceGuard::acquire(&root).expect("mutex after release");
        drop(third);
    }

    #[test]
    fn activation_message_id_is_registered_once_and_is_stable() {
        let first = activation_message_id();
        assert_ne!(first, 0, "activation message must register");
        assert_eq!(first, activation_message_id());
        // Registered messages live in the reserved 0xC000..=0xFFFF range, so they can never
        // collide with the WM_* messages the subclass handles explicitly.
        assert!((0xC000..=0xFFFF).contains(&first));
    }

    #[test]
    fn main_and_overlay_command_boundaries_are_isolated() {
        assert!(authorize_main_label(MAIN_WINDOW_LABEL).is_ok());
        assert!(matches!(
            authorize_main_label(OVERLAY_WINDOW_LABEL),
            Err(ShellLifecycleError::InvalidCaller(_))
        ));
        assert!(authorize_overlay_label(OVERLAY_WINDOW_LABEL).is_ok());
        assert!(matches!(
            authorize_overlay_label(MAIN_WINDOW_LABEL),
            Err(ShellLifecycleError::InvalidCaller(_))
        ));
    }

    #[test]
    fn tauri_config_has_strict_csp_and_explicit_window_capabilities() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("valid Tauri config JSON");
        assert_eq!(
            config["app"]["security"]["capabilities"],
            serde_json::json!(["main", "overlay"])
        );
        assert_eq!(
            config["bundle"]["icon"],
            serde_json::json!(["icons/icon.ico"])
        );
        let csp = config["app"]["security"]["csp"]
            .as_str()
            .expect("CSP string");
        for required in [
            "default-src 'self'",
            "object-src 'none'",
            "base-uri 'none'",
            "frame-src 'none'",
            "frame-ancestors 'none'",
            "form-action 'none'",
        ] {
            assert!(csp.contains(required), "missing CSP directive: {required}");
        }
        assert!(!csp.contains("https:"));
        assert!(!csp.contains("'unsafe-eval'"));
        assert!(!csp.contains("'unsafe-inline'"));
        let labels = config["app"]["windows"]
            .as_array()
            .expect("windows")
            .iter()
            .map(|window| window["label"].as_str().expect("window label"))
            .collect::<Vec<_>>();
        assert_eq!(labels, [MAIN_WINDOW_LABEL, OVERLAY_WINDOW_LABEL]);
        let windows = config["app"]["windows"].as_array().expect("windows");
        let main = &windows[0];
        let overlay = &windows[1];
        assert_eq!(main["minWidth"], serde_json::json!(760));
        assert_eq!(main["minHeight"], serde_json::json!(560));
        assert_eq!(overlay["focus"], serde_json::json!(false));
        assert_eq!(overlay["visible"], serde_json::json!(false));
        assert_eq!(overlay["alwaysOnTop"], serde_json::json!(true));
        assert_eq!(overlay["skipTaskbar"], serde_json::json!(true));
        assert_eq!(overlay["decorations"], serde_json::json!(false));
    }

    #[test]
    fn overlay_capability_is_render_event_only_and_main_is_not_core_default() {
        let main: Value = serde_json::from_str(include_str!("../capabilities/main.json"))
            .expect("valid main capability");
        let overlay: Value = serde_json::from_str(include_str!("../capabilities/overlay.json"))
            .expect("valid overlay capability");
        assert_eq!(main["windows"], serde_json::json!([MAIN_WINDOW_LABEL]));
        assert_eq!(
            overlay["windows"],
            serde_json::json!([OVERLAY_WINDOW_LABEL])
        );
        let main_permissions = main["permissions"].as_array().expect("main permissions");
        assert!(!main_permissions.iter().any(|value| value == "core:default"));
        assert_eq!(
            overlay["permissions"],
            serde_json::json!(["core:event:allow-listen", "core:event:allow-unlisten"])
        );
        let serialized = serde_json::to_string(&overlay).expect("serialize overlay");
        for forbidden in [
            "filesystem",
            "shell",
            "network",
            "updater",
            "model",
            "delete",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "forbidden capability: {forbidden}"
            );
        }
    }
}
