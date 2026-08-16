<#
.SYNOPSIS
    Moves finished findings to todo\fixed and rewrites todo\README.md.

.DESCRIPTION
    A finding is finished when the branch that fixes it is merged. This moves
    its file, stamps it with the branch and the commit that closed it, and
    rebuilds the overview so the open list only ever shows what is still open.

    Run it once per merged branch.

.PARAMETER Numbers
    The finding numbers this branch closed, e.g. 28,29,30.

.PARAMETER Branch
    The branch that closed them.

.PARAMETER Commit
    The commit that closed them. Defaults to the branch's tip.

.EXAMPLE
    pwsh tools\todo_erledigt.ps1 -Numbers 28,29,30 -Branch fix/toter-code
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][int[]]$Numbers,
    [Parameter(Mandatory)][string]$Branch,
    [string]$Commit
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$todo = Join-Path $root 'todo'
$fixed = Join-Path $todo 'fixed'
New-Item -ItemType Directory -Force $fixed | Out-Null

if (-not $Commit) {
    Push-Location $root
    $Commit = (git rev-parse --short $Branch 2>$null)
    Pop-Location
}
$today = Get-Date -Format 'yyyy-MM-dd'

foreach ($n in $Numbers) {
    $pattern = '{0:D2}-*.md' -f $n
    $file = Get-ChildItem $todo -Filter $pattern -File | Select-Object -First 1
    if (-not $file) {
        Write-Host ("  {0:D2}: keine Datei gefunden" -f $n) -ForegroundColor Yellow
        continue
    }

    # Stand umschreiben und die Herkunft festhalten, damit die Datei fuer sich
    # allein erklaert, wann und womit der Punkt geschlossen wurde.
    $text = Get-Content $file.FullName -Raw
    $text = $text -replace '(?m)^\| \*\*Stand\*\* \| offen \|$',
        ("| **Stand** | erledigt am $today |`n| **Behoben in** | ``$Branch`` ($Commit) |")
    Set-Content -Path $file.FullName -Value $text -NoNewline -Encoding UTF8

    Move-Item $file.FullName (Join-Path $fixed $file.Name) -Force
    Write-Host ("  {0} -> fixed\" -f $file.Name) -ForegroundColor Green
}

# ---------------------------------------------------------------------------
# Rebuild the overview from the files that are actually there.
# ---------------------------------------------------------------------------
$readme = Join-Path $todo 'README.md'
$lines = Get-Content $readme

# -Filter takes Windows wildcards, which have no character classes: '[0-9]*.md'
# there matches nothing at all. Match on the name instead.
$open = @{}
Get-ChildItem $todo -File | Where-Object { $_.Name -match '^\d+-.*\.md$' } | ForEach-Object {
    $open[[int]([regex]::Match($_.Name, '^(\d+)-').Groups[1].Value)] = $_.Name
}
$done = @{}
Get-ChildItem $fixed -File | Where-Object { $_.Name -match '^\d+-.*\.md$' } | ForEach-Object {
    $done[[int]([regex]::Match($_.Name, '^(\d+)-').Groups[1].Value)] = $_.Name
}

$out = New-Object System.Collections.Generic.List[string]
$doneRows = New-Object System.Collections.Generic.List[string]

foreach ($rawLine in $lines) {
    # Get-Content splits on \n and leaves \r behind, which would break every
    # regex anchored with $.
    $line = $rawLine.TrimEnd("`r")

    # A row of one of the three severity tables: | [07](07-....md) | Titel | Ort | Art |
    if ($line -match '^\| \[(\d+)\]\((\d+)-([^)]+)\) \| (.+?) \| (.+?) \| (.+?) \|$') {
        $num = [int]$Matches[1]
        $title = $Matches[4]
        $where = $Matches[5]
        $kind = $Matches[6]
        if ($done.ContainsKey($num)) {
            # Moved: out of the open table, into the finished one.
            $doneRows.Add(("| [{0:D2}](fixed/{1}) | {2} | {3} | {4} |" -f $num, $done[$num], $title, $where, $kind))
            continue
        }
    }
    $out.Add($line)
}

# Drop severity tables that have no rows left, together with their heading.
$cleaned = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt $out.Count; $i++) {
    $line = $out[$i]
    if ($line -match '^## (Hoch|Mittel|Niedrig)$') {
        # Look ahead: is there at least one data row before the next heading?
        $hasRow = $false
        for ($j = $i + 1; $j -lt $out.Count -and $out[$j] -notmatch '^## '; $j++) {
            if ($out[$j] -match '^\| \[\d+\]') { $hasRow = $true; break }
        }
        if (-not $hasRow) {
            while ($i + 1 -lt $out.Count -and $out[$i + 1] -notmatch '^## ') { $i++ }
            continue
        }
    }
    $cleaned.Add($line)
}

