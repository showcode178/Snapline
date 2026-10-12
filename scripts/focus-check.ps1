param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\x86_64-pc-windows-msvc\release\snapline.exe'),
    [string]$Artifacts = (Join-Path (Split-Path -Parent $PSScriptRoot) 'artifacts'),
    [switch]$LockForeground
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "Build the native release first: $Exe" }
Add-Type -AssemblyName System.Windows.Forms
if (-not ('SnaplineFocusMonitor' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Threading;

// Counts injected key events only. Never stores characters or physical key presses.
public sealed class SnaplineFocusMonitor : IDisposable {
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] private struct Message {
        public IntPtr Hwnd; public uint Id; public UIntPtr WParam; public IntPtr LParam;
        public uint Time; public Point Position; public uint Private;
    }
    [StructLayout(LayoutKind.Sequential)] private struct KeyboardEvent {
        public uint Key, Scan, Flags, Time; public UIntPtr Extra;
    }
    private delegate IntPtr HookProc(int code, IntPtr message, IntPtr data);
    [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr SetWindowsHookEx(int id, HookProc callback, IntPtr module, uint thread);
    [DllImport("user32.dll")] private static extern bool UnhookWindowsHookEx(IntPtr hook);
    [DllImport("user32.dll")] private static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr message, IntPtr data);
    [DllImport("user32.dll")] private static extern int GetMessage(out Message message, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] private static extern bool PeekMessage(out Message message, IntPtr hwnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] private static extern bool PostThreadMessage(uint thread, uint message, UIntPtr wparam, IntPtr lparam);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] private static extern IntPtr GetModuleHandle(string module);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] private static extern IntPtr OpenMutex(uint access, bool inherit, string name);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool LockSetForegroundWindow(uint lockCode);
    [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();

    private readonly Dictionary<uint,int> injected = new Dictionary<uint,int>();
    private readonly ManualResetEvent ready = new ManualResetEvent(false);
    private readonly Thread thread;
    private readonly HookProc callback;
    private IntPtr hook;
    private uint threadId;
    private Exception startupError;

    public SnaplineFocusMonitor() {
        callback = Observe;
        thread = new Thread(Run) { IsBackground = true, Name = "Snapline injected-key check" };
        thread.Start();
        if (!ready.WaitOne(5000)) throw new TimeoutException("Keyboard monitor startup timed out.");
        if (startupError != null) throw startupError;
    }
    private void Run() {
        try {
            threadId = GetCurrentThreadId();
            Message message;
            PeekMessage(out message, IntPtr.Zero, 0, 0, 0);
            hook = SetWindowsHookEx(13, callback, GetModuleHandle(null), 0);
            if (hook == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            ready.Set();
            while (GetMessage(out message, IntPtr.Zero, 0, 0) > 0) { }
        } catch (Exception error) { startupError = error; ready.Set(); }
        finally { if (hook != IntPtr.Zero) UnhookWindowsHookEx(hook); }
    }
    private IntPtr Observe(int code, IntPtr message, IntPtr data) {
        if (code >= 0) {
            KeyboardEvent key = (KeyboardEvent)Marshal.PtrToStructure(data, typeof(KeyboardEvent));
            if ((key.Flags & 0x10) != 0) {
                lock (injected) {
                    int count; injected.TryGetValue(key.Key, out count); injected[key.Key] = count + 1;
                }
            }
        }
        return CallNextHookEx(hook, code, message, data);
    }
    public Dictionary<uint,int> Counts() {
        lock (injected) { return new Dictionary<uint,int>(injected); }
    }
    public void Dispose() {
        if (thread.IsAlive) {
            PostThreadMessage(threadId, 0x12, UIntPtr.Zero, IntPtr.Zero);
            if (!thread.Join(5000)) throw new TimeoutException("Keyboard monitor shutdown timed out.");
        }
        ready.Dispose();
    }
    public static bool InstanceRunning() {
        IntPtr mutex = OpenMutex(0x00100000, false, "Local\\Snapline.Instance");
        if (mutex == IntPtr.Zero) return false;
        CloseHandle(mutex); return true;
    }
}
'@
}

