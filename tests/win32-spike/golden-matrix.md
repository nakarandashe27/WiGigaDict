# M0 Step 3 Win32 golden matrix

The harness records no target text, screenshots, full local paths, or clipboard payloads. A green transport call is not delivery: external application rows remain `uncertain` until an exact process version/window-control/method rule has passed the full matrix.

| OS | Surface | Hotkey down/up | Overlay focus | Unicode packet | Virtual-key `SendInput` | Clipboard | Evidence / state |
|---|---|---|---|---|---|---|---|
| Windows 10 22H2 build 19045.6456 | Standard Win32 `EDIT` fixture | Pass (injected deterministic replay; repeat ignored) | Pass, 0 focus steals / 100 cycles | `target_ack` | `transport_only` → `uncertain` | busy/restore failures → `uncertain` | Automated evidence committed below |
| Windows 10 22H2 | VS Code 1.134.0 isolated editor, extensions disabled | Covered by shared deterministic harness | Pass, 0 focus steals / 100 cycles | `40/40 transport_only` → `uncertain`; title observer did not acknowledge | Not attempted after possible delivery | Not attempted after possible delivery | Completed negative verdict; no compatibility rule active |
| Windows 10 22H2 | Windows Terminal / Claude Code 2.1.239 disposable prompt | Covered by shared deterministic harness | Probe reached insertion only after its zero-steal precondition; post-run JSON was absent | Manual observer saw corrupted glyphs instead of the exact marker → `uncertain` | Not attempted after possible delivery | Not attempted after possible delivery | Completed negative manual verdict; no compatibility rule active |
| Windows 10 22H2 | Chrome 151.0.7922.173 isolated loopback text field | Covered by shared deterministic harness | Pass, 0 focus steals / 100 cycles | `40/40 transport_only` + exact fixture observer acknowledgement | Not attempted after possible delivery | Not attempted after possible delivery | Pass for the fixture; no broad compatibility rule active |
| Windows 11 x64 | All surfaces | Deferred to BL-047 | Deferred | Deferred | Deferred | Deferred | Not tested and not claimed compatible; see ADR-005 |

Committed evidence: [`evidence/windows-10-19045-standard-controls.json`](evidence/windows-10-19045-standard-controls.json), [`evidence/windows-10-19045-tauri-webview2-overlay.json`](evidence/windows-10-19045-tauri-webview2-overlay.json), [`evidence/windows-10-19045-vscode-1.134.0.json`](evidence/windows-10-19045-vscode-1.134.0.json), [`evidence/windows-10-19045-chrome-151.0.7922.173.json`](evidence/windows-10-19045-chrome-151.0.7922.173.json) and [`evidence/windows-10-19045-terminal-claude-code-2.1.239.json`](evidence/windows-10-19045-terminal-claude-code-2.1.239.json).

The real Tauri/WRY WebView2 overlay acceptance path passed on Windows 10 after an explicit armed foreground transition: required styles present, `0` target mismatches and `0` focus steals across 100 cycles. This does not substitute for the external application rows.

## Manual acceptance protocol

1. Use a disposable empty document or field and a deterministic non-sensitive marker. Never use a live prompt, chat, form submission, password field, elevated app, or document containing user data.
2. Record OS build, exact process/file version, top-level and focused control classes, method, expected/accepted input units, foreground identity before/after, clipboard restoration outcome, and observer verdict. Do not record the field contents.
3. Run key-down, repeat-key-down and key-up; confirm exactly one admission and one finalization.
4. Show/hide the overlay at least 100 times while the target remains foreground. Any focus change fails the row.
5. Change foreground between key-up and insertion, destroy the captured HWND, use an elevated fixture, force partial input, hold the clipboard open in another thread/process, and force restoration failure. Every row must end `uncertain`, preserve the source text, and perform no automatic retry.
6. A successful `SendInput` count alone is `transport_only`. Activate a `certified_transport` rule only for the exact versioned combination after its positive and negative cells pass on every OS for which compatibility is claimed. Windows 11 is deferred by ADR-005 and is not currently claimed.
