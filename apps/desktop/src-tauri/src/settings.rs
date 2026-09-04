use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use wigigadict_storage::{
    AppConfiguration, CleanupProfileOption, ConfigurationRepository, ConfigurationUpdate,
    RuntimeProfileOption,
};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
    SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, w};

use crate::HotkeyBinding;
use crate::archive::{self, ArchiveService};
use crate::capture::{
    CapturePhase, CaptureService, InputDeviceStatus, input_devices, validate_hotkey,
};
use crate::diagnostics::DiagnosticService;
use crate::shell_lifecycle;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub configuration: AppConfiguration,
    pub runtime_profiles: Vec<RuntimeProfileOption>,
    pub cleanup_profiles: Vec<CleanupProfileOption>,
    pub input_devices: Vec<InputDeviceStatus>,
    pub input_device_error: Option<String>,
    pub startup_registered: bool,
}

pub struct SettingsService {
    database_path: PathBuf,
    archive: ArchiveService,
}

impl SettingsService {
    pub fn new(database_path: PathBuf, managed_root: PathBuf) -> Self {
        let archive = ArchiveService::new(&database_path, managed_root);
        Self {
            database_path,
            archive,
        }
    }

    fn repository(&self) -> Result<ConfigurationRepository, String> {
        ConfigurationRepository::open(&self.database_path).map_err(|error| error.to_string())
    }

    fn view(&self) -> Result<SettingsView, String> {
        let catalog = self
            .repository()?
            .catalog()
            .map_err(|error| error.to_string())?;
        let (input_devices, input_device_error) = match input_devices() {
            Ok(devices) => (devices, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Ok(SettingsView {
            configuration: catalog.configuration,
            runtime_profiles: catalog.runtime_profiles,
            cleanup_profiles: catalog.cleanup_profiles,
            input_devices,
            input_device_error,
            startup_registered: startup_entry_matches()?,
        })
    }
}

pub fn initialize_configuration(database_path: &Path) -> Result<AppConfiguration, String> {
    let mut repository =
        ConfigurationRepository::open(database_path).map_err(|error| error.to_string())?;
    let configuration = repository
        .ensure_default(now_ms())
        .map_err(|error| error.to_string())?;
    let requested = configuration
        .archive_directory
        .as_deref()
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(archive::default_directory)?;
    let directory = archive::prepare_directory(&requested)?;
    let directory = directory.to_string_lossy().into_owned();
    if configuration.archive_directory.as_deref() == Some(directory.as_str()) {
        return Ok(configuration);
    }
    let update = ConfigurationUpdate {
        expected_config_version: configuration.config_version,
        hotkey_binding: configuration.hotkey_binding,
        microphone_device_id: configuration.microphone_device_id,
        active_runtime_profile_id: configuration.active_runtime_profile_id,
        active_cleanup_profile_id: configuration.active_cleanup_profile_id,
        startup_enabled: configuration.startup_enabled,
        warmup_enabled: configuration.warmup_enabled,
        diagnostic_mode: configuration.diagnostic_mode,
        archive_directory: Some(directory),
    };
    repository
        .update(&update, now_ms())
        .map_err(|error| error.to_string())
}

pub fn reconcile_configured_startup(enabled: bool) -> Result<(), String> {
    set_startup_entry(enabled)
}

#[tauri::command]
pub fn settings_get(
    window: tauri::WebviewWindow,
    service: State<'_, SettingsService>,
) -> Result<SettingsView, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    service.view()
}

#[tauri::command]
pub fn archive_directory_pick(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    pick_archive_directory()
}

#[tauri::command]

pub fn settings_update(
    app: AppHandle,
    window: tauri::WebviewWindow,
    service: State<'_, SettingsService>,
    binding: State<'_, HotkeyBinding>,
    capture: State<'_, std::sync::Arc<CaptureService>>,
    diagnostics: State<'_, DiagnosticService>,
    mut update: ConfigurationUpdate,
) -> Result<SettingsView, String> {
    shell_lifecycle::authorize_main_window(&window).map_err(|error| error.to_string())?;
    let capture_status = capture.status()?;
    if matches!(
        capture_status.phase,
        CapturePhase::Preparing | CapturePhase::Recording | CapturePhase::Finalizing
    ) {
        return Err("active capture must finish before settings can change".into());
    }

    let next_shortcut = validate_hotkey(&update.hotkey_binding)?;
    if let Some(selected) = update.microphone_device_id.as_deref()
        && !input_devices()?
            .iter()
            .any(|device| device.id == selected && device.healthy)
    {
        return Err("selected microphone is unavailable or unhealthy".into());
    }

    let mut repository = service.repository()?;
    repository
        .validate_update(&update)
        .map_err(|error| error.to_string())?;
    let previous = repository
        .active()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "active configuration is missing".to_owned())?;
    let requested_archive = update
        .archive_directory
        .as_deref()
        .ok_or_else(|| "local archive directory is required".to_owned())?;
    let canonical_archive = archive::prepare_directory(Path::new(requested_archive))?;
    update.archive_directory = Some(canonical_archive.to_string_lossy().into_owned());
    if previous.archive_directory != update.archive_directory {
        service.archive.backfill_to(&canonical_archive)?;
    }
    let startup_was_registered = startup_entry_matches()?;

