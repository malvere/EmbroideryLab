param(
    [string]$Version,
    [string]$OutputDirectory,
    [string]$DocumentationDirectory,
    [string]$SourceCommit,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
$crateRoot = Join-Path $taskRoot 'embroidery-engine'
if (-not $Version) {
    $manifest = Get-Content -LiteralPath (Join-Path $crateRoot 'Cargo.toml') -Raw
    $Version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-.][A-Za-z0-9.]+)?$') { throw 'Invalid release version.' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $taskRoot "outputs\releases\v$Version" }
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $taskRoot $OutputDirectory }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $DocumentationDirectory) {
    $publicDocs = Join-Path $taskRoot 'publication\EmbroideryLab'
    $DocumentationDirectory = if (Test-Path -LiteralPath (Join-Path $publicDocs 'README.md')) { $publicDocs } else { $taskRoot }
}
foreach ($document in @('LICENSE', 'README.md')) {
    if (-not (Test-Path -LiteralPath (Join-Path $DocumentationDirectory $document) -PathType Leaf)) {
        throw "Prepare $document in $DocumentationDirectory before packaging."
    }
}
if (-not $SourceCommit) {
    $SourceCommit = (& git -C $DocumentationDirectory rev-parse HEAD 2>$null | Select-Object -First 1)
    if (-not $SourceCommit) { throw 'Pass -SourceCommit with the published source revision.' }
}

$priorPath = $env:Path
$priorFlags = $env:CARGO_ENCODED_RUSTFLAGS
$cargoPath = Join-Path $env:USERPROFILE '.cargo\bin'
$gccPath = Join-Path $taskRoot 'tools\winlibs\mingw64\bin'
$availableTools = @($cargoPath, $gccPath) | Where-Object { Test-Path -LiteralPath $_ -PathType Container }
$env:Path = ($availableTools + @($env:Path)) -join ';'
$env:CARGO_ENCODED_RUSTFLAGS = @("--remap-path-prefix=$taskRoot=.", '-Clink-arg=-Wl,--no-insert-timestamp') -join [char]31
$inspectionCandidates = @(
    (Join-Path $taskRoot 'tools\llvm-mingw-20261020-ucrt-x86_64\bin\llvm-readobj.exe'),
    (Join-Path $gccPath 'objdump.exe')
)
foreach ($commandName in @('llvm-readobj.exe', 'objdump.exe')) {
    $command = Get-Command $commandName -ErrorAction SilentlyContinue
    if ($command) { $inspectionCandidates += $command.Source }
}
$inspectionTool = $inspectionCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $inspectionTool) {
    $env:Path = $priorPath
    $env:CARGO_ENCODED_RUSTFLAGS = $priorFlags
    throw 'Install LLVM llvm-readobj or MinGW objdump to audit release imports.'
}

function Get-ImportedDlls([string]$Path) {
    $isLlvm = [IO.Path]::GetFileName($inspectionTool) -match '^llvm-readobj'
    $arguments = if ($isLlvm) { @('--coff-imports', $Path) } else { @('-p', $Path) }
    $output = & $inspectionTool @arguments
    if ($LASTEXITCODE -ne 0) { throw "Import inspection failed: $Path" }
    @($output | ForEach-Object {
        if ($_ -match '(?:^\s*Name:|DLL Name:)\s*(\S+\.dll)') { $Matches[1].ToLowerInvariant() }
    } | Sort-Object -Unique)
}

