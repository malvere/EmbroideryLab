param([switch]$Headless, [switch]$Release)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
& (Join-Path $PSScriptRoot 'setup-pdfium.ps1') | Out-Null
$cargoPath = Join-Path $env:USERPROFILE '.cargo\bin'
$gccPath = Join-Path $taskRoot 'tools\winlibs\mingw64\bin'
$taskPriorPath = $env:Path
$taskToolPaths = @($cargoPath, $gccPath) | Where-Object { Test-Path -LiteralPath $_ -PathType Container }
$env:Path = ($taskToolPaths + @($env:Path)) -join ';'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $env:Path = $taskPriorPath
    throw 'Install Rust with rustup and add cargo to PATH.'
}
Push-Location (Join-Path $taskRoot 'embroidery-engine')
try {
    $cargoFlags = @()
    if ($Headless) { $cargoFlags += '--no-default-features' }
    & cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt failed' }
    & cargo test @cargoFlags
    if ($LASTEXITCODE -ne 0) { throw 'cargo test failed' }
    & cargo clippy @cargoFlags --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'cargo clippy failed' }
    $buildFlags = @($cargoFlags) + @('--locked', '--bins')
    if ($Release) { $buildFlags += '--release' }
    & cargo build @buildFlags
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally { Pop-Location; $env:Path = $taskPriorPath }
