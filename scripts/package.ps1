param(
    [switch]$SkipBuild,
    [string]$SourceExe,
    [string]$Version = '0.1.0'
)

$ErrorActionPreference = 'Stop'
$snaplinePackageRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'windows-env.ps1')

if (-not $SkipBuild) {
    $snaplinePreviousRustflags = $env:RUSTFLAGS
    try {
        $env:RUSTFLAGS = "$snaplinePreviousRustflags -Ctarget-feature=+crt-static".Trim()
        Push-Location $snaplinePackageRoot
        try {
            cargo build --locked --release --target x86_64-pc-windows-msvc
            if ($LASTEXITCODE -ne 0) { throw 'Cargo release build failed.' }
        } finally { Pop-Location }
    } finally { $env:RUSTFLAGS = $snaplinePreviousRustflags }
}

if (-not $SourceExe) {
    $snaplineTargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $snaplinePackageRoot 'target' }
    $SourceExe = Join-Path $snaplineTargetRoot 'x86_64-pc-windows-msvc\release\snapline.exe'
}
if (-not (Test-Path -LiteralPath $SourceExe)) { throw "Application executable was not found: $SourceExe" }
$snaplineDist = Join-Path $snaplinePackageRoot 'dist'
New-Item -ItemType Directory -Path $snaplineDist -Force | Out-Null
Copy-Item -LiteralPath $SourceExe -Destination (Join-Path $snaplineDist 'Snapline.exe') -Force

$snaplineMetadataText = cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $snaplinePackageRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'Could not collect dependency license metadata.' }
$snaplineMetadata = $snaplineMetadataText | ConvertFrom-Json
$snaplineLicenseText = [Text.StringBuilder]::new()
[void]$snaplineLicenseText.AppendLine('Snapline third-party dependency license metadata and available license texts')
[void]$snaplineLicenseText.AppendLine('Slint is used under LicenseRef-Slint-Royalty-free-2.0, included in SLINT_LICENSE.md.')
foreach ($snaplineDependency in ($snaplineMetadata.packages | Where-Object source | Sort-Object name, version)) {
    [void]$snaplineLicenseText.AppendLine("`n===== $($snaplineDependency.name) $($snaplineDependency.version) =====")
    [void]$snaplineLicenseText.AppendLine("License expression: $($snaplineDependency.license)")
    [void]$snaplineLicenseText.AppendLine("Source: $($snaplineDependency.repository)")
    $snaplineDependencyRoot = Split-Path -Parent $snaplineDependency.manifest_path
    $snaplineLicenseFiles = @(Get-ChildItem -LiteralPath $snaplineDependencyRoot -File | Where-Object Name -Match '^(licen[cs]e|copying|copyright|notice)')
    $snaplineLicenseDirectory = Join-Path $snaplineDependencyRoot 'LICENSES'
    if (Test-Path -LiteralPath $snaplineLicenseDirectory -PathType Container) {
        $snaplineLicenseFiles += Get-ChildItem -LiteralPath $snaplineLicenseDirectory -File
    }
    if ($snaplineDependency.license_file) {
        $snaplineLicenseFiles += Get-Item -LiteralPath (Join-Path $snaplineDependencyRoot $snaplineDependency.license_file)
    }
    foreach ($snaplineLicenseFile in ($snaplineLicenseFiles | Sort-Object FullName -Unique)) {
        [void]$snaplineLicenseText.AppendLine("--- $($snaplineLicenseFile.Name) ---")
        [void]$snaplineLicenseText.AppendLine([IO.File]::ReadAllText($snaplineLicenseFile.FullName))
    }
}
[IO.File]::WriteAllText((Join-Path $snaplineDist 'THIRD_PARTY_LICENSES.txt'), $snaplineLicenseText.ToString(), [Text.UTF8Encoding]::new($false))

$snaplinePortableFiles = @('Snapline.exe', 'THIRD_PARTY_LICENSES.txt')
foreach ($snaplineDocument in @('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'SLINT_LICENSE.md')) {
    Copy-Item -LiteralPath (Join-Path $snaplinePackageRoot $snaplineDocument) -Destination (Join-Path $snaplineDist $snaplineDocument) -Force
    $snaplinePortableFiles += $snaplineDocument
}
$snaplinePortablePaths = $snaplinePortableFiles | ForEach-Object { Join-Path $snaplineDist $_ }
Compress-Archive -LiteralPath $snaplinePortablePaths -DestinationPath (Join-Path $snaplineDist "Snapline-$Version-Windows-x64.zip") -Force

$snaplineNsis = $env:SNAPLINE_NSIS_EXE
if (-not $snaplineNsis) {
    $snaplineNsisCommand = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if ($snaplineNsisCommand) { $snaplineNsis = $snaplineNsisCommand.Source }
}
if (-not $snaplineNsis) {
    foreach ($snaplineNsisCandidate in @(
        (Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'),
        (Join-Path $snaplinePackageRoot '.tools\NSIS\makensis.exe')
    )) {
        if (Test-Path -LiteralPath $snaplineNsisCandidate) { $snaplineNsis = $snaplineNsisCandidate; break }
    }
}
if (-not $snaplineNsis -or -not (Test-Path -LiteralPath $snaplineNsis)) {
    throw 'Portable ZIP is ready. Install NSIS or set SNAPLINE_NSIS_EXE to build the setup executable.'
}
& $snaplineNsis '/V2' "/DAPP_VERSION=$Version" "/DAPP_ROOT=$snaplinePackageRoot" (Join-Path $PSScriptRoot 'installer.nsi')
if ($LASTEXITCODE -ne 0) { throw 'NSIS installer compilation failed.' }
Get-ChildItem -LiteralPath $snaplineDist -File | Where-Object Extension -In '.exe', '.zip' | Select-Object Name, Length, FullName
