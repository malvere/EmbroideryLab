param(
    [Alias('Profile')]
    [ValidateSet('debug', 'release')]
    [string]$BuildProfile = 'release',
    [string]$OutputDirectory
)

# Prepare a local runnable directory from an already completed build. This
# script never compiles, deletes, archives, tags, pushes, or publishes anything.
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$taskPathSeparators = [char[]]@([char]92, [char]47)
$taskCrateRoot = Join-Path $taskRoot 'embroidery-engine'
$taskOutputRoot = [IO.Path]::GetFullPath((Join-Path $taskRoot 'outputs\autodigitize'))
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $taskOutputRoot 'app' }
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $taskRoot $OutputDirectory }
$taskDestination = [IO.Path]::GetFullPath($OutputDirectory)
$taskOutputPrefix = $taskOutputRoot.TrimEnd($taskPathSeparators) + [IO.Path]::DirectorySeparatorChar
if (-not $taskDestination.StartsWith($taskOutputPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The local app directory must be a child of outputs/autodigitize.'
}
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or -not [Environment]::Is64BitOperatingSystem) {
    throw 'This preparation script supports Windows x64 only.'
}

function Get-PinnedLiteral([string]$Text, [string]$VariableName) {
    $pattern = '(?m)^\$' + [regex]::Escape($VariableName) + '\s*=\s*''([^'']+)''\s*$'
    $matchesFound = [regex]::Matches($Text, $pattern)
    if ($matchesFound.Count -ne 1) { throw "Cannot read the pinned PDFium constant: $VariableName" }
    $matchesFound[0].Groups[1].Value
}