# Append or replace the finished section.
$text = ($cleaned -join "`n")
$section = "## Erledigt`n`n" +
    "Diese Punkte sind behoben. Die Berichte liegen in ``fixed\`` und tragen unten,`n" +
    "in welchem Zweig und mit welchem Commit sie geschlossen wurden.`n`n" +
    "| Nr | Punkt | Ort | Art |`n|---|---|---|---|`n"

$allDone = @()
if ($text -match '(?ms)^## Erledigt.*?\n\|---\|---\|---\|---\|\n(.*?)(?=\n## |\z)') {
    $allDone += ($Matches[1] -split "`n" | Where-Object { $_ -match '^\| \[' })
    $text = $text -replace '(?ms)\n## Erledigt.*?(?=\n## |\z)', ''
}
$allDone += $doneRows
$allDone = $allDone | Where-Object { $_ } | Sort-Object { [int]([regex]::Match($_, '\[(\d+)\]').Groups[1].Value) } -Unique

$section += (($allDone -join "`n") + "`n")

# Put it before the trailing "Ordner fixed" note if that is there, else at the end.
if ($text -match '(?ms)\n## Ordner `?fixed`?') {
    $text = $text -replace '(?ms)\n## Ordner', ("`n" + $section + "`n## Ordner")
}
else {
    $text = $text.TrimEnd() + "`n`n" + $section
}

# The two numbers in the head have to follow, or the overview lies about itself.
$total = $open.Count + $done.Count
$headline = if ($done.Count -eq 0) {
    "$total Punkte, je einer pro Datei. Details stehen dort, hier nur die Liste."
}
else {
    "$total Punkte, je einer pro Datei: **$($open.Count) offen**, $($done.Count) erledigt. " +
    "Details stehen dort, hier nur die Liste."
}
$text = $text -replace '(?m)^\d+ Punkte, je einer pro Datei.*$', $headline

# Rebuild the by-kind table from the files that are still open.
$kinds = [ordered]@{}
foreach ($name in $open.Values) {
    $body = Get-Content (Join-Path $todo $name) -Raw
    if ($body -match '(?m)^\| \*\*Art\*\* \| (.+?) \|') {
        $kind = $Matches[1].Trim()
        if ($kinds.Contains($kind)) { $kinds[$kind]++ } else { $kinds[$kind] = 1 }
    }
}
if ($kinds.Count -gt 0) {
    $rows = ($kinds.GetEnumerator() | Sort-Object Value -Descending |
        ForEach-Object { "| $($_.Key) | $($_.Value) |" }) -join "`n"
    $text = $text -replace '(?ms)(^## Nach Art\r?\n\r?\n\| Art \| Anzahl \|\r?\n\|---\|---\|\r?\n).*?(?=\r?\n## )',
        ('${1}' + $rows + "`n")
}

Set-Content -Path $readme -Value $text -Encoding UTF8

Write-Host ""
Write-Host ("Offen: {0}   Erledigt: {1}" -f $open.Count, $done.Count)
