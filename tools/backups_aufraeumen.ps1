<#
.SYNOPSIS
    Removes the backup directories the test suite left behind.

.DESCRIPTION
    Measured on 2026-08-16: %LOCALAPPDATA%\ctxmenu\backups held 1289
    directories, of which 1274 came from test runs and 15 were real. The tests
    have since learned to clean up after themselves even when an assertion
    fails (see todo\fixed\33-*), but everything from before that is still there,
    and it sits between the user's own backups in the Backups tab.

    This is deliberately a separate script that has to be started by hand, and
    deliberately shows what it would do before doing it. That directory belongs
    to the user: fifteen of those entries are the only copy of a registry key
    somebody deleted on purpose.

    What counts as a test artefact: the action name is exactly one of the names
    the test suite uses. Anything else is left alone, including anything the
    user named himself.

.PARAMETER Apply
    Actually remove them. Without this the script only reports.

.EXAMPLE
    pwsh tools\backups_aufraeumen.ps1           # zeigt nur, was es tun wuerde
    pwsh tools\backups_aufraeumen.ps1 -Apply    # raeumt wirklich auf
#>
[CmdletBinding()]
param([switch]$Apply)

$ErrorActionPreference = 'Stop'

# Every label the test suite passes to backup::export / execute() begins with
# `selftest`, and the tests write their registry keys under
# HKCU\...\Classes\ctxmenu_selftest_* for the same reason: the prefix is the
# convention that keeps them apart from anything real.
#
# A fixed list was the first attempt and it was wrong: a later branch added
# selftest_blocked, _elsewhere, _wide, _gap and _absent, and the script would
# have quietly kept 140 directories it should have removed. The prefix is what
# the tests actually promise, so the prefix is what this matches.
#
# What this means for a user's own action names: an action called `selftest`
# would be caught. Every real label this program produces is a verb it chose
# itself (delete, gesamt, manuell, Löschen) or one the user typed in the
# window, and the list below shows what is kept before anything is removed.
$testPrefix = 'selftest'

$root = Join-Path $env:LOCALAPPDATA 'ctxmenu\backups'
if (-not (Test-Path $root)) {
    Write-Host "Kein Sicherungsverzeichnis unter $root."
    exit 0
}

# Directory names look like 20260816T165711123_selftest_plan_2: a timestamp,
# an underscore, the action, and possibly _2 from unique_directory when two
# actions landed in the same millisecond.
$all = Get-ChildItem $root -Directory
$artefacts = $all | Where-Object {
    $action = $_.Name -replace '^\d{8}T\d{9}_', '' -replace '_\d+$', ''
    $action -like "$testPrefix*"
}
$keep = $all.Count - $artefacts.Count

Write-Host ("Verzeichnisse gesamt / total     : {0}" -f $all.Count)
Write-Host ("Davon aus Testlaeufen / tests    : {0}" -f $artefacts.Count)
Write-Host ("Echte Sicherungen / real, kept   : {0}" -f $keep) -ForegroundColor Green
Write-Host ""

if ($keep -gt 0) {
    Write-Host "Diese bleiben / these are kept:"
    $all | Where-Object { $artefacts -notcontains $_ } |
        Sort-Object Name -Descending |
        ForEach-Object { Write-Host ("  {0}" -f $_.Name) }
    Write-Host ""
}

if (-not $Apply) {
    Write-Host "Nichts entfernt. Zum Aufraeumen: tools\backups_aufraeumen.ps1 -Apply" -ForegroundColor Yellow
    exit 0
}

# One retry pass: a directory whose .reg files were written moments ago can
# still be held by a scanner, and the same contention is why the tests needed
# a retry of their own.
$removed = 0
$stuck = @()
foreach ($directory in $artefacts) {
    $done = $false
    foreach ($attempt in 1..3) {
        try {
            Remove-Item $directory.FullName -Recurse -Force -ErrorAction Stop
            $removed++
            $done = $true
            break
        }
        catch {
            if ($attempt -lt 3) { Start-Sleep -Milliseconds 300 }
        }
    }
    if (-not $done) { $stuck += $directory.Name }
}

Write-Host ("Entfernt / removed : {0}" -f $removed) -ForegroundColor Green
if ($stuck.Count -gt 0) {
    Write-Host ("Belegt geblieben / still locked: {0}" -f $stuck.Count) -ForegroundColor Yellow
    Write-Host "Noch einmal starten, wenn nichts mehr darauf zugreift."
}
