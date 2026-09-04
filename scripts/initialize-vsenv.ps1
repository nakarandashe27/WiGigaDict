function Initialize-VsDevEnvironment {
  $vsDevCmdCandidates = @("C:\BuildTools\Common7\Tools\VsDevCmd.bat")
  $vsWhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  if (Test-Path -LiteralPath $vsWhere) {
    $installationPath = & $vsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($installationPath) {
      $vsDevCmdCandidates += Join-Path $installationPath "Common7\Tools\VsDevCmd.bat"
    }
  }
  $vsDevCmd = $vsDevCmdCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
  if (-not $vsDevCmd) {
    throw "Visual Studio Developer Command Prompt was not found. Install VS 2022 Build Tools with the C++ workload."
  }

  $environmentLines = cmd.exe /d /s /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 && set"
  foreach ($line in $environmentLines) {
    if ($line -match "^([^=]+)=(.*)$") {
      [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], "Process")
    }
  }

  $cargoBin = if ($env:CARGO_HOME) {
    Join-Path $env:CARGO_HOME "bin"
  }
  else {
    Join-Path $env:USERPROFILE ".cargo\bin"
  }
  if (-not (Test-Path -LiteralPath $cargoBin)) {
    throw "Rustup cargo bin directory not found at $cargoBin. Install the pinned Rust toolchain first."
  }
  $env:Path = "$cargoBin;$env:Path"
}
