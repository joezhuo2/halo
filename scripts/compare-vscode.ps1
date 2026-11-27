param(
    [string]$Halo = "target\release\halo-ide.exe",
    [string]$Code = "",
    [int]$Runs = 5,
    [int]$IdleSeconds = 20
)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class EditorWindow {
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
function Tree([int]$root) {
    $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId)
    $ids = New-Object System.Collections.Generic.List[int]
    $ids.Add($root)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        foreach ($p in $all) { if ($p.ParentProcessId -eq $ids[$i] -and -not $ids.Contains([int]$p.ProcessId)) { $ids.Add([int]$p.ProcessId) } }
    }
    @(Get-Process -Id $ids -ErrorAction SilentlyContinue)
}
function Run($exe, [string]$arguments, $vars) {
    if ($exe -eq $halo) { Remove-Item $saved -ErrorAction SilentlyContinue }
    $psi = New-Object System.Diagnostics.ProcessStartInfo $exe, $arguments
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    foreach ($k in $vars.Keys) { $psi.EnvironmentVariables[$k] = $vars[$k] }
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $p = [System.Diagnostics.Process]::Start($psi)
    $p.BeginOutputReadLine(); $p.BeginErrorReadLine()
    while (-not [EditorWindow]::Shown([uint32]$p.Id) -and $clock.ElapsedMilliseconds -lt 30000) { Start-Sleep -Milliseconds 2 }
    $shown = $clock.ElapsedMilliseconds
    while ($clock.ElapsedMilliseconds -lt $IdleSeconds * 1000) { Start-Sleep -Milliseconds 100 }
    $tree = Tree $p.Id
    $ids = @($tree | ForEach-Object { $_.Id })
    # what Task Manager shows as Memory: pages only this process uses, so shared DLL and GPU pages are not counted once per process
    $own = (Get-CimInstance Win32_PerfRawData_PerfProc_Process | Where-Object { $ids -contains [int]$_.IDProcess } | Measure-Object WorkingSetPrivate -Sum).Sum
    $run = [pscustomobject]@{
        Shown = $shown
        WorkingSet = [math]::Round(($tree | Measure-Object WorkingSet64 -Sum).Sum / 1MB, 1)
        Own = [math]::Round($own / 1MB, 1)
        Private = [math]::Round(($tree | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 1)
        Processes = $tree.Count
    }
    taskkill /T /F /PID $p.Id 2>$null | Out-Null
    $p.WaitForExit()
    Start-Sleep -Seconds 2
    $run
}
function Bench($name, $exe, [string]$arguments, $vars = @{}) {
    Run $exe $arguments $vars | Out-Null
    $runs = for ($i = 1; $i -le $Runs; $i++) {
        $r = Run $exe $arguments $vars
        "${name} run ${i}: shown $($r.Shown) ms, working set $($r.WorkingSet) MB, private working set $($r.Own) MB, private $($r.Private) MB, $($r.Processes) processes" | Write-Host
        $r
    }
    [pscustomobject]@{
        Editor = $name
        "Window on screen (ms)" = Median $runs.Shown
        "Working set (MB)" = Median $runs.WorkingSet
        "Private working set (MB)" = Median $runs.Own
        "Private bytes (MB)" = Median $runs.Private
        Processes = Median $runs.Processes
    }
}

if (-not $Code) {
    $cli = (Get-Command code -ErrorAction SilentlyContinue).Source
    $Code = if ($cli) { Join-Path (Split-Path (Split-Path $cli)) "Code.exe" } else { "$env:LOCALAPPDATA\Programs\Microsoft VS Code\Code.exe" }
}
if (-not (Test-Path $Halo)) { throw "No Halo build at $Halo. Run cargo build --release from the repository root, or pass -Halo with the path to halo-ide.exe." }
if (-not (Test-Path $Code)) { throw "No VS Code at $Code. Pass -Code with the path to Code.exe." }
$halo = (Resolve-Path $Halo).Path
$folder = Join-Path $env:TEMP "editor-compare"
New-Item -ItemType Directory -Force (Join-Path $folder "docs") | Out-Null
Set-Content (Join-Path $folder "README.md") "# Test folder"
Set-Content (Join-Path $folder "docs\notes.txt") "notes"
Set-Content (Join-Path $folder "data.json") '{ "a": 1 }'
# a profile of its own makes VS Code start a separate instance with nothing to restore; extensions still load from ~\.vscode\extensions
$userData = Join-Path $env:TEMP "editor-compare-vscode"
$vscode = "--user-data-dir `"$userData`" --new-window --skip-welcome --skip-release-notes --disable-workspace-trust `"$folder`""

$saved = "$env:APPDATA\halo\session.json"
$backup = "$env:TEMP\halo-session-backup.json"
if (Test-Path $saved) { Move-Item $saved $backup -Force }
try {
    $results = @(
        Bench "VS Code" $Code $vscode
        Bench "VS Code, no extensions" $Code "$vscode --disable-extensions"
        Bench "Halo" $halo "`"$folder`"" @{ HALO_COMMANDS = "none" }
    )
} finally {
    if (Test-Path $backup) { Move-Item $backup $saved -Force } else { Remove-Item $saved -ErrorAction SilentlyContinue }
}
$results | Format-Table -AutoSize
"VS Code $((Get-Item $Code).VersionInfo.ProductVersion), $(@(Get-ChildItem "$env:USERPROFILE\.vscode\extensions" -Directory -ErrorAction SilentlyContinue).Count) extensions installed"
