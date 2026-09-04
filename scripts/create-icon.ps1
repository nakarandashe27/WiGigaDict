$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$iconDirectory = Join-Path $repoRoot "apps\desktop\src-tauri\icons"
$pngPath = Join-Path $iconDirectory "icon.generated.png"
$icoPath = Join-Path $iconDirectory "icon.ico"

Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force -Path $iconDirectory | Out-Null

$bitmap = New-Object System.Drawing.Bitmap 64, 64
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.Clear([System.Drawing.ColorTranslator]::FromHtml("#0b1220"))
$pen = New-Object System.Drawing.Pen ([System.Drawing.ColorTranslator]::FromHtml("#71e3ba")), 7
$pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$pen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
$points = [System.Drawing.PointF[]]@(
  (New-Object System.Drawing.PointF(14, 18)),
  (New-Object System.Drawing.PointF(22, 46)),
  (New-Object System.Drawing.PointF(32, 28)),
  (New-Object System.Drawing.PointF(42, 46)),
  (New-Object System.Drawing.PointF(50, 18))
)
$graphics.DrawLines($pen, $points)
$bitmap.Save($pngPath, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$pen.Dispose()
$bitmap.Dispose()

$png = [System.IO.File]::ReadAllBytes($pngPath)
$header = [byte[]](0, 0, 1, 0, 1, 0)
$entry = [byte[]](64, 64, 0, 0, 1, 0, 32, 0)
$entry += [BitConverter]::GetBytes([uint32]$png.Length)
$entry += [BitConverter]::GetBytes([uint32]22)
[System.IO.File]::WriteAllBytes($icoPath, $header + $entry + $png)
Remove-Item -LiteralPath $pngPath -Force
Write-Output "Generated technical icon: $icoPath"

