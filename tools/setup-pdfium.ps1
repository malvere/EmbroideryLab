param([ValidateSet('debug', 'release')][string[]]$Profiles = @('debug', 'release'))
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or -not [Environment]::Is64BitOperatingSystem) {
    throw 'This setup script provisions PDFium for Windows x64. Install the matching PDFium library on other platforms.'
}

# Pin both the verified release archive and its extracted library. Cached and
# already deployed DLLs are verified against these constants, not a local manifest.
$pdfiumBuild = '8125'
$archiveHash = '9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b'
$libraryHash = '4b0dec92555d69437bef0df37be630181ab3e83a6823ca264ede355a4f2ef994'
$archiveUrl = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/$pdfiumBuild/pdfium-win-x64.tgz"
$cacheDirectory = Join-Path $taskRoot "binaries\pdfium\chromium-$pdfiumBuild"
$archivePath = Join-Path $cacheDirectory 'pdfium-win-x64.tgz'
$runtimeRoot = Join-Path $cacheDirectory 'runtime'
$libraryPath = Join-Path $runtimeRoot 'bin\pdfium.dll'
New-Item -ItemType Directory -Force -Path $cacheDirectory | Out-Null

function Test-ExpectedHash([string]$Path, [string]$Expected) {
    (Test-Path -LiteralPath $Path -PathType Leaf) -and
        (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.Equals($Expected, [StringComparison]::OrdinalIgnoreCase)
}

if (-not (Test-ExpectedHash $archivePath $archiveHash)) {
    $downloadPath = Join-Path $cacheDirectory ('.download-' + [guid]::NewGuid().ToString('N'))
    try {
        Invoke-WebRequest -Uri $archiveUrl -OutFile $downloadPath
        if (-not (Test-ExpectedHash $downloadPath $archiveHash)) {
            throw 'PDFium archive SHA256 mismatch; the runtime was not installed.'
        }
        Move-Item -LiteralPath $downloadPath -Destination $archivePath -Force
    } finally {
        if (Test-Path -LiteralPath $downloadPath) { Remove-Item -LiteralPath $downloadPath -Force }
    }
}
if (-not (Test-ExpectedHash $libraryPath $libraryHash) -or
    -not (Test-Path -LiteralPath (Join-Path $runtimeRoot 'LICENSE') -PathType Leaf) -or
    -not (Test-Path -LiteralPath (Join-Path $runtimeRoot 'licenses') -PathType Container)) {
    $entries = @(& tar -tzf $archivePath)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect the verified PDFium archive; install tar.' }
    foreach ($entry in $entries) {
        if ($entry -match '^[\\/]' -or $entry -match ':' -or $entry -split '[\\/]' -contains '..') {
            throw "Unexpected path in PDFium archive: $entry"
        }
    }
    New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
    & tar -xzf $archivePath -C $runtimeRoot
    if ($LASTEXITCODE -ne 0) { throw 'Cannot extract the verified PDFium runtime.' }
}
if (-not (Test-ExpectedHash $libraryPath $libraryHash)) { throw 'PDFium library SHA256 mismatch.' }
$licenseFiles = @((Get-Item -LiteralPath (Join-Path $runtimeRoot 'LICENSE')))
$licenseFiles += @(Get-ChildItem -LiteralPath (Join-Path $runtimeRoot 'licenses') -Recurse -File)
if ($licenseFiles.Count -lt 2) { throw 'PDFium third-party licenses are missing.' }

foreach ($profile in $Profiles | Select-Object -Unique) {
    $destination = Join-Path $taskRoot "embroidery-engine\target\$profile"
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    $destinationLibrary = Join-Path $destination 'pdfium.dll'
    if (-not (Test-ExpectedHash $destinationLibrary $libraryHash)) {
        Copy-Item -LiteralPath $libraryPath -Destination $destinationLibrary -Force
    }
    $legalDestination = Join-Path $destination 'pdfium-licenses'
    foreach ($file in $licenseFiles) {
        $relative = $file.FullName.Substring($runtimeRoot.Length).TrimStart('\', '/')
        $legalFile = Join-Path $legalDestination $relative
        New-Item -ItemType Directory -Force -Path (Split-Path $legalFile -Parent) | Out-Null
        $expected = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        if (-not (Test-ExpectedHash $legalFile $expected)) { Copy-Item -LiteralPath $file.FullName -Destination $legalFile -Force }
    }
}

[pscustomobject]@{
    Build = $pdfiumBuild
    ArchiveUrl = $archiveUrl
    ArchiveSha256 = $archiveHash
    LibraryPath = $libraryPath
    LibrarySha256 = $libraryHash
    LicenseDirectory = $runtimeRoot
    Profiles = $Profiles
}
