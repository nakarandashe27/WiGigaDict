# WiGigaDict desktop shell

The Rust crate owns the Windows process lifecycle and every privileged Tauri command. React remains a bundled presentation layer.

## Step 7 contract

- Bootstrap rejects an elevated token, prepares `%LOCALAPPDATA%\WiGigaDict\`, applies a protected inheritable owner/SYSTEM/Administrators DACL, and acquires the per-user named mutex before starting the sidecar or any future writer owner.
- Each process generation has a UUID. A second shell exits before writer-capable setup.
- Closing the main window hides it to the tray. Only the tray Quit action requests process exit.
- Main-window WTS notifications map lock/disconnect/logoff and Windows shutdown messages into a fail-closed lifecycle state. Sensitive windows are hidden; unlock never resumes capture or delivery.
- `main` and `overlay` use separate explicit capability files. The overlay can only listen/unlisten to lifecycle events, while every application command also validates the caller label in Rust.
- Production CSP permits bundled assets and Tauri IPC only; remote frames/navigation, objects, forms, inline script/style and eval are absent.

Step 7 does not register the hotkey or access the microphone. Step 8 owns that integration and must connect capture to the existing safety transition.

## Step 13 recovery boundary

- The main window can list the bounded session aggregate and invoke Retry, Copy audit, Resolve, Pin/Unpin, or Delete. Every command re-authorizes the caller label in Rust.
- Retry is only a user action: the UI warns about duplicate risk, Rust captures a new foreground target, and the existing evidence-first insertion ladder records a new operation. Nothing retries on startup or after an uncertain outcome.
- Startup resumes running deletion journals and applies deterministic retention before writer services start.
- Step 14 replaces the minimal surface with the final M1 overlay/settings/recovery UX while preserving the Step 13 data boundary.

## Step 14 overlay and settings boundary

- The 420×88 overlay is render-only and receives content-free lifecycle events. Rust applies `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`, uses the foreground monitor work area with `SWP_NOACTIVATE`, and auto-hides only terminal evidence states.
- `Delivered`, `Uncertain`, and `Error` are distinct. A transport-only or interrupted insertion can never publish success.
- Tray navigation is explicit; `--startup` keeps the main window hidden. User-level startup uses an exact HKCU Run command and never requests elevation.
- Settings update one immutable configuration snapshot with optimistic versioning and last-known-good preservation. Hotkey/startup live state is restored if coordinated application fails.
- Retry/Delete dialogs are keyboard-accessible and explicit about duplicate/destructive risk; raw/evidence remain opt-in disclosures.

## Step 15 diagnostics and offline boundary

- `DiagnosticService` is owned by the privileged main process. It writes essential content-free lifecycle/failure events and enables expanded progress only through the persisted diagnostic opt-in.
- Main-window commands expose only status, deterministic manifest preview, and explicit export. Unknown fields, oversized paths, stale preview ids, missing exact confirmation, non-absolute/non-`.wigigadiag.json` destinations, reparse points, and overwrite attempts fail closed.
- Capture/ASR/delivery traces distinguish commit, focus/target check, retry, delivered, uncertain, and failed outcomes without audio, transcript, clipboard, window title, environment, or full path values.
- `scripts/offline-audit.ps1` combines a static network boundary inventory with a dead-proxy/offline process harness and marker-secret assertion; it is part of the canonical quality gate.

## Verification

```powershell
cargo test -p wigigadict-desktop --all-targets --locked --offline

## Step 16 golden-flow gate (in progress)

- A 100-session integration test connects durable PCM commit, ASR lease/crash/re-lease, immutable raw, deterministic cleanup, initial target capture, production insertion evidence policy, and recovery projection on one persistent database.
- The automated matrix produces 80 `target_ack → delivered` and 20 `transport_only → uncertain` fixture outcomes, retains 100 physical WAV files and 100 raw transcripts, and asserts zero irrecoverable sessions plus exactly ten same-attempt crash retries.
- Frozen thresholds and the content-free evaluator live under `tests/golden-flow/`; malformed, incomplete, duplicate, false-delivered, wrong-OS/runtime, quality, latency, and resource evidence fails closed.
- This automation does not certify a real target. Step 16 remains open until the owner completes 50 Codex and 50 VS Code dictations on Windows 10 build 19045 and the aggregate evaluator reports `passed=true`.

cargo clippy -p wigigadict-desktop --all-targets --locked --offline -- -D warnings
pwsh -NoProfile -File .\scripts\quality.ps1
```

The desktop Rust suite currently covers 34 shell/capture/insertion/overlay/settings/diagnostic/golden-flow safety cases; frontend suites add 16 unit and 3 integration checks plus strict lint/typecheck and production build verification.
