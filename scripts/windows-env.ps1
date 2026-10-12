# Dot-source this file before cargo commands: . .\scripts\windows-env.ps1
$snaplineRepoRoot = Split-Path -Parent $PSScriptRoot
$snaplineCargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path -LiteralPath $snaplineCargoBin) {
    $env:PATH = "$snaplineCargoBin;$env:PATH"
}

$snaplineTargetPointer = Join-Path $snaplineRepoRoot '.tools\cargo-target.txt'
if (-not $env:CARGO_TARGET_DIR -and (Test-Path -LiteralPath $snaplineTargetPointer)) {
    $env:CARGO_TARGET_DIR = (Get-Content -LiteralPath $snaplineTargetPointer -Raw).Trim()
}

$snaplineMsvcRoot = $env:SNAPLINE_MSVC_ROOT
$snaplineMsvcPointer = Join-Path $snaplineRepoRoot '.tools\msvc-path.txt'
if (-not $snaplineMsvcRoot -and (Test-Path -LiteralPath $snaplineMsvcPointer)) {
    $snaplineMsvcRoot = (Get-Content -LiteralPath $snaplineMsvcPointer -Raw).Trim()
}
if (-not $snaplineMsvcRoot) {
    $snaplineMsvcRoot = Join-Path $snaplineRepoRoot '.tools\msvc'
}

if (Test-Path -LiteralPath (Join-Path $snaplineMsvcRoot 'VC\Tools\MSVC')) {
    $snaplineMsvcVersion = Get-ChildItem -LiteralPath (Join-Path $snaplineMsvcRoot 'VC\Tools\MSVC') -Directory |
        Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name
    $snaplineSdkVersion = Get-ChildItem -LiteralPath (Join-Path $snaplineMsvcRoot 'Windows Kits\10\bin') -Directory |
        Where-Object Name -Like '10.*' | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name
    if (-not $snaplineSdkVersion) { throw 'Portable Windows SDK is incomplete.' }
    $snaplineMsvc = Join-Path $snaplineMsvcRoot "VC\Tools\MSVC\$snaplineMsvcVersion"
    $snaplineSdk = Join-Path $snaplineMsvcRoot 'Windows Kits\10'
    $env:VSCMD_ARG_HOST_ARCH = 'x64'
    $env:VSCMD_ARG_TGT_ARCH = 'x64'
    $env:VCToolsVersion = $snaplineMsvcVersion
    $env:VCToolsInstallDir = "$snaplineMsvc\"
    $env:WindowsSdkDir = "$snaplineSdk\"
    $env:WindowsSDKVersion = "$snaplineSdkVersion\"
    $env:WindowsSdkBinPath = "$snaplineSdk\bin\"
    $env:WindowsSdkVerBinPath = "$snaplineSdk\bin\$snaplineSdkVersion\"
    $env:RC_PATH = "$snaplineSdk\bin\$snaplineSdkVersion\x64\rc.exe"
    $env:PATH = "$snaplineMsvc\bin\Hostx64\x64;$snaplineSdk\bin\$snaplineSdkVersion\x64;$snaplineSdk\bin\$snaplineSdkVersion\x64\ucrt;$env:PATH"
    $env:INCLUDE = "$snaplineMsvc\include;$snaplineSdk\Include\$snaplineSdkVersion\ucrt;$snaplineSdk\Include\$snaplineSdkVersion\shared;$snaplineSdk\Include\$snaplineSdkVersion\um;$snaplineSdk\Include\$snaplineSdkVersion\winrt;$snaplineSdk\Include\$snaplineSdkVersion\cppwinrt"
    $env:LIB = "$snaplineMsvc\lib\x64;$snaplineSdk\Lib\$snaplineSdkVersion\ucrt\x64;$snaplineSdk\Lib\$snaplineSdkVersion\um\x64"
} elseif (-not (Get-Command cl.exe -ErrorAction SilentlyContinue) -or -not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
    $snaplineVswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $snaplineVswhere)) {
        throw 'MSVC tools not found. Install Visual Studio Build Tools with Desktop development with C++, or set SNAPLINE_MSVC_ROOT to a portable MSVC folder.'
    }
    $snaplineVsPath = & $snaplineVswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $snaplineVsPath) { throw 'Visual Studio C++ tools were not found.' }
    $snaplineDevShell = Join-Path $snaplineVsPath 'Common7\Tools\Launch-VsDevShell.ps1'
    & $snaplineDevShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
}
