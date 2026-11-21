if ($Global:__HaloState) { return }
$Global:__HaloState = @{ LastId = -1; Esc = [char]0x1b; Bel = [char]7; Prompt = $function:prompt; ReadLine = $null }

function Global:__Halo-Escape([string]$Value) {
    $out = [System.Text.StringBuilder]::new()
    foreach ($c in $Value.ToCharArray()) {
        if ($c -eq [char]'\') { [void]$out.Append('\\') }
        elseif ([int]$c -lt 0x20 -or $c -eq [char]';') { [void]$out.Append(('\x{0:x2}' -f [int]$c)) }
        else { [void]$out.Append($c) }
    }
    $out.ToString()
}

function Global:__Halo-Osc([string]$Body) {
    "$($Global:__HaloState.Esc)]633;$Body$($Global:__HaloState.Bel)"
}

function Global:prompt {
    $code = if ($?) { 0 } elseif ($LASTEXITCODE) { $LASTEXITCODE } else { 1 }
    $s = $Global:__HaloState
    $last = Get-History -Count 1
    $out = ''
    if ($s.LastId -ne -1) {
        if ($last -and $last.Id -ne $s.LastId) {
            $out += __Halo-Osc "E;$(__Halo-Escape $last.CommandLine)"
            $out += __Halo-Osc "D;$code"
        } else {
            $out += __Halo-Osc 'D'
        }
    }
    $out += __Halo-Osc "P;Cwd=$(__Halo-Escape $executionContext.SessionState.Path.CurrentLocation.ProviderPath)"
    $out += __Halo-Osc 'A'
    $out += & $s.Prompt
    $out += __Halo-Osc 'B'
    $s.LastId = if ($last) { $last.Id } else { 0 }
    $out
}

if (Get-Command PSConsoleHostReadLine -ErrorAction SilentlyContinue) {
    $Global:__HaloState.ReadLine = $function:PSConsoleHostReadLine
    function Global:PSConsoleHostReadLine {
        $line = & $Global:__HaloState.ReadLine
        [Console]::Write((__Halo-Osc "E;$(__Halo-Escape $line)") + (__Halo-Osc 'C'))
        $line
    }
}