    let mut current_binding = binding
        .0
        .lock()
        .map_err(|_| "hotkey binding mutex was poisoned".to_owned())?;
    let previous_shortcut = *current_binding;
    let hotkey_changed = previous_shortcut.id() != next_shortcut.id();
    if hotkey_changed {
        replace_registered_shortcut(&app, previous_shortcut, next_shortcut)?;
    }

    if let Err(error) = set_startup_entry(update.startup_enabled) {
        rollback_shortcut(
            &app,
            &mut current_binding,
            previous_shortcut,
            next_shortcut,
            hotkey_changed,
        );
        return Err(error);
    }

    if let Err(error) = capture.select_device(update.microphone_device_id.clone()) {
        let _ = set_startup_entry(startup_was_registered);
        rollback_shortcut(
            &app,
            &mut current_binding,
            previous_shortcut,
            next_shortcut,
            hotkey_changed,
        );
        return Err(error);
    }
    if let Err(error) = capture.select_runtime_profile(update.active_runtime_profile_id.clone()) {
        let _ = capture.select_device(previous.microphone_device_id.clone());
        let _ = set_startup_entry(startup_was_registered);
        rollback_shortcut(
            &app,
            &mut current_binding,
            previous_shortcut,
            next_shortcut,
            hotkey_changed,
        );
        return Err(error);
    }

    if let Err(error) = repository.update(&update, now_ms()) {
        let _ = capture.select_device(previous.microphone_device_id);
        let _ = capture.select_runtime_profile(previous.active_runtime_profile_id);
        let _ = set_startup_entry(startup_was_registered);
        rollback_shortcut(
            &app,
            &mut current_binding,
            previous_shortcut,
            next_shortcut,
            hotkey_changed,
        );
        return Err(error.to_string());
    }

    if hotkey_changed {
        *current_binding = next_shortcut;
    }
    drop(current_binding);
    diagnostics.set_expanded_events_enabled(update.diagnostic_mode);
    service.view()
}

fn replace_registered_shortcut(
    app: &AppHandle,
    previous: Shortcut,
    next: Shortcut,
) -> Result<(), String> {
    app.global_shortcut()
        .register(next)
        .map_err(|_| "global hotkey is already reserved".to_owned())?;
    if app.global_shortcut().unregister(previous).is_err() {
        let _ = app.global_shortcut().unregister(next);
        return Err("previous global hotkey could not be released".into());
    }
    Ok(())
}

fn pick_archive_directory() -> Result<Option<String>, String> {
    const RPC_E_CHANGED_MODE: HRESULT = HRESULT(0x80010106_u32 as i32);
    const HRESULT_CANCELLED: HRESULT = HRESULT(0x800704C7_u32 as i32);

    // SAFETY: the command owns no COM objects before this call. Tauri may already have initialized
    // the thread with a different apartment; in that case COM is usable but must not be uninitialized
    // by us.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let uninitialize = initialized.is_ok();
    if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
        return Err(format!(
            "folder picker initialization failed: {initialized:?}"
        ));
    }

    let result = (|| {
        // SAFETY: COM is initialized on this thread and FileOpenDialog is an in-process COM class.
        let dialog: IFileOpenDialog = unsafe {
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("folder picker is unavailable: {error}"))?
        };
        // SAFETY: dialog is a live COM object owned by this scope.
        let options = unsafe { dialog.GetOptions() }
            .map_err(|error| format!("folder picker options are unavailable: {error}"))?;
        // SAFETY: flags are valid for IFileOpenDialog and dialog remains live.
        unsafe {
            dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
        }
        .map_err(|error| format!("folder picker options were rejected: {error}"))?;
        // SAFETY: a null owner is supported and lets Windows place the native dialog.
        if let Err(error) = unsafe { dialog.Show(None) } {
            if error.code() == HRESULT_CANCELLED {
                return Ok(None);
            }
            return Err(format!("folder picker failed: {error}"));
        }
        // SAFETY: dialog has a selected filesystem item after a successful Show.
        let item = unsafe { dialog.GetResult() }
            .map_err(|error| format!("selected folder is unavailable: {error}"))?;
        // SAFETY: GetDisplayName allocates the returned string; it is copied before CoTaskMemFree.
        let value = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
            .map_err(|error| format!("selected folder path is unavailable: {error}"))?;
        // SAFETY: value is the valid PWSTR returned above.
        let decoded = unsafe { value.to_string() };
        // SAFETY: GetDisplayName documents CoTaskMemFree for this allocation.
        unsafe { CoTaskMemFree(Some(value.0.cast())) };
        let path =
            decoded.map_err(|error| format!("selected folder path is not Unicode: {error}"))?;
        let canonical = archive::prepare_directory(Path::new(&path))?;
        Ok(Some(canonical.to_string_lossy().into_owned()))
    })();

    if uninitialize {
        // SAFETY: this thread successfully initialized COM above and all COM objects are dropped.
        unsafe { CoUninitialize() };
    }
    result
}

