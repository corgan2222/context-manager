<#
.SYNOPSIS
    Sends a real file to the real service and checks that the result comes back.

.DESCRIPTION
    The one test that cannot be faked: it drives `ctxmenu --favourite` exactly
    the way a right-click in Explorer does, against the SnapOtter instance the
    program was built against, and looks at what lands on disk.

    Everything else in the test suite reads a saved description. This one proves
    that sending and receiving still work end to end.

    Run it before and after any change to `webtool/` or `service/`.

    The `--favourite` mode ends with a message box, which would block a script.
    So: start it, wait for the result file, then close the process. The file is
    written before the box appears.

.PARAMETER Exe
    Which binary to test. Defaults to the release build.

.PARAMETER Favourite
    Which favourite id to run. Must exist in the user's favourites.json.

.PARAMETER TimeoutSeconds
    How long to wait for the result file.

.EXAMPLE
    pwsh tools\abnahme_snapotter.ps1
    pwsh tools\abnahme_snapotter.ps1 -Exe target\x86_64-pc-windows-msvc\debug\ctxmenu.exe
#>
[CmdletBinding()]
param(
    [string]$Exe = 'target\x86_64-pc-windows-msvc\release\ctxmenu.exe',
    [string]$Favourite = 'snapotter__compress_image',
    [int]$TimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Fail($message) {
    Write-Host "FEHLGESCHLAGEN / FAILED: $message" -ForegroundColor Red
    exit 1
}

if (-not (Test-Path $Exe)) {
    Fail "no binary at $Exe -- build it first (cargo build --release)"
}

# The source picture. test.png is in the repository and is a real photograph,
# large enough that a compressor has something to do.
$source = Join-Path $root 'test.png'
if (-not (Test-Path $source)) { Fail "no test.png at $source" }

# Its own directory under tmp, so nothing lands next to the user's files and a
# leftover from a previous run cannot be mistaken for a result.
$work = Join-Path $root 'tmp\abnahme\lauf'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force $work | Out-Null

$input = Join-Path $work 'abnahme.png'
Copy-Item $source $input
$inputSize = (Get-Item $input).Length
Write-Host "Eingabe / input : $input ($([math]::Round($inputSize/1KB)) KB)"
Write-Host "Favorit / id    : $Favourite"
Write-Host "Programm / exe  : $Exe"

# The result lands beside the input, with the suffix the favourite carries.
$before = @(Get-ChildItem $work -File | ForEach-Object { $_.Name })

$started = Get-Date
$process = Start-Process -FilePath $Exe -ArgumentList @('--favourite', $Favourite, $input) `
    -PassThru -WindowStyle Hidden

$result = $null
while (((Get-Date) - $started).TotalSeconds -lt $TimeoutSeconds) {
    Start-Sleep -Milliseconds 400
    $fresh = @(Get-ChildItem $work -File | Where-Object { $before -notcontains $_.Name })
    if ($fresh.Count -gt 0) { $result = $fresh[0]; break }
    if ($process.HasExited -and $process.ExitCode -ne 0) { break }
}

$elapsed = ((Get-Date) - $started).TotalSeconds

# The message box keeps the process alive; it has said everything it is going to.
if (-not $process.HasExited) { $process | Stop-Process -Force }

if (-not $result) {
    Fail "no result file after $([math]::Round($elapsed,1)) s -- the service did not answer, or the favourite is broken"
}

$resultSize = $result.Length
if ($resultSize -eq 0) { Fail "the result file is empty" }

# A PNG starts with these eight bytes. A JSON error page does not.
$magic = [System.IO.File]::ReadAllBytes($result.FullName)[0..7]
$png = @(137, 80, 78, 71, 13, 10, 26, 10)
$isPng = -not (Compare-Object $magic $png)

Write-Host ""
Write-Host "Ergebnis / result : $($result.Name) ($([math]::Round($resultSize/1KB)) KB)"
Write-Host "Dauer / took      : $([math]::Round($elapsed,1)) s"
Write-Host "Gueltiges PNG     : $isPng"
Write-Host "Verkleinert auf   : $([math]::Round(100 * $resultSize / $inputSize, 1)) % der Eingabe"

if (-not $isPng) {
    $head = [System.Text.Encoding]::UTF8.GetString([System.IO.File]::ReadAllBytes($result.FullName))
    Fail "the answer is not a PNG. First bytes: $($head.Substring(0, [Math]::Min(200, $head.Length)))"
}

Write-Host ""
Write-Host "BESTANDEN / PASSED: senden und empfangen funktionieren." -ForegroundColor Green
exit 0