if ([SnaplineFocusMonitor]::InstanceRunning()) { throw 'Close the existing Snapline instance before this isolated check.' }
$snaplineRunRoot = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ('focus-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $snaplineRunRoot -Force | Out-Null
$snaplineResults = [Collections.Generic.List[object]]::new()
$snaplineMonitor = $null
$snaplineProbe = $null
$snaplineProcess = $null
$snaplineFatal = $null
$snaplineEditReport = $null
$snaplineInjectedCounts = [ordered]@{}
$snaplineForegroundLocked = $false
$snaplineForegroundUnlocked = $null
$snaplineOriginalForeground = [SnaplineFocusMonitor]::GetForegroundWindow()
$snaplineClipboardBefore = [SnaplineFocusMonitor]::GetClipboardSequenceNumber()

function Add-Check([string]$Name, [bool]$Passed, $Evidence) {
    $snaplineResults.Add([pscustomobject]@{ name = $Name; passed = $Passed; evidence = $Evidence })
    $snaplineStatus = if ($Passed) { 'PASS' } else { 'FAIL' }
    Write-Output "$snaplineStatus $Name"
}

try {
    # Old binaries can inject Alt while activating the editor. Give them our
    # empty window as the foreground target, never the user's chat window.
    $snaplineProbe = [Windows.Forms.Form]::new()
    $snaplineProbe.Text = 'Snapline focus regression check'
    $snaplineProbe.Width = 360
    $snaplineProbe.Height = 140
    $snaplineProbe.ShowInTaskbar = $false
    $snaplineProbe.StartPosition = [Windows.Forms.FormStartPosition]::CenterScreen
    $snaplineProbe.Show()
    $snaplineProbe.Activate()
    [void][SnaplineFocusMonitor]::SetForegroundWindow($snaplineProbe.Handle)
    [Windows.Forms.Application]::DoEvents()
    $snaplineProbeIsForeground = [SnaplineFocusMonitor]::GetForegroundWindow() -eq $snaplineProbe.Handle
    Add-Check 'Empty probe owns foreground before the executable starts' $snaplineProbeIsForeground @{
        expected = $snaplineProbe.Handle.ToInt64(); actual = [SnaplineFocusMonitor]::GetForegroundWindow().ToInt64()
    }
    if (-not $snaplineProbeIsForeground) { throw 'Could not activate the safe empty probe; the executable was not started.' }
    $snaplineMonitor = [SnaplineFocusMonitor]::new()
    if ($LockForeground) {
        $snaplineForegroundLocked = [SnaplineFocusMonitor]::LockSetForegroundWindow(1)
        Add-Check 'Foreground locking is enabled before the executable starts' $snaplineForegroundLocked @{
            lock_code = 1; enabled = $snaplineForegroundLocked
        }
        if (-not $snaplineForegroundLocked) { throw 'Could not lock foreground switching; the executable was not started.' }
    }
    # --edit-check creates its own empty profile, dispatches Slint pointer events,
    # and intercepts copying. This script never synthesizes keys or mouse input.
    $snaplineProcess = Start-Process -FilePath ([IO.Path]::GetFullPath($Exe)) -ArgumentList @('--edit-check', ('"{0}"' -f $snaplineRunRoot)) -WindowStyle Hidden -PassThru
    $snaplineDeadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not $snaplineProcess.WaitForExit(80)) {
        [Windows.Forms.Application]::DoEvents()
        if ([DateTime]::UtcNow -gt $snaplineDeadline) { throw 'Edit check timed out after 20 seconds.' }
    }
    $snaplineEditReportPath = Join-Path $snaplineRunRoot 'report.json'
    if (-not (Test-Path -LiteralPath $snaplineEditReportPath -PathType Leaf)) { throw 'Edit check did not create its report.' }
    $snaplineEditReport = Get-Content -LiteralPath $snaplineEditReportPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Add-Check 'Double-click editor and save interactions pass' ($snaplineProcess.ExitCode -eq 0 -and $snaplineEditReport.passed) @{
        exit_code = $snaplineProcess.ExitCode; edit_report = $snaplineEditReportPath; checks = $snaplineEditReport.checks.Count
    }
} catch {
    $snaplineFatal = $_.Exception.Message
    Add-Check 'Focus check completes without a fatal error' $false @{ error = $snaplineFatal }
} finally {
    if ($LockForeground) { $snaplineForegroundUnlocked = [SnaplineFocusMonitor]::LockSetForegroundWindow(2) }
    if ($snaplineProcess) {
        if (-not $snaplineProcess.HasExited) { Stop-Process -Id $snaplineProcess.Id -Force -ErrorAction SilentlyContinue }
        $snaplineProcess.Dispose()
    }
    if ($snaplineMonitor) {
        $snaplineMonitor.Dispose()
        $snaplineAltEvents = 0
        $snaplineTotalEvents = 0
        foreach ($snaplineCount in $snaplineMonitor.Counts().GetEnumerator()) {
            $snaplineInjectedCounts[('0x{0:X2}' -f $snaplineCount.Key)] = $snaplineCount.Value
            $snaplineTotalEvents += $snaplineCount.Value
            if ($snaplineCount.Key -in @(0x12, 0xA4, 0xA5)) { $snaplineAltEvents += $snaplineCount.Value }
        }
        Add-Check 'Opening the editor injects no Alt key events' ($snaplineAltEvents -eq 0) @{ injected_alt_events = $snaplineAltEvents }
        Add-Check 'No keyboard input is synthesized during editor interactions' ($snaplineTotalEvents -eq 0) @{ injected_events = $snaplineTotalEvents; counts_by_virtual_key = $snaplineInjectedCounts }
    }
    if ($snaplineProbe) { $snaplineProbe.Close(); $snaplineProbe.Dispose() }
    [void][SnaplineFocusMonitor]::SetForegroundWindow($snaplineOriginalForeground)
    $snaplineClipboardAfter = [SnaplineFocusMonitor]::GetClipboardSequenceNumber()
    Add-Check 'Clipboard sequence is unchanged' ($snaplineClipboardBefore -eq $snaplineClipboardAfter) @{ before = $snaplineClipboardBefore; after = $snaplineClipboardAfter }
    $snaplineReport = [ordered]@{
        executable = [IO.Path]::GetFullPath($Exe); executable_sha256 = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
        finished_utc = [DateTime]::UtcNow.ToString('o'); test_profile = (Join-Path $snaplineRunRoot 'profile')
        passed = @($snaplineResults | Where-Object { -not $_.passed }).Count -eq 0
        checks = $snaplineResults.ToArray(); fatal_error = $snaplineFatal
        injected_counts_by_virtual_key = $snaplineInjectedCounts
        foreground_lock_requested = [bool]$LockForeground
        foreground_lock_enabled = $snaplineForegroundLocked
        foreground_unlock_succeeded = $snaplineForegroundUnlocked
    }
    $snaplineReportPath = Join-Path $snaplineRunRoot 'focus-report.json'
    [IO.File]::WriteAllText($snaplineReportPath, ($snaplineReport | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Write-Output "Report: $snaplineReportPath"
}
if (-not $snaplineReport.passed) { throw "Focus checks failed. See $snaplineReportPath" }
