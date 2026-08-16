<#
.SYNOPSIS
    Sends real files to the real service and checks that the results come back.

.DESCRIPTION
    The one test that cannot be faked. Everything else in the suite reads a saved
    description; this drives `ctxmenu favourite run` against the SnapOtter
    instance the program was built against and looks at what lands on disk.

    Run it before and after any change to `webtool/` or `service/`.

    Why several rounds: the service decides per request whether to answer 200
    with a `downloadUrl` or 202 with a job id, and it is not deterministic.
    Measured on 2026-08-16: three identical requests gave 200, 202, 200. One
    round proves nothing, so this sends the same file several times and reports
    how many rounds came back with a file.

.PARAMETER Exe
    Which binary to test. Defaults to the release build.

.PARAMETER Favourite
    Which favourite id to run. Must exist in the user's favourites.json.

.PARAMETER Rounds
    How many times to send. Six is enough to hit the asynchronous branch at
    least once with high probability.

.EXAMPLE
    pwsh tools\abnahme_snapotter.ps1
    pwsh tools\abnahme_snapotter.ps1 -Exe D:\...\_wt\_target\...\ctxmenu.exe -Rounds 3
#>
[CmdletBinding()]
param(
    [string]$Exe = 'target\x86_64-pc-windows-msvc\release\ctxmenu.exe',
    [string]$Favourite = 'snapotter__compress_image',
    [int]$Rounds = 6
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path $Exe)) {
    Write-Host "FEHLGESCHLAGEN / FAILED: no binary at $Exe" -ForegroundColor Red
    exit 1
}

$source = Join-Path $root 'test.png'
if (-not (Test-Path $source)) {
    Write-Host "FEHLGESCHLAGEN / FAILED: no test.png at $source" -ForegroundColor Red
    exit 1
}

$work = Join-Path $root 'tmp\abnahme\lauf'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force $work | Out-Null

Write-Host "Programm / exe : $Exe"
Write-Host "Favorit / id   : $Favourite"
Write-Host "Runden / rounds: $Rounds"
Write-Host ""

$png = @(137, 80, 78, 71, 13, 10, 26, 10)
$good = 0
$async = 0
$other = 0

foreach ($round in 1..$Rounds) {
    $input = Join-Path $work "runde$round.png"
    Copy-Item $source $input

    $started = Get-Date
    # `favourite run` is the same code path as a click in the Explorer menu, but
    # it reports on the console instead of in a message box, so a script can read it.
    $output = & $Exe favourite run $Favourite $input 2>&1 | Out-String
    $code = $LASTEXITCODE
    $took = ((Get-Date) - $started).TotalSeconds

    $result = Get-ChildItem $work -File |
        Where-Object { $_.Name -like "runde$round.*" -and $_.Name -ne "runde$round.png" } |
        Select-Object -First 1

    if ($code -eq 0 -and $result) {
        $magic = [System.IO.File]::ReadAllBytes($result.FullName)[0..7]
        if (Compare-Object $magic $png) {
            Write-Host ("  Runde {0}: Datei zurueck, aber kein PNG" -f $round) -ForegroundColor Red
            $other++
        }
        else {
            $good++
            Write-Host ("  Runde {0}: OK  {1} KB in {2:N1} s" -f $round,
                [math]::Round($result.Length / 1KB), $took) -ForegroundColor Green
        }
    }
    elseif ($output -match 'Auftragsnummer|queued the job|async') {
        $async++
        Write-Host ("  Runde {0}: der Dienst hat den Auftrag nur eingereiht (202) / queued" -f $round) -ForegroundColor Yellow
    }
    else {
        $other++
        Write-Host ("  Runde {0}: FEHLER / error: {1}" -f $round, $output.Trim()) -ForegroundColor Red
    }
}

Write-Host ""
Write-Host ("Ergebnis / result: {0} von {1} Runden brachten eine Datei zurueck." -f $good, $Rounds)
if ($async -gt 0) {
    Write-Host ("{0} Runden endeten mit einer Auftragsnummer, die dieses Programm nicht abholt." -f $async) -ForegroundColor Yellow
}
if ($other -gt 0) {
    Write-Host ("{0} Runden liefen in einen anderen Fehler." -f $other) -ForegroundColor Red
}

Write-Host ""
if ($good -eq $Rounds) {
    Write-Host "BESTANDEN / PASSED: jede Runde kam zurueck." -ForegroundColor Green
    exit 0
}
if ($good -gt 0 -and $other -eq 0) {
    Write-Host "TEILWEISE / PARTIAL: senden und empfangen gehen, aber der asynchrone Fall fehlt noch." -ForegroundColor Yellow
    exit 2
}
Write-Host "FEHLGESCHLAGEN / FAILED." -ForegroundColor Red
exit 1