function Copy-LocalArtifact([string]$Source, [string]$RelativeDestination) {
    $destinationFile = [IO.Path]::GetFullPath((Join-Path $taskDestination $RelativeDestination))
    $destinationPrefix = $taskDestination.TrimEnd($taskPathSeparators) + [IO.Path]::DirectorySeparatorChar
    if (-not $destinationFile.StartsWith($destinationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Artifact destination leaves the local app directory: $RelativeDestination"
    }
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) { throw "Required local artifact is missing: $Source" }
    $sourceHash = (Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash.ToLowerInvariant()
    New-Item -ItemType Directory -Force -Path (Split-Path $destinationFile -Parent) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $destinationFile -Force
    $copiedHash = (Get-FileHash -LiteralPath $destinationFile -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($sourceHash -ne $copiedHash) { throw "Artifact changed while copying: $Source" }
}

function Get-ImportedDlls([string]$BinaryPath) {
    $inspectionOutput = @(& $taskObjdump '-p' $BinaryPath)
    if ($LASTEXITCODE -ne 0) { throw "PE import inspection failed: $BinaryPath" }
    if (-not ($inspectionOutput -match 'file format pei-x86-64')) {
        throw "Expected a Windows x64 PE artifact: $BinaryPath"
    }
    @($inspectionOutput | ForEach-Object {
        if ($_ -match 'DLL Name:') {
            if ($_ -notmatch '^\s*DLL Name:\s*([A-Za-z0-9_.+-]+\.dll)\s*$') {
                throw "Unrecognized DLL import entry in $BinaryPath"
            }
            $Matches[1].ToLowerInvariant()
        }
    } | Sort-Object -Unique)
}

function Test-SystemDll([string]$DllName) {
    $DllName -match '^(api-ms-win-|ext-ms-win-)' -or
        (Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32\$DllName") -PathType Leaf)
}

function Complete-DependencyNotices($Metadata, [string]$Destination, [string]$ReviewedNoticeRoot = (Join-Path $PSScriptRoot 'autodigitize-licenses')) {
    $supportRoot = [IO.Path]::GetFullPath($ReviewedNoticeRoot)
    $supportPrefix = $supportRoot.TrimEnd($taskPathSeparators) + [IO.Path]::DirectorySeparatorChar
    $manifestPath = Join-Path $supportRoot 'manifest.json'
    $support = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($support.schema -ne 1 -or $support.target -ne 'x86_64-pc-windows-gnu') {
        throw 'Unrecognized reviewed dependency-notice manifest.'
    }
    $copiedIndex = @(Get-Content -LiteralPath (Join-Path $Destination 'INDEX.json') -Raw | ConvertFrom-Json)
    $resolved = @{}
    foreach ($node in $Metadata.resolve.nodes) { $resolved[$node.id] = $true }
    $completeIndex = @()
    foreach ($package in $Metadata.packages | Sort-Object name, version) {
        if (-not $resolved.ContainsKey($package.id) -or $package.name -eq 'embroidery-lab') { continue }
        if (-not $package.license) { throw "Unresolved dependency license expression: $($package.name) $($package.version)" }
        $record = @($copiedIndex | Where-Object { $_.name -eq $package.name -and $_.version -eq $package.version })
        if ($record.Count -ne 1) { throw "Missing or duplicate dependency notice record: $($package.name) $($package.version)" }
        $sourceDirectory = [IO.Path]::GetFullPath((Split-Path $package.manifest_path -Parent))
        $sourcePrefix = $sourceDirectory.TrimEnd($taskPathSeparators) + [IO.Path]::DirectorySeparatorChar
        $packageDirectory = "$($package.name)-$($package.version)"
        $files = @($record[0].bundled_license_files)
        $provenance = @()
        foreach ($relative in $files) {
            $source = [IO.Path]::GetFullPath((Join-Path $sourceDirectory $relative))
            if (-not $source.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Dependency notice source leaves its package.' }
            $provenance += [ordered]@{
                file = $relative
                sha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
                source_url = "https://crates.io/api/v1/crates/$($package.name)/$($package.version)/download"
                source_path = $relative
                source_kind = 'Exact cached Cargo package notice'
            }
        }
        $reviewed = @($support.packages | Where-Object { $_.name -eq $package.name -and $_.version -eq $package.version })
        $selected = $null
        $commit = $null
        if ($reviewed.Count -gt 1) { throw "Duplicate reviewed notice entry: $packageDirectory" }
        if ($reviewed.Count -eq 1) {
            $review = $reviewed[0]
            if ($review.license_expression -ne $package.license -or $review.repository -ne $package.repository -or @($review.files).Count -eq 0) {
                throw "Reviewed notice metadata no longer matches the dependency: $packageDirectory"
            }
            $vcs = Get-Content -LiteralPath (Join-Path $sourceDirectory '.cargo_vcs_info.json') -Raw | ConvertFrom-Json
            if ($vcs.git.sha1 -ne $review.upstream_commit -or $review.upstream_commit -notmatch '^[0-9a-f]{40}$') {
                throw "Reviewed notice commit does not match the cached dependency: $packageDirectory"
            }
            $selected = $review.selected_license
            $commit = $review.upstream_commit
            foreach ($file in $review.files) {
                $source = [IO.Path]::GetFullPath((Join-Path $supportRoot $file.file))
                if (-not $source.StartsWith($supportPrefix, [StringComparison]::OrdinalIgnoreCase) -or
                    $file.sha256 -notmatch '^[0-9a-f]{64}$' -or
                    $file.source_url -notmatch ('^https://raw\.githubusercontent\.com/[^/]+/[^/]+/' + [regex]::Escape($commit) + '/')) {
                    throw "Invalid reviewed notice provenance: $packageDirectory"
                }
                if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $file.sha256) {
                    throw "Reviewed notice SHA256 mismatch: $($file.file)"
                }
                $noticeDestinationFile = [IO.Path]::GetFullPath((Join-Path (Join-Path $Destination $packageDirectory) $file.destination))
                $packagePrefix = [IO.Path]::GetFullPath((Join-Path $Destination $packageDirectory)).TrimEnd($taskPathSeparators) + [IO.Path]::DirectorySeparatorChar
                if (-not $noticeDestinationFile.StartsWith($packagePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Reviewed notice destination leaves its dependency.' }
                Copy-LocalArtifact $source "THIRD_PARTY_LICENSES\$packageDirectory\$($file.destination)"
                $files += $file.destination.Replace([char]92, [char]47)
                $provenance = @($provenance | Where-Object { $_.file -ne $file.destination })
                $provenance += [ordered]@{
                    file = $file.destination
                    sha256 = $file.sha256
                    source_url = $file.source_url
                    source_file_sha256 = $file.source_file_sha256
                    extraction = $file.extraction
                    source_kind = 'Reviewed upstream text at exact cached crate VCS commit'
                }
            }
        }
        $files = @($files | Sort-Object -Unique)
        # SPDX identifiers and author metadata alone are not full license text.
        # A changed dependency with no copied license now fails preparation.
        $primary = @($files | Where-Object { $_ -match '(^|/)(LICENSE|LICENCE|COPYING|UNLICENSE)([._-]|$)' })
        if ($primary.Count -eq 0) { throw "Unresolved full license text: $packageDirectory. Review its exact upstream revision before preparing this bundle." }
        if ($package.name -eq 'epaint_default_fonts') {
            foreach ($required in @('LICENSE-MIT', 'fonts/emoji-icon-font-mit-license.txt', 'fonts/Hack-Regular.txt', 'fonts/OFL.txt', 'fonts/UFL.txt')) {
                if ($required -notin $files) { throw "Embedded font notice missing: $required" }
            }
        }
        foreach ($file in $files) {
            $bundled = Join-Path (Join-Path $Destination $packageDirectory) $file
            if (-not (Test-Path -LiteralPath $bundled -PathType Leaf) -or (Get-Item -LiteralPath $bundled).Length -lt 80) {
                throw "Missing or empty full dependency notice: $packageDirectory/$file"
            }
            $expected = @($provenance | Where-Object { $_.file -eq $file } | Select-Object -Last 1)
            if ($expected.Count -ne 1 -or (Get-FileHash -LiteralPath $bundled -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected[0].sha256) {
                throw "Bundled dependency notice SHA256 mismatch: $packageDirectory/$file"
            }
        }
        $attribution = [ordered]@{
            name = $package.name; version = $package.version; license = $package.license
            selected_license = $selected; authors = @($package.authors); repository = $package.repository
            upstream_commit = $commit; bundled_license_files = $files; notice_provenance = $provenance
        }
        $attribution | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path (Join-Path $Destination $packageDirectory) 'ATTRIBUTION.json') -Encoding utf8
        $completeIndex += $attribution
    }
    $completeIndex | ConvertTo-Json -Depth 9 | Set-Content -LiteralPath (Join-Path $Destination 'INDEX.json') -Encoding utf8
    [ordered]@{
        resolved_dependencies = $completeIndex.Count
        reviewed_supplements = @($completeIndex | Where-Object { $_.upstream_commit }).Count
        unresolved_dependencies = 0
        reviewed_manifest_sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        network_used_during_preparation = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Destination 'COVERAGE.json') -Encoding utf8
}

$taskPriorPath = $env:Path
try {
    $taskCargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    $taskGccBin = Join-Path $taskRoot 'tools\winlibs\mingw64\bin'
    $taskToolPaths = @($taskCargoBin, $taskGccBin) | Where-Object { Test-Path -LiteralPath $_ -PathType Container }
    $env:Path = ($taskToolPaths + @($env:Path)) -join ';'
    $taskObjdump = (Get-Command objdump.exe -ErrorAction Stop).Source
    $taskRuntimeDirectory = Split-Path (Get-Command gcc.exe -ErrorAction Stop).Source -Parent
    $taskCompiler = @(& rustc --version --verbose)
    if ($LASTEXITCODE -ne 0 -or -not ($taskCompiler -match '^host: x86_64-pc-windows-gnu\s*$')) {
        throw 'The existing build must use the x86_64-pc-windows-gnu Rust toolchain.'
    }
    $taskRustSysroot = (& rustc --print sysroot | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate Rust standard-library notices.' }

    # Metadata resolves licenses for the Windows GUI dependency graph only.
    # It does not compile either executable or invoke the release packager.
    $taskMetadataText = @(& cargo metadata --locked --offline --format-version 1 `
        --manifest-path (Join-Path $taskCrateRoot 'Cargo.toml') `
        --filter-platform x86_64-pc-windows-gnu --no-default-features --features gui)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read cached Windows GUI dependency metadata.' }
    $taskMetadata = ($taskMetadataText -join "`n") | ConvertFrom-Json
    $taskBinaryDirectory = Join-Path $taskMetadata.target_directory $BuildProfile
    $taskBinaryNames = @('embroidery-lab.exe', 'embroidery-cli.exe')
    foreach ($taskBinary in $taskBinaryNames) {
        if (-not (Test-Path -LiteralPath (Join-Path $taskBinaryDirectory $taskBinary) -PathType Leaf)) {
            throw "Complete the requested $BuildProfile build first: $taskBinary"
        }
    }
    foreach ($taskDocument in @('LICENSE', 'PRODUCT.md', 'DESIGN.md', 'docs\workstation-usage.md', 'docs\workstation-validation.md', 'docs\autodigitize-design.md', 'docs\autodigitize-validation.md', 'docs\autodigitize-usage.md', 'docs\algorithm-completion.md', 'docs\image-import-compatibility.md', 'docs\svg-import.md')) {
        if (-not (Test-Path -LiteralPath (Join-Path $taskRoot $taskDocument) -PathType Leaf)) {
            throw "Required documentation is missing: $taskDocument"
        }
    }

    # Read the existing setup pins without running setup/download/deployment.
    $taskPdfiumSetup = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'setup-pdfium.ps1') -Raw
    $taskPdfiumBuild = Get-PinnedLiteral $taskPdfiumSetup 'pdfiumBuild'
    $taskPdfiumLibraryHash = Get-PinnedLiteral $taskPdfiumSetup 'libraryHash'
    $taskPdfiumArchiveHash = Get-PinnedLiteral $taskPdfiumSetup 'archiveHash'
    if ($taskPdfiumBuild -notmatch '^\d+$' -or $taskPdfiumLibraryHash -notmatch '^[0-9a-f]{64}$' -or $taskPdfiumArchiveHash -notmatch '^[0-9a-f]{64}$') {
        throw 'Invalid pinned PDFium build or SHA256 values.'
    }
    $taskPdfiumCache = Join-Path $taskRoot "binaries\pdfium\chromium-$taskPdfiumBuild"
    $taskPdfiumRuntime = Join-Path $taskPdfiumCache 'runtime'
    $taskPdfiumLibrary = Join-Path $taskPdfiumRuntime 'bin\pdfium.dll'
    $taskPdfiumArchive = Join-Path $taskPdfiumCache 'pdfium-win-x64.tgz'
    foreach ($taskPinnedFile in @(
        @{ Path = $taskPdfiumLibrary; Hash = $taskPdfiumLibraryHash },
        @{ Path = $taskPdfiumArchive; Hash = $taskPdfiumArchiveHash }
    )) {
        if (-not (Test-Path -LiteralPath $taskPinnedFile.Path -PathType Leaf) -or
            (Get-FileHash -LiteralPath $taskPinnedFile.Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $taskPinnedFile.Hash) {
            throw 'PDFium is missing or does not match setup-pdfium.ps1. Prepare the pinned runtime before creating the local app.'
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $taskPdfiumRuntime 'LICENSE') -PathType Leaf) -or
        -not (Test-Path -LiteralPath (Join-Path $taskPdfiumRuntime 'licenses') -PathType Container)) {
        throw 'Pinned PDFium notices are missing.'
    }
    $taskRustLegalRoot = Join-Path $taskRustSysroot 'share\doc\rust'
    if (-not (Test-Path -LiteralPath (Join-Path $taskRustLegalRoot 'COPYRIGHT-library.html') -PathType Leaf)) {
        throw 'Rust COPYRIGHT-library.html is missing; install the matching Rust documentation before creating the local app.'
    }

    $taskAllowedRuntime = @('libgcc_s_seh-1.dll', 'libstdc++-6.dll', 'libwinpthread-1.dll')
    if (Test-Path -LiteralPath $taskDestination -PathType Container) {
        foreach ($taskExistingBinary in Get-ChildItem -LiteralPath $taskDestination -File) {
            if (($taskExistingBinary.Extension -eq '.dll' -and $taskExistingBinary.Name -notin (@('pdfium.dll') + $taskAllowedRuntime)) -or
                ($taskExistingBinary.Extension -eq '.exe' -and $taskExistingBinary.Name -notin $taskBinaryNames)) {
                throw "Unreviewed existing binary in the app directory: $($taskExistingBinary.Name). Choose a fresh output directory; no files were deleted."
            }
        }
    }
    New-Item -ItemType Directory -Force -Path $taskDestination | Out-Null
    foreach ($taskBinary in $taskBinaryNames) { Copy-LocalArtifact (Join-Path $taskBinaryDirectory $taskBinary) $taskBinary }
    Copy-LocalArtifact $taskPdfiumLibrary 'pdfium.dll'

    $taskImports = [ordered]@{}
    $taskAudited = @{}
    $taskRuntimeDlls = [Collections.Generic.List[string]]::new()
    $taskQueue = [Collections.Generic.Queue[string]]::new()
    foreach ($taskBinary in (@('pdfium.dll') + $taskBinaryNames)) { $taskQueue.Enqueue($taskBinary) }
    while ($taskQueue.Count -gt 0) {
        $taskBinary = $taskQueue.Dequeue()
        if ($taskAudited.ContainsKey($taskBinary)) { continue }
        $taskAudited[$taskBinary] = $true
        $taskImports[$taskBinary] = @(Get-ImportedDlls (Join-Path $taskDestination $taskBinary))
        foreach ($taskDll in $taskImports[$taskBinary]) {
            if ($taskDll -in $taskAllowedRuntime) {
                if ($taskDll -notin $taskRuntimeDlls) {
                    Copy-LocalArtifact (Join-Path $taskRuntimeDirectory $taskDll) $taskDll
                    $taskRuntimeDlls.Add($taskDll)
                }
                $taskQueue.Enqueue($taskDll)
            } elseif (-not (Test-SystemDll $taskDll)) {
                throw "Unreviewed non-system dependency: $taskDll. The local directory has not been certified; no files were deleted."
            }
        }
    }
    if ($taskRuntimeDlls.Count -gt 0) {
        throw 'This build requires MinGW runtime DLLs whose matching distribution notices have not been reviewed. Retain the existing build, review those runtime notices, and prepare a fresh directory before claiming the bundle complete.'
    }

    # Import precisely one reviewed function from the existing packager's AST.
    # Dot-sourcing package-release.ps1 itself would execute release operations.
    $taskPackager = Join-Path $PSScriptRoot 'package-release.ps1'
    $taskParseTokens = $null
    $taskParseErrors = $null
    $taskPackagerAst = [Management.Automation.Language.Parser]::ParseFile($taskPackager, [ref]$taskParseTokens, [ref]$taskParseErrors)
    if ($taskParseErrors.Count -gt 0) { throw 'Cannot parse the existing dependency-license collector.' }
    $taskCollectors = @($taskPackagerAst.FindAll({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Copy-DependencyLicenses'
    }, $false))
    if ($taskCollectors.Count -ne 1) { throw 'Expected exactly one Copy-DependencyLicenses function in the existing packager.' }
    # The historical collector spells a literal separator as '\\'. PowerShell
    # does not use backslash escapes; normalize only that exact quoted literal
    # in the extracted function so its TrimEnd/TrimStart char arguments work.
    $taskDoubleSeparatorLiteral = "'" + [string]::new([char]92, 2) + "'"
    $taskSingleSeparatorLiteral = "'" + [char]92 + "'"
    $taskCollectorText = $taskCollectors[0].Extent.Text.Replace($taskDoubleSeparatorLiteral, $taskSingleSeparatorLiteral)
    . ([ScriptBlock]::Create($taskCollectorText))
    $taskLegalDestination = Join-Path $taskDestination 'THIRD_PARTY_LICENSES'
    Copy-DependencyLicenses $taskMetadata $taskLegalDestination
    Complete-DependencyNotices $taskMetadata $taskLegalDestination
    $taskLicenseIndex = Get-Content -LiteralPath (Join-Path $taskLegalDestination 'INDEX.json') -Raw | ConvertFrom-Json
    $taskFontNotice = @($taskLicenseIndex | Where-Object { $_.name -eq 'epaint_default_fonts' -and $_.bundled_license_files.Count -gt 0 })
    if ($taskFontNotice.Count -ne 1) { throw 'The dependency collector did not include embedded-font notices.' }

    $taskPdfiumLegal = Join-Path $taskLegalDestination "pdfium-$taskPdfiumBuild"
    New-Item -ItemType Directory -Force -Path $taskPdfiumLegal | Out-Null
    Copy-LocalArtifact (Join-Path $taskPdfiumRuntime 'LICENSE') "THIRD_PARTY_LICENSES\pdfium-$taskPdfiumBuild\LICENSE"
    foreach ($taskLicense in Get-ChildItem -LiteralPath (Join-Path $taskPdfiumRuntime 'licenses') -File -Recurse) {
        $taskRelative = $taskLicense.FullName.Substring($taskPdfiumRuntime.Length).TrimStart($taskPathSeparators)
        Copy-LocalArtifact $taskLicense.FullName "THIRD_PARTY_LICENSES\pdfium-$taskPdfiumBuild\$taskRelative"
    }
    [ordered]@{ Build = $taskPdfiumBuild; ArchiveSha256 = $taskPdfiumArchiveHash; LibrarySha256 = $taskPdfiumLibraryHash } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskPdfiumLegal 'PROVENANCE.json') -Encoding utf8
    Copy-LocalArtifact (Join-Path $taskRustLegalRoot 'COPYRIGHT-library.html') 'THIRD_PARTY_LICENSES\rust-standard-library\COPYRIGHT-library.html'
    $taskRustLicenses = Join-Path $taskRustLegalRoot 'licenses'
    if (Test-Path -LiteralPath $taskRustLicenses -PathType Container) {
        foreach ($taskLicense in Get-ChildItem -LiteralPath $taskRustLicenses -File -Recurse) {
            $taskRelative = $taskLicense.FullName.Substring($taskRustLicenses.Length).TrimStart($taskPathSeparators)
            Copy-LocalArtifact $taskLicense.FullName "THIRD_PARTY_LICENSES\rust-standard-library\licenses\$taskRelative"
        }
    }
    foreach ($taskDocument in @('LICENSE', 'PRODUCT.md', 'DESIGN.md', 'docs\workstation-usage.md', 'docs\workstation-validation.md', 'docs\autodigitize-design.md', 'docs\autodigitize-validation.md', 'docs\autodigitize-usage.md', 'docs\algorithm-completion.md', 'docs\image-import-compatibility.md', 'docs\svg-import.md')) {
        Copy-LocalArtifact (Join-Path $taskRoot $taskDocument) $taskDocument
    }

    $taskReadme = @"
# EmbroideryLab — локальная запускаемая сборка

Профиль: $BuildProfile. Запустите embroidery-lab.exe из этой папки.
embroidery-cli.exe — консольный инструмент. PDFium и нужные MinGW DLL находятся рядом.

Откройте PNG/JPEG, задайте размер в мм, при необходимости выделите внешний фон
и исключите его из оцифровки, затем нажмите «Сгенерировать вышивку».
Сохраните ELAB для дальнейшего редактирования; DST и список нитей служат для экспорта.

Рабочее место, темы, панели и команды: docs/workstation-usage.md.
Очистка и автооцифровка: docs/autodigitize-usage.md.
Проверка нового интерфейса: docs/workstation-validation.md.
Совместимость WebP/JPEG/PNG: docs/image-import-compatibility.md.
Векторный SVG, обводки и ограничения: docs/svg-import.md.
Реализованные ограничения: docs/autodigitize-design.md.
Завершение алгоритмов после повторной сверки: docs/algorithm-completion.md.
Фактические проверки: docs/autodigitize-validation.md.
build.json содержит SHA256 файлов, профиль, HEAD и признак незакоммиченных изменений.
В нём не утверждается, что параметры компиляции можно восстановить из готового EXE.

Это локальная папка, без архива, публикации или релиза. Проверка на ткани не заявлена.
Лицензия проекта — LICENSE; уведомления зависимостей — THIRD_PARTY_LICENSES.
"@
    Set-Content -LiteralPath (Join-Path $taskDestination 'README.md') -Value $taskReadme -Encoding utf8
    $taskHead = (& git -C $taskRoot rev-parse HEAD | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot record the local source HEAD.' }
    $taskStatus = @(& git -C $taskRoot status --porcelain=v1 --untracked-files=normal)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot record whether the working tree is dirty.' }
    $taskFiles = @(Get-ChildItem -LiteralPath $taskDestination -File -Recurse | Where-Object { $_.FullName -ne (Join-Path $taskDestination 'build.json') } | Sort-Object FullName | ForEach-Object {
        [ordered]@{ path = $_.FullName.Substring($taskDestination.Length).TrimStart($taskPathSeparators).Replace([char]92, [char]47); bytes = $_.Length; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $taskBuildInfo = [ordered]@{
        prepared_utc = [DateTime]::UtcNow.ToString('o')
        local_only = $true
        profile = $BuildProfile
        source_head = $taskHead
        source_dirty = ($taskStatus.Count -gt 0)
        target = 'x86_64-pc-windows-gnu'
        dependency_metadata_features = @('gui')
        executable_features = 'Not inferred from prebuilt executables; exact requested-profile files are copied.'
        compiler = ($taskCompiler -join "`n")
        binary_source_directory = $taskBinaryDirectory
        imports = $taskImports
        bundled_mingw_runtime_dlls = @($taskRuntimeDlls | Sort-Object)
        pdfium_build = $taskPdfiumBuild
        pdfium_sha256 = $taskPdfiumLibraryHash
        license_collector_source_sha256 = (Get-FileHash -LiteralPath $taskPackager -Algorithm SHA256).Hash.ToLowerInvariant()
        license_collector_path_literal_normalized = $true
        reviewed_notice_manifest_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'autodigitize-licenses\manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant()
        files = $taskFiles
    }
    $taskBuildInfo | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $taskDestination 'build.json') -Encoding utf8
    [pscustomobject]@{ Directory = $taskDestination; Profile = $BuildProfile; Executable = (Join-Path $taskDestination 'embroidery-lab.exe'); Files = $taskFiles.Count; BuildManifest = (Join-Path $taskDestination 'build.json') }
} finally {
    $env:Path = $taskPriorPath
}
