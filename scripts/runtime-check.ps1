param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\x86_64-pc-windows-msvc\release\snapline.exe'),
    [string]$Artifacts = (Join-Path (Split-Path -Parent $PSScriptRoot) 'artifacts')
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "Build the native release first: $Exe" }
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
if (-not ('SnaplineRuntimeNative' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class SnaplineRuntimeNative {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X; public int Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
    public class WindowInfo { public IntPtr Handle; public string Title; public long ExtendedStyle; public Rect Bounds; public uint Dpi; }
    private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr data);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr hwnd, int index);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] private static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr OpenMutex(uint access, bool inherit, string name);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    public static bool InstanceRunning() {
        IntPtr mutex=OpenMutex(0x00100000, false, "Local\\Snapline.Instance");
        if(mutex==IntPtr.Zero) return false;
        CloseHandle(mutex); return true;
    }
    public static uint Owner(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd,out pid); return pid; }
    public static WindowInfo[] Windows(int pid) {
        var windows=new List<WindowInfo>();
        EnumWindows((hwnd,data)=> {
            if(Owner(hwnd)==pid && IsWindowVisible(hwnd)) {
                var title=new StringBuilder(512); GetWindowText(hwnd,title,title.Capacity);
                Rect rect; GetWindowRect(hwnd,out rect);
                windows.Add(new WindowInfo { Handle=hwnd, Title=title.ToString(), ExtendedStyle=GetWindowLongPtr(hwnd,-20).ToInt64(), Bounds=rect, Dpi=GetDpiForWindow(hwnd) });
            }
            return true;
        },IntPtr.Zero);
        return windows.ToArray();
    }
    public static void ToggleShortcut() {
        try { keybd_event(0x11,0,0,UIntPtr.Zero); keybd_event(0x12,0,0,UIntPtr.Zero); keybd_event(0x54,0,0,UIntPtr.Zero); }
        finally { keybd_event(0x54,0,2,UIntPtr.Zero); keybd_event(0x12,0,2,UIntPtr.Zero); keybd_event(0x11,0,2,UIntPtr.Zero); }
    }
}
'@
}

if ([SnaplineRuntimeNative]::InstanceRunning()) { throw 'An existing Snapline instance is running; this check will not affect its profile.' }
foreach ($snaplineModifier in @(0x10, 0x11, 0x12, 0x5b, 0x5c)) {
    if (([SnaplineRuntimeNative]::GetAsyncKeyState($snaplineModifier) -band 0x8000) -ne 0) { throw 'Release held modifier keys before running the runtime check.' }
}

$snaplineRunRoot = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ('runtime-' + [Guid]::NewGuid().ToString('N'))
$snaplineProfile = Join-Path $snaplineRunRoot 'profile'
$snaplineWatch = Join-Path $snaplineRunRoot 'watched-screenshots'
New-Item -ItemType Directory -Path $snaplineProfile, $snaplineWatch -Force | Out-Null
$snaplineResults = [Collections.Generic.List[object]]::new()
$snaplineStartedProcesses = [Collections.Generic.List[Diagnostics.Process]]::new()
$snaplineOriginalDataDir = $env:SNAPLINE_DATA_DIR
$snaplineOriginalForeground = [SnaplineRuntimeNative]::GetForegroundWindow()
$snaplineOriginalCursor = [SnaplineRuntimeNative+Point]::new()
[void][SnaplineRuntimeNative]::GetCursorPos([ref]$snaplineOriginalCursor)
$snaplineClipboardBefore = [SnaplineRuntimeNative]::GetClipboardSequenceNumber()
$snaplineFatal = $null
$snaplineInitialWindows = @()
$snaplineFinalWindows = @()
$snaplineApp = $null