function Test-SystemDll([string]$Name) {
    $Name -match '^(api-ms-win-|ext-ms-win-)' -or (Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32\$Name") -PathType Leaf)
}

function Copy-DependencyLicenses($Metadata, [string]$Destination) {
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    $resolved = @{}
    foreach ($node in $Metadata.resolve.nodes) { $resolved[$node.id] = $true }
    $index = @()
    foreach ($package in $Metadata.packages | Sort-Object name, version) {
        if (-not $resolved.ContainsKey($package.id) -or $package.name -eq 'embroidery-lab') { continue }
        $sourceDirectory = Split-Path $package.manifest_path -Parent
        $legalFiles = @(Get-ChildItem -LiteralPath $sourceDirectory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE|UNLICENSE)' })
        if ($package.license_file) {
            $licensePath = if ([IO.Path]::IsPathRooted($package.license_file)) { $package.license_file } else { Join-Path $sourceDirectory $package.license_file }
            if (Test-Path -LiteralPath $licensePath -PathType Leaf) { $legalFiles += Get-Item -LiteralPath $licensePath }
        }
        foreach ($subdirectory in @('fonts', 'licenses', 'LICENSES', 'license', 'third_party')) {
            $candidate = Join-Path $sourceDirectory $subdirectory
            if (Test-Path -LiteralPath $candidate -PathType Container) {
                $legalFiles += @(Get-ChildItem -LiteralPath $candidate -File -Recurse | Where-Object {
                    $_.Name -match '^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE|UNLICENSE|OFL\.|UFL\.)' -or ($subdirectory -eq 'fonts' -and $_.Extension -eq '.txt')
                })
            }
        }
        $copied = @()
        foreach ($file in $legalFiles | Sort-Object FullName -Unique) {
            $sourcePrefix = [IO.Path]::GetFullPath($sourceDirectory).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
            if (-not $file.FullName.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "License file lies outside its dependency directory: $($package.name)"
            }
            $relative = $file.FullName.Substring($sourceDirectory.Length).TrimStart('\','/')
            $destinationFile = Join-Path $Destination (Join-Path "$($package.name)-$($package.version)" $relative)
            New-Item -ItemType Directory -Force -Path (Split-Path $destinationFile -Parent) | Out-Null
            Copy-Item -LiteralPath $file.FullName -Destination $destinationFile
            $copied += $relative.Replace('\','/')
        }
        $index += [ordered]@{ name=$package.name; version=$package.version; license=$package.license; authors=@($package.authors); repository=$package.repository; bundled_license_files=$copied }
    }
    $index | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Destination 'INDEX.json') -Encoding utf8
}

$temporaryStage = $null
try {
    $pdfium = & (Join-Path $PSScriptRoot 'setup-pdfium.ps1') -Profiles release
    Push-Location $crateRoot
    try {
        $compiler = (& rustc --version --verbose) -join "`n"
        if ($LASTEXITCODE -ne 0 -or $compiler -notmatch '(?m)^host: x86_64-pc-windows-gnu\s*$') {
            throw 'Packaging requires the x86_64-pc-windows-gnu Rust toolchain and MinGW GCC on PATH.'
        }
        $gccCommand = Get-Command gcc.exe -ErrorAction Stop
        $runtimeDirectory = Split-Path $gccCommand.Source -Parent
        $rustSysroot = (& rustc --print sysroot | Select-Object -First 1)
        if (-not $SkipBuild) {
            & cargo build --locked --release --bins --no-default-features --features gui --jobs 2
            if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
        }
        $metadataText = & cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-gnu --no-default-features --features gui
        if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata failed.' }
        $metadata = $metadataText | ConvertFrom-Json
    } finally { Pop-Location }

    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $temporaryStage = Join-Path $OutputDirectory ('.package-' + [guid]::NewGuid().ToString('N'))
    $archiveBase = "EmbroideryLab-$Version-windows-x64"
    $stage = Join-Path $temporaryStage $archiveBase
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    $imports = [ordered]@{}
    $runtimeDlls = @{}
    # PDFium is loaded dynamically, so it is not listed in either executable's
    # PE imports. Bundle and audit it explicitly using the pinned setup result.
    Copy-Item -LiteralPath $pdfium.LibraryPath -Destination (Join-Path $stage 'pdfium.dll')
    $runtimeDlls['pdfium.dll'] = @(Get-ImportedDlls $pdfium.LibraryPath)
    foreach ($dependency in $runtimeDlls['pdfium.dll']) {
        if (-not (Test-SystemDll $dependency)) { throw "Unreviewed PDFium dependency $dependency; do not publish this package." }
    }
    foreach ($binary in @('embroidery-lab.exe', 'embroidery-cli.exe')) {
        $source = Join-Path $crateRoot "target\release\$binary"
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Release binary missing: $source" }
        Copy-Item -LiteralPath $source -Destination (Join-Path $stage $binary)
        $imports[$binary] = @(Get-ImportedDlls $source)
        $queue = [Collections.Generic.Queue[string]]::new()
        foreach ($dll in $imports[$binary]) { $queue.Enqueue($dll) }
        while ($queue.Count -gt 0) {
            $dll = $queue.Dequeue()
            if ($runtimeDlls.ContainsKey($dll)) { continue }
            if ($dll -match '^(libgcc_s_seh-1|libstdc\+\+-6|libwinpthread-1)\.dll$') {
                $runtimePath = Join-Path $runtimeDirectory $dll
                if (-not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) { throw "Required compiler runtime missing: $runtimePath" }
                Copy-Item -LiteralPath $runtimePath -Destination (Join-Path $stage $dll)
                $runtimeDlls[$dll] = @(Get-ImportedDlls $runtimePath)
                foreach ($dependency in $runtimeDlls[$dll]) { $queue.Enqueue($dependency) }
            } elseif (-not (Test-SystemDll $dll)) {
                throw "Unreviewed non-system dependency $dll; do not publish this package."
            }
        }
    }
    Copy-Item -LiteralPath (Join-Path $DocumentationDirectory 'README.md') -Destination (Join-Path $stage 'README.md')
    Copy-Item -LiteralPath (Join-Path $DocumentationDirectory 'LICENSE') -Destination (Join-Path $stage 'LICENSE')
    foreach ($relativeDocument in @('BUILDING.md', 'CHANGELOG.md', 'VALIDATION.md', 'embroidery-engine\README.md')) {
        $documentPath = Join-Path $DocumentationDirectory $relativeDocument
        if (Test-Path -LiteralPath $documentPath -PathType Leaf) {
            $destinationPath = Join-Path $stage $relativeDocument
            New-Item -ItemType Directory -Force -Path (Split-Path $destinationPath -Parent) | Out-Null
            Copy-Item -LiteralPath $documentPath -Destination $destinationPath
        }
    }
    Copy-DependencyLicenses $metadata (Join-Path $stage 'THIRD_PARTY_LICENSES')
    $pdfiumLegalDestination = Join-Path $stage "THIRD_PARTY_LICENSES\pdfium-$($pdfium.Build)"
    New-Item -ItemType Directory -Force -Path $pdfiumLegalDestination | Out-Null
    Copy-Item -LiteralPath (Join-Path $pdfium.LicenseDirectory 'LICENSE') -Destination (Join-Path $pdfiumLegalDestination 'LICENSE')
    Copy-Item -LiteralPath (Join-Path $pdfium.LicenseDirectory 'licenses') -Destination $pdfiumLegalDestination -Recurse
    $pdfium | Select-Object Build, ArchiveUrl, ArchiveSha256, LibrarySha256 | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $pdfiumLegalDestination 'PROVENANCE.json') -Encoding utf8
    $rustLegalRoot = Join-Path $rustSysroot 'share\doc\rust'
    $rustLegalDestination = Join-Path $stage 'THIRD_PARTY_LICENSES\rust-standard-library'
    $rustLibraryCopyright = Join-Path $rustLegalRoot 'COPYRIGHT-library.html'
    if (Test-Path -LiteralPath $rustLibraryCopyright -PathType Leaf) {
        New-Item -ItemType Directory -Force -Path $rustLegalDestination | Out-Null
        Copy-Item -LiteralPath $rustLibraryCopyright -Destination (Join-Path $rustLegalDestination 'COPYRIGHT-library.html')
        $rustLicenseDirectory = Join-Path $rustLegalRoot 'licenses'
        if (Test-Path -LiteralPath $rustLicenseDirectory -PathType Container) {
            $rustLicenseDestination = Join-Path $rustLegalDestination 'licenses'
            New-Item -ItemType Directory -Force -Path $rustLicenseDestination | Out-Null
            Get-ChildItem -LiteralPath $rustLicenseDirectory -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $rustLicenseDestination $_.Name) }
        }
    }
    $buildInfo = [ordered]@{ version=$Version; source_commit=$SourceCommit; target='x86_64-pc-windows-gnu'; features=@('gui'); capture_feature=$false; compiler=$compiler; imports=$imports; bundled_runtime_dlls=@($runtimeDlls.Keys | Sort-Object); pdfium_build=$pdfium.Build; pdfium_sha256=$pdfium.LibrarySha256 }
    $buildInfo | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $stage 'BUILD-INFO.json') -Encoding utf8
    $buildInfo | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'build-info.json') -Encoding utf8
    # Normalize archive timestamps; this also avoids including local extraction
    # times from the Cargo cache. The commit itself is recorded in BUILD-INFO.
    $archiveTimestamp = [datetime]::new(2026, 1, 1, 0, 0, 0, [DateTimeKind]::Utc)
    Get-ChildItem -LiteralPath $stage -Recurse -File | ForEach-Object { $_.LastWriteTimeUtc = $archiveTimestamp }
    $archive = Join-Path $OutputDirectory "$archiveBase.zip"
    Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal -Force
    Copy-Item -LiteralPath (Join-Path $stage 'embroidery-lab.exe') -Destination (Join-Path $OutputDirectory "$archiveBase.exe")
    Copy-Item -LiteralPath (Join-Path $stage 'embroidery-cli.exe') -Destination (Join-Path $OutputDirectory "embroidery-cli-$Version-windows-x64.exe")
    Copy-Item -LiteralPath (Join-Path $stage 'pdfium.dll') -Destination (Join-Path $OutputDirectory 'pdfium.dll')
    $loosePdfiumLicenses = Join-Path $OutputDirectory 'pdfium-licenses'
    New-Item -ItemType Directory -Force -Path $loosePdfiumLicenses | Out-Null
    Copy-Item -LiteralPath (Join-Path $pdfium.LicenseDirectory 'LICENSE') -Destination (Join-Path $loosePdfiumLicenses 'LICENSE')
    Copy-Item -LiteralPath (Join-Path $pdfium.LicenseDirectory 'licenses') -Destination $loosePdfiumLicenses -Recurse -Force
    $pdfiumLegalArchive = Join-Path $OutputDirectory 'pdfium-licenses.zip'
    Compress-Archive -LiteralPath $loosePdfiumLicenses -DestinationPath $pdfiumLegalArchive -CompressionLevel Optimal -Force
    $assets = @($archive, (Join-Path $OutputDirectory "$archiveBase.exe"), (Join-Path $OutputDirectory "embroidery-cli-$Version-windows-x64.exe"), (Join-Path $OutputDirectory 'pdfium.dll'), $pdfiumLegalArchive)
    $checksums = foreach ($asset in $assets | Sort-Object) { "{0}  {1}" -f (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant(), [IO.Path]::GetFileName($asset) }
    $checksums | Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS.txt') -Encoding ascii
    foreach ($asset in $assets) { [pscustomobject]@{file=$asset; bytes=(Get-Item -LiteralPath $asset).Length; sha256=(Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()} }
} finally {
    $env:Path = $priorPath
    $env:CARGO_ENCODED_RUSTFLAGS = $priorFlags
    if ($temporaryStage -and (Test-Path -LiteralPath $temporaryStage)) {
        $resolvedStage = [IO.Path]::GetFullPath($temporaryStage)
        $expectedRoot = $OutputDirectory.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
        if (-not $resolvedStage.StartsWith($expectedRoot, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolvedStage) -notmatch '^\.package-[0-9a-f]{32}$') {
            throw 'Refusing to remove an unexpected packaging directory.'
        }
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
}
