param(
    [string]$Exe = "target\release\halo-ide.exe",
    [string]$Session = "",
    [int]$Runs = 5,
    [int]$IdleSeconds = 10
)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class HaloWindow {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int s);
    public static bool Shown(uint target) {
        bool found = false;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            int cloaked = 1;
            if (pid == target && IsWindowVisible(h) && DwmGetWindowAttribute(h, 14, out cloaked, 4) == 0 && cloaked == 0) { found = true; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@
function Median($xs) { $s = @($xs | Sort-Object); $s[[int][math]::Floor(($s.Count - 1) / 2)] }
$exe = (Resolve-Path $Exe).Path
$saved = "$env:APPDATA\halo\session.json"
$backup = "$env:TEMP\halo-session-backup.json"
if (Test-Path $saved) { Copy-Item $saved $backup -Force }
$shown = @(); $settled = @(); $ws = @(); $priv = @()
try {
    for ($i = 1; $i -le $Runs; $i++) {
        if ($Session) { Copy-Item $Session $saved -Force }
        $psi = New-Object System.Diagnostics.ProcessStartInfo $exe
        $psi.UseShellExecute = $false
        $psi.EnvironmentVariables["HALO_COMMANDS"] = "none"
        $clock = [System.Diagnostics.Stopwatch]::StartNew()
        $p = [System.Diagnostics.Process]::Start($psi)
        while (-not [HaloWindow]::Shown([uint32]$p.Id) -and $clock.ElapsedMilliseconds -lt 15000) { Start-Sleep -Milliseconds 2 }
        $tShown = $clock.ElapsedMilliseconds
        $samples = New-Object System.Collections.Generic.List[double[]]
        $busy = $tShown
        while ($clock.ElapsedMilliseconds - $busy -lt 1500 -and $clock.ElapsedMilliseconds -lt 20000) {
            Start-Sleep -Milliseconds 20
            $p.Refresh()
            $t = $clock.ElapsedMilliseconds
            $samples.Add(@($t, $p.TotalProcessorTime.TotalMilliseconds))
            $old = $samples | Where-Object { $_[0] -le $t - 500 } | Select-Object -Last 1
            if (-not $old -or $samples[$samples.Count - 1][1] - $old[1] -gt 250) { $busy = $t }
        }
        while ($clock.ElapsedMilliseconds -lt $IdleSeconds * 1000) { Start-Sleep -Milliseconds 100 }
        $p.Refresh()
        $w = [math]::Round($p.WorkingSet64 / 1MB, 1); $pr = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        taskkill /T /F /PID $p.Id 2>$null | Out-Null
        $p.WaitForExit()
        "run ${i}: shown $tShown ms, settled $busy ms, working set $w MB, private $pr MB"
        $shown += $tShown; $settled += $busy; $ws += $w; $priv += $pr
        Start-Sleep -Milliseconds 500
    }
} finally {
    if (Test-Path $backup) { Copy-Item $backup $saved -Force; Remove-Item $backup }
}
"median: shown $(Median $shown) ms, settled $(Median $settled) ms, working set $(Median $ws) MB, private $(Median $priv) MB"
