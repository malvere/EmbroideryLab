param([string]$File, [switch]$Release)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
$taskProfile = if ($Release) { 'release' } else { 'debug' }
$taskExe = Join-Path $taskRoot "embroidery-engine\target\$taskProfile\embroidery-lab.exe"
if (-not (Test-Path -LiteralPath $taskExe)) { throw 'Build first with tools/build.ps1' }
if ($File) { & $taskExe $File } else { & $taskExe }
