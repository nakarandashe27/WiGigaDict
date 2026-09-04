#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() && launch_fixture_for_executable()? {
        return Ok(());
    }
    if arguments.is_empty()
        && let Some((
            surface,
            expected_process,
            report_path,
            activation_title,
            observer_title_marker,
            prepare_vscode_editor,
        )) = external_defaults()
    {
        let mut generated = vec![
            "external".to_owned(),
            surface.to_owned(),
            expected_process.to_owned(),
            report_path.to_string_lossy().into_owned(),
        ];
        if let Some(title) = activation_title {
            generated.push(title.to_owned());
        }
        if let Some(marker) = observer_title_marker {
            generated.push(marker.to_owned());
        } else if prepare_vscode_editor {
            generated.push(String::new());
        }
        if prepare_vscode_editor {
            generated.push("prepare_vscode_editor".to_owned());
        }
        return run_external(&generated);
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "external")
    {
        return run_external(&arguments);
    }

    let report_path = arguments
        .first()
        .map(std::path::PathBuf::from)
        .or_else(default_report_path);
    let report = wigigadict_win32_spike::windows::run_automated_matrix()?;
    write_report(report_path, &report)?;

    if report.all_required_passed() {
        Ok(())
    } else {
        Err("one or more required Win32 spike checks failed".into())
    }
}

#[cfg(windows)]
type ExternalDefaults = (
    &'static str,
    &'static str,
    std::path::PathBuf,
    Option<&'static str>,
    Option<&'static str>,
    bool,
);

#[cfg(windows)]
fn external_defaults() -> Option<ExternalDefaults> {
    let executable = std::env::current_exe().ok()?;
    let stem = executable.file_stem()?.to_str()?;
    let (surface, expected_process, activation_title, observer_title_marker, prepare_vscode_editor) =
        match stem {
            "wigigadict-vscode-probe" => (
                "vscode_codex",
                "Code.exe",
                Some("external-target.txt"),
                None,
                true,
            ),
            "wigigadict-browser-probe" => (
                "browser",
                "chrome.exe",
                Some("WiGigaDict M0 browser fixture"),
                Some("[ack]"),
                false,
            ),
            "wigigadict-terminal-probe" => (
                "terminal_claude_code",
                "WindowsTerminal.exe",
                None,
                None,
                false,
            ),
            _ => return None,
        };
    let repository = executable.parent()?.parent()?.parent()?;
    let report_path = repository.join(format!(
        "artifacts/win32-spike/external-{surface}-{}.json",
        std::process::id()
    ));
    Some((
        surface,
        expected_process,
        report_path,
        activation_title,
        observer_title_marker,
        prepare_vscode_editor,
    ))
}

#[cfg(windows)]
fn launch_fixture_for_executable() -> Result<bool, Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let stem = executable
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("fixture launcher executable name is unavailable")?;
    let repository = executable
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .ok_or("fixture launcher is not under target/<profile>")?;
    let artifacts = repository.join("artifacts/win32-spike");

    match stem {
        "wigigadict-vscode-fixture-launcher" => {
            let local_app_data = std::env::var_os("LOCALAPPDATA")
                .ok_or("LOCALAPPDATA is unavailable for the VS Code fixture")?;
            let code = std::path::PathBuf::from(local_app_data)
                .join("Programs/Microsoft VS Code/Code.exe");
            let profile = artifacts.join(format!("vscode-profile-{}", std::process::id()));
            let extensions = artifacts.join(format!("vscode-extensions-{}", std::process::id()));
            std::process::Command::new(code)
                .arg("--user-data-dir")
                .arg(profile)
                .arg("--extensions-dir")
                .arg(extensions)
                .arg("--disable-extensions")
                .arg("--new-window")
                .arg(repository.join("tests/win32-spike/fixtures/external-target.txt"))
                .spawn()?;
            Ok(true)
        }
        "wigigadict-browser-fixture-launcher" => {
            let chrome =
                std::path::Path::new(r"C:\Program Files\Google\Chrome\Application\chrome.exe");
            let profile = artifacts.join(format!("chrome-profile-{}", std::process::id()));
            std::process::Command::new(chrome)
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg("--app=http://127.0.0.1:8765/browser-target.html")
                .arg("--no-first-run")
                .arg("--disable-sync")
                .spawn()?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(windows)]
fn run_external(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let surface = arguments.get(1).ok_or("external surface is required")?;
    if !matches!(
        surface.as_str(),
        "vscode_codex" | "terminal_claude_code" | "browser"
    ) {
        return Err(
            "external surface must be vscode_codex, terminal_claude_code, or browser".into(),
        );
    }
    let expected_process = arguments
        .get(2)
        .ok_or("expected foreground process name is required")?;
    let report_path = arguments
        .get(3)
        .map(std::path::PathBuf::from)
        .ok_or("external report path is required")?;
    let activation_title = arguments.get(4).map(String::as_str);
    let observer_title_marker = arguments
        .get(5)
        .map(String::as_str)
        .filter(|value| !value.is_empty());
    let prepare_vscode_editor = arguments
        .get(6)
        .is_some_and(|argument| argument == "prepare_vscode_editor");
    let report = wigigadict_win32_spike::windows::run_external_probe(
        surface,
        expected_process,
        activation_title,
        observer_title_marker,
        prepare_vscode_editor,
    )?;
    write_report(Some(report_path), &report)?;

    if report.all_required_passed() {
        Ok(())
    } else {
        Err("external Win32 probe did not meet its fail-closed acceptance contract".into())
    }
}

#[cfg(windows)]
fn write_report<T: serde::Serialize>(
    report_path: Option<std::path::PathBuf>,
    report: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(report)?;

    if let Some(path) = report_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, format!("{json}\n"))?;
    }

    println!("{json}");
    Ok(())
}

#[cfg(windows)]
fn default_report_path() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let repository = executable.parent()?.parent()?.parent()?;
    Some(repository.join("artifacts/win32-spike/interactive-default.json"))
}

#[cfg(not(windows))]
fn main() {
    eprintln!("wigigadict-win32-spike is Windows-only");
    std::process::exit(2);
}
