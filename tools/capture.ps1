param([string]$Output = 'outputs/gui/editor.png', [string]$File, [string]$Size)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
$taskOutput = if ([IO.Path]::IsPathRooted($Output)) { $Output } else { Join-Path $taskRoot $Output }
New-Item -ItemType Directory -Force -Path (Split-Path $taskOutput -Parent) | Out-Null
$taskPriorCapture = $env:EMBROIDERYLAB_CAPTURE_TO
$taskPriorSize = $env:EMBROIDERYLAB_CAPTURE_SIZE
$env:EMBROIDERYLAB_CAPTURE_TO = $taskOutput
$env:EMBROIDERYLAB_CAPTURE_SIZE = $Size
try {
    $taskExe = Join-Path $taskRoot 'embroidery-engine\target\debug\embroidery-lab.exe'
    if (-not (Test-Path -LiteralPath $taskExe -PathType Leaf)) { throw 'Build with cargo build --features capture --bins first.' }
    if ($File) { & $taskExe $File } else { & $taskExe }
    if ($LASTEXITCODE -ne 0) { throw 'Capture failed. Build with cargo build --features capture --bins first.' }
    if (-not (Test-Path -LiteralPath $taskOutput)) { throw 'No framebuffer capture was produced.' }
} finally { $env:EMBROIDERYLAB_CAPTURE_TO = $taskPriorCapture; $env:EMBROIDERYLAB_CAPTURE_SIZE = $taskPriorSize }