function Add-Check([string]$Name, [bool]$Passed, $Evidence) {
    $snaplineResults.Add([pscustomobject]@{ name = $Name; passed = $Passed; evidence = $Evidence })
    $snaplineCheckStatus = if ($Passed) { 'PASS' } else { 'FAIL' }
    Write-Output "$snaplineCheckStatus $Name"
}
function Wait-For([scriptblock]$Condition, [int]$TimeoutMs = 8000) {
    $snaplineDeadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $snaplineValue = & $Condition
        if ($snaplineValue) { return $snaplineValue }
        Start-Sleep -Milliseconds 80
    } while ([DateTime]::UtcNow -lt $snaplineDeadline)
    return $null
}
function Read-Items {
    $snaplineItemsPath = Join-Path $snaplineProfile 'items.json'
    if (Test-Path -LiteralPath $snaplineItemsPath) {
        try {
            $snaplineParsedItems = Get-Content -LiteralPath $snaplineItemsPath -Raw -Encoding UTF8 | ConvertFrom-Json
            foreach ($snaplineItem in $snaplineParsedItems) { $snaplineItem }
        } catch { }
    }
}
function New-TestImage([string]$Path, [Drawing.Color]$Color) {
    $snaplineBitmap = [Drawing.Bitmap]::new(40, 30)
    $snaplineGraphics = [Drawing.Graphics]::FromImage($snaplineBitmap)
    try { $snaplineGraphics.Clear($Color); $snaplineBitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png) }
    finally { $snaplineGraphics.Dispose(); $snaplineBitmap.Dispose() }
}
function Get-WindowEvidence([int]$ProcessId) {
    foreach ($snaplineNativeWindow in [SnaplineRuntimeNative]::Windows($ProcessId)) {
        [pscustomobject]@{
            handle = $snaplineNativeWindow.Handle.ToInt64(); title = $snaplineNativeWindow.Title
            extended_style = ('0x{0:X}' -f $snaplineNativeWindow.ExtendedStyle)
            width = $snaplineNativeWindow.Bounds.Right - $snaplineNativeWindow.Bounds.Left
            height = $snaplineNativeWindow.Bounds.Bottom - $snaplineNativeWindow.Bounds.Top
            bounds = $snaplineNativeWindow.Bounds; dpi = $snaplineNativeWindow.Dpi
        }
    }
}
function Get-CaptureWindow {
    [SnaplineRuntimeNative]::Windows($snaplineApp.Id) | Where-Object Title -EQ 'Snapline Capture' | Select-Object -First 1
}

