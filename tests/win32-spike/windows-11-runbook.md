# Windows 11 Step 3 acceptance runbook

Run only in a disposable Windows 11 x64 VM or runner with an unlocked interactive desktop. Use empty repository-owned targets; never use a live prompt, user document, password field or elevated target. Copy the resulting content-free JSON reports back to `tests/win32-spike/evidence/` only after manual observer review.

1. Record Windows edition, display version, build and architecture.
2. Run the standard control matrix:

   ```powershell
   pwsh -NoProfile -File .\scripts\win32-spike.ps1 -ReportPath artifacts/win32-spike/windows-11-standard-controls.json
   ```

3. Run the real Tauri/WebView2 overlay. After the window appears, wait one second, switch away and focus `WiGigaDict` once:

   ```powershell
   pwsh -NoProfile -File .\scripts\tauri-overlay-spike.ps1 -ReportPath artifacts/win32-spike/windows-11-tauri-webview2.json
   ```

4. Open an empty VS Code editor with extensions disabled. Focus Monaco and run:

   ```powershell
   pwsh -NoProfile -File .\scripts\run-win32-external-spike.ps1 -Surface vscode_codex -ExpectedProcess Code.exe -ReportPath artifacts/win32-spike/windows-11-vscode.json
   ```

5. Open a disposable empty Claude Code prompt in Windows Terminal. Do not press Enter after the marker arrives. Start the probe, then focus the prompt within 60 seconds:

   ```powershell
   pwsh -NoProfile -File .\scripts\run-win32-external-spike.ps1 -Surface terminal_claude_code -ExpectedProcess WindowsTerminal.exe -ReportPath artifacts/win32-spike/windows-11-terminal.json
   ```

6. Serve `tests/win32-spike/fixtures/browser-target.html` on loopback, open it in an isolated browser profile, focus the empty field and run:

   ```powershell
   pwsh -NoProfile -File .\scripts\run-win32-external-spike.ps1 -Surface browser -ExpectedProcess chrome.exe -ReportPath artifacts/win32-spike/windows-11-browser.json
   ```

7. For every external row, visually confirm whether the deterministic marker appeared exactly once. Record only the boolean observer verdict, process/file version, window and focused-control classes, method, input-unit counts, foreground retention and clipboard restoration outcome. Do not record target text or screenshots.
8. Reject any row with a focus steal, target mismatch, partial input, retry after possible delivery, missing observer verdict or unreviewed report. A successful `SendInput` count remains `transport_only -> uncertain`; it is not delivery acknowledgement by itself.