fn rollback_shortcut(
    app: &AppHandle,
    binding: &mut std::sync::MutexGuard<'_, Shortcut>,
    previous: Shortcut,
    next: Shortcut,
    changed: bool,
) {
    if !changed {
        return;
    }
    let _ = app.global_shortcut().register(previous);
    let _ = app.global_shortcut().unregister(next);
    **binding = previous;
}

fn startup_command(executable: &Path) -> Result<String, String> {
    let path = executable
        .to_str()
        .ok_or_else(|| "application path is not Unicode".to_owned())?;
    if path.contains('"') {
        return Err("application path contains an invalid quote".into());
    }
    Ok(format!(r#""{path}" --startup"#))
}

fn expected_startup_command() -> Result<String, String> {
    let executable =
        std::env::current_exe().map_err(|_| "application executable path is unavailable")?;
    startup_command(&executable)
}

fn set_startup_entry(enabled: bool) -> Result<(), String> {
    let key = create_run_key()?;
    let result = if enabled {
        let command = expected_startup_command()?;
        let encoded = OsStr::new(&command)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: encoded is a live UTF-16 buffer and the byte view has the same lifetime.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                encoded.as_ptr().cast::<u8>(),
                encoded.len() * std::mem::size_of::<u16>(),
            )
        };
        // SAFETY: key is an open HKCU Run key and both value name and bytes are valid.
        unsafe { RegSetValueExW(key.0, w!("WiGigaDict"), None, REG_SZ, Some(bytes)) }
    } else {
        // SAFETY: key is an open HKCU Run key and the static value name is valid.
        let result = unsafe { RegDeleteValueW(key.0, w!("WiGigaDict")) };
        if result == ERROR_FILE_NOT_FOUND {
            ERROR_SUCCESS
        } else {
            result
        }
    };
    win32_result(result, "update user startup entry")
}

fn startup_entry_matches() -> Result<bool, String> {
    let mut raw_key = HKEY::default();
    // SAFETY: output points to initialized storage and the static subkey is valid.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            KEY_QUERY_VALUE,
            &mut raw_key,
        )
    };
    if opened == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    win32_result(opened, "open user startup key")?;
    let key = RegistryKey(raw_key);
    let mut value_type = REG_SZ;
    let mut byte_count = 0_u32;
    // SAFETY: key and value name are valid; this first call queries the required size.
    let sized = unsafe {
        RegQueryValueExW(
            key.0,
            w!("WiGigaDict"),
            None,
            Some(&mut value_type),
            None,
            Some(&mut byte_count),
        )
    };
    if sized == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    win32_result(sized, "query user startup entry")?;
    if value_type != REG_SZ || byte_count == 0 || !byte_count.is_multiple_of(2) {
        return Ok(false);
    }
    let mut encoded = vec![0_u16; byte_count as usize / 2];
    // SAFETY: encoded has byte_count writable bytes and key/value name remain valid.
    let queried = unsafe {
        RegQueryValueExW(
            key.0,
            w!("WiGigaDict"),
            None,
            Some(&mut value_type),
            Some(encoded.as_mut_ptr().cast::<u8>()),
            Some(&mut byte_count),
        )
    };
    win32_result(queried, "read user startup entry")?;
    while encoded.last() == Some(&0) {
        encoded.pop();
    }
    let actual =
        String::from_utf16(&encoded).map_err(|_| "user startup entry is not UTF-16".to_owned())?;
    Ok(actual == expected_startup_command()?)
}

fn create_run_key() -> Result<RegistryKey, String> {
    let mut raw_key = HKEY::default();
    // SAFETY: output points to initialized storage and all static inputs are valid.
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            w!(""),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut raw_key,
            None,
        )
    };
    win32_result(result, "open user startup key")?;
    Ok(RegistryKey(raw_key))
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by RegOpenKeyExW or RegCreateKeyExW and is owned here.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn win32_result(result: WIN32_ERROR, operation: &str) -> Result<(), String> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "{operation} failed with Windows error {}",
            result.0
        ))
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
    fn startup_command_is_quoted_and_user_level() {
        let command = startup_command(Path::new(r"C:\Program Files\WiGigaDict\app.exe")).unwrap();
        assert_eq!(
            command,
            r#""C:\Program Files\WiGigaDict\app.exe" --startup"#
        );
        assert!(!command.to_ascii_lowercase().contains("runas"));
    }
}