try {
    $snaplineBaseline = Join-Path $snaplineWatch 'baseline.png'
    New-TestImage $snaplineBaseline ([Drawing.Color]::CornflowerBlue)
    $snaplineSettings = [ordered]@{
        clipboard_enabled = $false; watch_enabled = $true; watch_folder = $snaplineWatch
        capture_sound = $false
        size = 'medium'; max_items = 30; show_over_fullscreen = $true
        theme = 'light'; language = 'zh-CN'; autostart = $false; hotkey = 'Ctrl+Alt+T'
    }
    [IO.File]::WriteAllText((Join-Path $snaplineProfile 'settings.json'), ($snaplineSettings | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $env:SNAPLINE_DATA_DIR = $snaplineProfile
    $snaplineApp = Start-Process -FilePath ([IO.Path]::GetFullPath($Exe)) -WindowStyle Hidden -PassThru
    $snaplineStartedProcesses.Add($snaplineApp)
    $snaplineOverlay = Wait-For { [SnaplineRuntimeNative]::Windows($snaplineApp.Id) | Where-Object { ($_.ExtendedStyle -band 0x08000080) -eq 0x08000080 -and ($_.Bounds.Right - $_.Bounds.Left) -ge 500 -and ($_.Bounds.Bottom - $_.Bounds.Top) -ge 150 -and $_.Title.Length -gt 0 } | Select-Object -First 1 }
    $snaplineInitialWindows = @(Get-WindowEvidence $snaplineApp.Id)
    if (-not $snaplineOverlay) { throw 'The native overlay did not become visible within 8 seconds.' }
    Start-Sleep -Milliseconds 250
    $snaplineForegroundAfter = [SnaplineRuntimeNative]::GetForegroundWindow()
    Add-Check 'Overlay startup does not take foreground' ([SnaplineRuntimeNative]::Owner($snaplineForegroundAfter) -ne $snaplineApp.Id) @{
        before = $snaplineOriginalForeground.ToInt64(); after = $snaplineForegroundAfter.ToInt64()
        foreground_unchanged = $snaplineOriginalForeground -eq $snaplineForegroundAfter
    }
    $snaplineOverlay = [SnaplineRuntimeNative]::Windows($snaplineApp.Id) | Where-Object Handle -EQ $snaplineOverlay.Handle | Select-Object -First 1
    $snaplineStyle = $snaplineOverlay.ExtendedStyle
    Add-Check 'Overlay uses topmost, noactivate and toolwindow styles' (($snaplineStyle -band 0x08000088) -eq 0x08000088 -and ($snaplineStyle -band 0x00040000) -eq 0) @{ extended_style = ('0x{0:X}' -f $snaplineStyle) }
    $snaplineScreen = [Windows.Forms.Screen]::FromHandle($snaplineOverlay.Handle)
    Add-Check 'Overlay aligns with monitor top and spans its width' ($snaplineOverlay.Bounds.Top -eq $snaplineScreen.Bounds.Top -and $snaplineOverlay.Bounds.Left -eq $snaplineScreen.Bounds.Left -and ($snaplineOverlay.Bounds.Right - $snaplineOverlay.Bounds.Left) -eq $snaplineScreen.Bounds.Width) @{
        overlay = $snaplineOverlay.Bounds; monitor = $snaplineScreen.Bounds; dpi = $snaplineOverlay.Dpi
    }

    $snaplineScale = $snaplineOverlay.Dpi / 96.0
    $snaplineBlank = [SnaplineRuntimeNative+Point]::new()
    $snaplineBlank.X = $snaplineOverlay.Bounds.Left + [int](32 * $snaplineScale)
    $snaplineBlank.Y = $snaplineOverlay.Bounds.Top + [int](220 * $snaplineScale)
    [void][SnaplineRuntimeNative]::SetCursorPos($snaplineBlank.X, $snaplineBlank.Y)
    $snaplineBlankReady = Wait-For { ([SnaplineRuntimeNative]::GetWindowLongPtr($snaplineOverlay.Handle, -20).ToInt64() -band 0x20) -ne 0 } 2500
    $snaplineWindowUnderBlank = [SnaplineRuntimeNative]::WindowFromPoint($snaplineBlank)
    Add-Check 'Blank overlay area passes pointer hit testing through' ($snaplineBlankReady -and $snaplineWindowUnderBlank -ne $snaplineOverlay.Handle) @{
        point = $snaplineBlank; transparent_style = [bool]$snaplineBlankReady
        hit_window = $snaplineWindowUnderBlank.ToInt64(); overlay_window = $snaplineOverlay.Handle.ToInt64()
    }

    [void][SnaplineRuntimeNative]::SetCursorPos(($snaplineOverlay.Bounds.Right - [int](300 * $snaplineScale)), ($snaplineOverlay.Bounds.Top + [int](20 * $snaplineScale)))
    $snaplineToolbarActive = Wait-For { ([SnaplineRuntimeNative]::GetWindowLongPtr($snaplineOverlay.Handle, -20).ToInt64() -band 0x20) -eq 0 } 2500
    [SnaplineRuntimeNative]::ToggleShortcut()
    $snaplineHotkeyHidden = Wait-For { ([SnaplineRuntimeNative]::GetWindowLongPtr($snaplineOverlay.Handle, -20).ToInt64() -band 0x20) -ne 0 } 2500
    [SnaplineRuntimeNative]::ToggleShortcut()
    $snaplineHotkeyRevealed = Wait-For { ([SnaplineRuntimeNative]::GetWindowLongPtr($snaplineOverlay.Handle, -20).ToInt64() -band 0x20) -eq 0 } 2500
    Add-Check 'Ctrl+Alt+T hides and reveals the interactive overlay' ([bool]$snaplineToolbarActive -and [bool]$snaplineHotkeyHidden -and [bool]$snaplineHotkeyRevealed) @{
        toolbar_initially_interactive = [bool]$snaplineToolbarActive; hidden = [bool]$snaplineHotkeyHidden; revealed = [bool]$snaplineHotkeyRevealed
    }

    Start-Sleep -Milliseconds 700
    $snaplineBaselineItems = @(Read-Items)
    Add-Check 'Folder watcher ignores pre-existing baseline files' ($snaplineBaselineItems.Count -eq 0) @{ items = $snaplineBaselineItems.Count }
    $snaplineNewSource = Join-Path $snaplineWatch 'new-screenshot.png'
    $snaplineCaptureForegroundBefore = [SnaplineRuntimeNative]::GetForegroundWindow()
    New-TestImage $snaplineNewSource ([Drawing.Color]::MediumSeaGreen)
    $snaplineSourceHash = (Get-FileHash -LiteralPath $snaplineNewSource -Algorithm SHA256).Hash.ToLowerInvariant()
    $snaplineNewImported = Wait-For { @(Read-Items | Where-Object id -EQ $snaplineSourceHash).Count -eq 1 }
    $snaplineNewItems = @(Read-Items)
    Add-Check 'Folder watcher imports a new image and creates its thumbnail' ([bool]$snaplineNewImported -and $snaplineNewItems.Count -eq 1 -and (Test-Path -LiteralPath (Join-Path $snaplineProfile "inbox\$snaplineSourceHash.png")) -and (Test-Path -LiteralPath (Join-Path $snaplineProfile "thumbs\$snaplineSourceHash.png"))) @{ items = $snaplineNewItems.Count; id = $snaplineSourceHash }
    $snaplineCapture = Wait-For { Get-CaptureWindow }
    $snaplineCaptureStyleOk = $false
    $snaplineCaptureBoundsOk = $false
    $snaplineCaptureHitThrough = $false
    $snaplineCaptureForegroundAfter = [SnaplineRuntimeNative]::GetForegroundWindow()
    $snaplineCapturePoint = [SnaplineRuntimeNative+Point]::new()
    $snaplineCaptureHit = [IntPtr]::Zero
    if ($snaplineCapture) {
        $snaplineCaptureStyleOk = ($snaplineCapture.ExtendedStyle -band 0x080000A8) -eq 0x080000A8 -and ($snaplineCapture.ExtendedStyle -band 0x00040000) -eq 0
        $snaplineCaptureScreen = [Windows.Forms.Screen]::FromHandle($snaplineCapture.Handle)
        $snaplineCaptureBoundsOk = $snaplineCapture.Bounds.Left -eq $snaplineCaptureScreen.Bounds.Left -and $snaplineCapture.Bounds.Top -eq $snaplineCaptureScreen.Bounds.Top -and $snaplineCapture.Bounds.Right -eq $snaplineCaptureScreen.Bounds.Right -and $snaplineCapture.Bounds.Bottom -eq $snaplineCaptureScreen.Bounds.Bottom
        $snaplineCapturePoint.X = $snaplineCapture.Bounds.Left + [int](($snaplineCapture.Bounds.Right - $snaplineCapture.Bounds.Left) / 2)
        $snaplineCapturePoint.Y = $snaplineCapture.Bounds.Top + [int](($snaplineCapture.Bounds.Bottom - $snaplineCapture.Bounds.Top) * 0.7)
        $snaplineCaptureHit = [SnaplineRuntimeNative]::WindowFromPoint($snaplineCapturePoint)
        $snaplineCaptureHitThrough = $snaplineCaptureHit -ne $snaplineCapture.Handle
    }
    Add-Check 'Screenshot animation helper is full-monitor, click-through and never activates' ([bool]$snaplineCapture -and $snaplineCaptureStyleOk -and $snaplineCaptureBoundsOk -and $snaplineCaptureHitThrough -and $snaplineCaptureForegroundAfter -eq $snaplineCaptureForegroundBefore -and [SnaplineRuntimeNative]::Owner($snaplineCaptureForegroundAfter) -ne $snaplineApp.Id) @{
        visible = [bool]$snaplineCapture; native_styles = $snaplineCaptureStyleOk; monitor_bounds = $snaplineCaptureBoundsOk
        extended_style = if ($snaplineCapture) { '0x{0:X}' -f $snaplineCapture.ExtendedStyle } else { $null }
        helper_bounds = if ($snaplineCapture) { $snaplineCapture.Bounds } else { $null }
        hit_point = $snaplineCapturePoint; hit_window = $snaplineCaptureHit.ToInt64(); hit_through = $snaplineCaptureHitThrough
        foreground_before = $snaplineCaptureForegroundBefore.ToInt64(); foreground_after = $snaplineCaptureForegroundAfter.ToInt64()
    }
    $snaplineCaptureHidden = Wait-For { -not (Get-CaptureWindow) } 4000
    Add-Check 'Screenshot animation helper automatically hides after landing' ([bool]$snaplineCapture -and [bool]$snaplineCaptureHidden) @{ was_visible = [bool]$snaplineCapture; hidden = [bool]$snaplineCaptureHidden }
    $snaplineDuplicate = Join-Path $snaplineWatch 'duplicate-screenshot.png'
    Copy-Item -LiteralPath $snaplineNewSource -Destination $snaplineDuplicate
    Start-Sleep -Milliseconds 1400
    $snaplineFinalItems = @(Read-Items)
    Add-Check 'Identical file content is deduplicated' ($snaplineFinalItems.Count -eq 1) @{ items = $snaplineFinalItems.Count }
    Add-Check 'Imported source files remain unchanged' ((Test-Path -LiteralPath $snaplineBaseline) -and (Test-Path -LiteralPath $snaplineDuplicate) -and (Get-FileHash -LiteralPath $snaplineNewSource -Algorithm SHA256).Hash.ToLowerInvariant() -eq $snaplineSourceHash) @{ original = $snaplineNewSource; duplicate = $snaplineDuplicate; sha256 = $snaplineSourceHash }

    $snaplineRapidFirst = Join-Path $snaplineWatch 'rapid-first.png'
    $snaplineRapidSecond = Join-Path $snaplineWatch 'rapid-second.png'
    New-TestImage $snaplineRapidFirst ([Drawing.Color]::MediumOrchid)
    $snaplineRapidFirstHash = (Get-FileHash -LiteralPath $snaplineRapidFirst -Algorithm SHA256).Hash.ToLowerInvariant()
    $snaplineRapidFirstCapture = Wait-For { Get-CaptureWindow }
    # Submit the second image during the first flight; each file is still saved.
    New-TestImage $snaplineRapidSecond ([Drawing.Color]::OrangeRed)
    $snaplineRapidSecondHash = (Get-FileHash -LiteralPath $snaplineRapidSecond -Algorithm SHA256).Hash.ToLowerInvariant()
    $snaplineRapidImported = Wait-For {
        $snaplineRapidItems = @(Read-Items)
        @($snaplineRapidItems | Where-Object id -EQ $snaplineRapidFirstHash).Count -eq 1 -and @($snaplineRapidItems | Where-Object id -EQ $snaplineRapidSecondHash).Count -eq 1
    }
    $snaplineRapidLatestCapture = Wait-For { Get-CaptureWindow } 2500
    $snaplineRapidHidden = Wait-For { -not (Get-CaptureWindow) } 4000
    $snaplineRapidSourcesKept = (Get-FileHash -LiteralPath $snaplineRapidFirst -Algorithm SHA256).Hash.ToLowerInvariant() -eq $snaplineRapidFirstHash -and (Get-FileHash -LiteralPath $snaplineRapidSecond -Algorithm SHA256).Hash.ToLowerInvariant() -eq $snaplineRapidSecondHash
    Add-Check 'Rapid captures both save and the latest animation finishes cleanly' ([bool]$snaplineRapidFirstCapture -and [bool]$snaplineRapidImported -and [bool]$snaplineRapidLatestCapture -and [bool]$snaplineRapidHidden -and $snaplineRapidSourcesKept -and @(Read-Items).Count -eq 3) @{
        first_helper_visible = [bool]$snaplineRapidFirstCapture; latest_helper_visible = [bool]$snaplineRapidLatestCapture
        both_imported = [bool]$snaplineRapidImported; helper_hidden = [bool]$snaplineRapidHidden
        sources_unchanged = $snaplineRapidSourcesKept; ids = @($snaplineRapidFirstHash, $snaplineRapidSecondHash)
    }

    $snaplineCancelSource = Join-Path $snaplineWatch 'cancel-flight.png'
    New-TestImage $snaplineCancelSource ([Drawing.Color]::Goldenrod)
    $snaplineCancelHash = (Get-FileHash -LiteralPath $snaplineCancelSource -Algorithm SHA256).Hash.ToLowerInvariant()
    $snaplineCancelCapture = Wait-For { Get-CaptureWindow }
    [void][SnaplineRuntimeNative]::SetCursorPos(($snaplineOverlay.Bounds.Right - [int](300 * $snaplineScale)), ($snaplineOverlay.Bounds.Top + [int](20 * $snaplineScale)))
    [SnaplineRuntimeNative]::ToggleShortcut()
    $snaplineCancelHidden = Wait-For { -not (Get-CaptureWindow) } 350
    # Native hit testing depends on the current pointer, so explicitly target the toolbar.
    [void][SnaplineRuntimeNative]::SetCursorPos(($snaplineOverlay.Bounds.Right - [int](300 * $snaplineScale)), ($snaplineOverlay.Bounds.Top + [int](20 * $snaplineScale)))
    $snaplineCancelPointer = [SnaplineRuntimeNative+Point]::new()
    [void][SnaplineRuntimeNative]::GetCursorPos([ref]$snaplineCancelPointer)
    $snaplineCancelStyleBefore = [SnaplineRuntimeNative]::GetWindowLongPtr($snaplineOverlay.Handle, -20).ToInt64()
    [SnaplineRuntimeNative]::ToggleShortcut()
    $snaplineCancelRestored = Wait-For { ([SnaplineRuntimeNative]::GetWindowLongPtr($snaplineOverlay.Handle, -20).ToInt64() -band 0x20) -eq 0 } 2500
    $snaplineCancelStyleAfter = [SnaplineRuntimeNative]::GetWindowLongPtr($snaplineOverlay.Handle, -20).ToInt64()
    $snaplineCancelStillHidden = -not (Get-CaptureWindow)
    $snaplineCancelSaved = @(Read-Items | Where-Object id -EQ $snaplineCancelHash).Count -eq 1 -and (Get-FileHash -LiteralPath $snaplineCancelSource -Algorithm SHA256).Hash.ToLowerInvariant() -eq $snaplineCancelHash
    Add-Check 'Hiding the shelf cancels a flying capture and restoring it keeps the saved image' ([bool]$snaplineCancelCapture -and [bool]$snaplineCancelHidden -and [bool]$snaplineCancelRestored -and $snaplineCancelStillHidden -and $snaplineCancelSaved) @{
        was_flying = [bool]$snaplineCancelCapture; cancelled_within_350ms = [bool]$snaplineCancelHidden
        shelf_restored = [bool]$snaplineCancelRestored; helper_stayed_hidden = $snaplineCancelStillHidden; saved = $snaplineCancelSaved
        pointer = $snaplineCancelPointer; style_before = ('0x{0:X}' -f $snaplineCancelStyleBefore); style_after = ('0x{0:X}' -f $snaplineCancelStyleAfter)
    }

    $snaplineSecond = Start-Process -FilePath ([IO.Path]::GetFullPath($Exe)) -WindowStyle Hidden -PassThru
    $snaplineStartedProcesses.Add($snaplineSecond)
    $snaplineSecondExited = $snaplineSecond.WaitForExit(8000)
    $snaplineSettingsWindow = Wait-For { [SnaplineRuntimeNative]::Windows($snaplineApp.Id) | Where-Object { $_.Handle -ne $snaplineOverlay.Handle -and $_.Title -match 'Settings|\u8bbe\u7f6e' } | Select-Object -First 1 }
    Add-Check 'Second instance exits and opens settings in the original process' ($snaplineSecondExited -and $snaplineSecond.ExitCode -eq 0 -and [bool]$snaplineSettingsWindow -and -not $snaplineApp.HasExited) @{
        original_pid = $snaplineApp.Id; second_pid = $snaplineSecond.Id; second_exited = $snaplineSecondExited
        settings_title = if ($snaplineSettingsWindow) { $snaplineSettingsWindow.Title } else { $null }
    }
    if ($snaplineSettingsWindow) { [void][SnaplineRuntimeNative]::PostMessage($snaplineSettingsWindow.Handle, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) }
} catch {
    $snaplineFatal = $_.Exception.Message
    Add-Check 'Runtime check completed without a fatal error' $false @{ error = $snaplineFatal }
} finally {
    if ($snaplineApp -and -not $snaplineApp.HasExited) { $snaplineFinalWindows = @(Get-WindowEvidence $snaplineApp.Id) }
    foreach ($snaplineProcess in $snaplineStartedProcesses) {
        if (-not $snaplineProcess.HasExited) { Stop-Process -Id $snaplineProcess.Id -Force -ErrorAction SilentlyContinue }
        $snaplineProcess.Dispose()
    }
    [void][SnaplineRuntimeNative]::SetCursorPos($snaplineOriginalCursor.X, $snaplineOriginalCursor.Y)
    [void][SnaplineRuntimeNative]::SetForegroundWindow($snaplineOriginalForeground)
    if ($null -eq $snaplineOriginalDataDir) { Remove-Item Env:SNAPLINE_DATA_DIR -ErrorAction SilentlyContinue }
    else { $env:SNAPLINE_DATA_DIR = $snaplineOriginalDataDir }
    $snaplineClipboardAfter = [SnaplineRuntimeNative]::GetClipboardSequenceNumber()
    Add-Check 'Clipboard sequence is unchanged' ($snaplineClipboardBefore -eq $snaplineClipboardAfter) @{ before = $snaplineClipboardBefore; after = $snaplineClipboardAfter }
    $snaplineReport = [ordered]@{
        executable = [IO.Path]::GetFullPath($Exe); executable_sha256 = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
        finished_utc = [DateTime]::UtcNow.ToString('o'); test_profile = $snaplineProfile
        initial_windows = $snaplineInitialWindows; final_windows = $snaplineFinalWindows
        passed = @($snaplineResults | Where-Object { -not $_.passed }).Count -eq 0
        checks = $snaplineResults.ToArray(); fatal_error = $snaplineFatal
    }
    $snaplineReportPath = Join-Path $snaplineRunRoot 'report.json'
    [IO.File]::WriteAllText($snaplineReportPath, ($snaplineReport | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Write-Output "Report: $snaplineReportPath"
}
if (-not $snaplineReport.passed) { throw "Runtime checks failed. See $snaplineReportPath" }
