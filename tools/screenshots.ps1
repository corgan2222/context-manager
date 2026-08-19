<#
.SYNOPSIS
    Takes the same set of screenshots every time. English only by default;
    German is a switch away and comes back for the release set.

.DESCRIPTION
    For the README on GitHub and as raw material for a walkthrough video.

    The point is repeatability: run it after an update and the pictures line up
    with the old ones, so a diff shows what actually changed in the interface
    rather than where the window happened to sit.

    What makes them repeatable:

    * `--window 2400x1500` fixes the size, so nothing reflows between runs.
    * `--lang de|en` fixes the language for the run without touching
      %LOCALAPPDATA%\ctxmenu\settings.json. The user's own setting is never
      written to by this script.
    * `--synthetic <n>` fills the table with generated rows for the shots where
      the content does not matter. The generator is deterministic, so the same
      number gives the same rows on every machine.
    * The script waits for the program to report on stderr that it is up
      (`startup_to_first_list_ms` where there is a table, `window_placed`
      where there is not), then waits again for the icon worker to catch up,
      so half-drawn rows do not end up in a picture.
    * `-Compare` cuts the status bar and the scroll bar away before diffing,
      because both change on their own. Measured across two runs with that in
      place: every picture identical to the pixel.

    Three things this had to get right on this machine, all of them learned the
    hard way and written down in CLAUDE.md:

    * **DPI first.** Four screens at 3840x2160 with 150% scaling. A script that
      does not call SetProcessDpiAwarenessContext sees them as 2560x1440 and
      gets two thirds of the real coordinates out of GetWindowRect. The older
      tools\capture_window.ps1 still does not, which is why its numbers are off
      by a factor of 1.5 on this machine.
    * **PrintWindow returns black** for this OpenGL window, measured: not one
      bright pixel. So the screen is copied instead.
    * **Only the window's own rectangle** is copied, never the whole desktop:
      the other screens have private things on them.

.PARAMETER Exe
    Which binary to photograph.

.PARAMETER Out
    Where the pictures go. Default tmp\screenshots.

.PARAMETER Languages
    Which languages to shoot. Default English alone, because that is what the
    README on GitHub shows and shooting one language halves the runtime while
    the layout is still being worked on. Every title below is written in both,
    so -Languages de,en gives the full set back without another edit.

.PARAMETER Only
    Shoot only the entries whose name matches this wildcard.

.PARAMETER Compare
    Compare against the pictures already in -Out instead of overwriting them,
    and report which ones changed. Needs ImageMagick.

.EXAMPLE
    pwsh tools\screenshots.ps1
    pwsh tools\screenshots.ps1 -Only '08-*'
    pwsh tools\screenshots.ps1 -Languages de,en
    pwsh tools\screenshots.ps1 -Compare
#>
[CmdletBinding()]
param(
    [string]$Exe = 'target\x86_64-pc-windows-msvc\release\ctxmenu.exe',
    [string]$Out = 'tmp\screenshots',
    # English alone by default. The German titles are still in the list, so
    # the release set is one -Languages de,en away.
    [ValidateSet('de', 'en')][string[]]$Languages = @('en'),
    [string]$Only = '*',
    # In PHYSICAL pixels. At 150% scaling this machine turns 2400x1500 into
    # 1600x1000 logical points, which is what the interface was laid out for.
    # Measured at 1600 physical (1067 logical): the status bar overlaps itself
    # and the toolbar clips, the same class of problem CLAUDE.md records at
    # 1267 logical points. Wide enough is part of being repeatable.
    [string]$WindowSize = '2400x1500',
    [switch]$Compare
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# PowerShell 7 dropped System.Drawing from the box: it lives in the
# System.Drawing.Common package now, and asking for Bitmap there fails with
# CS1069. Windows PowerShell 5.1 still has it and ships with every Windows, so
# the capture half runs there. Re-launch once, passing the same arguments on.
if ($PSVersionTable.PSEdition -eq 'Core') {
    # -Command rather than -File: with -File every argument arrives as one
    # string, so an array parameter turns into the literal "de,en" and fails
    # its ValidateSet. Building a command line keeps arrays arrays.
    $parts = foreach ($key in $PSBoundParameters.Keys) {
        $value = $PSBoundParameters[$key]
        if ($value -is [switch]) {
            if ($value.IsPresent) { "-$key" }
        }
        elseif ($value -is [array]) {
            "-$key " + (($value | ForEach-Object { "'$_'" }) -join ',')
        }
        else { "-$key '$value'" }
    }
    $command = "& '$PSCommandPath' " + ($parts -join ' ')
    & "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" `
        -NoProfile -ExecutionPolicy Bypass -Command $command
    exit $LASTEXITCODE
}

# ---------------------------------------------------------------------------
# The plan: what gets photographed, and what each picture is for.
#
# `Args` are passed to the program as-is. `Wait` is extra milliseconds after the
# list is reported, for views that keep loading after the first frame (icons,
# a service description off the network).
#
# Keep this list in the order a reader should meet the program: what it shows,
# what it can take apart, what it can add, what it can undo.
# ---------------------------------------------------------------------------
$shots = @(
    @{
        Name  = '01-overview'
        Title = @{ de = 'Alle Einträge auf einen Blick'; en = 'Every entry at a glance' }
        Use   = 'README top: the one picture that has to say what this is'
        Args  = @('--tab', 'categories')
        Wait  = 2500
    }
    @{
        Name  = '02-entry-detail'
        Title = @{ de = 'Ein Eintrag, aufgeschlüsselt'; en = 'One entry, taken apart' }
        Use   = 'README: registry path, scope, program, what the flags mean'
        Args  = @('--tab', 'categories', '--search', '7-Zip')
        Wait  = 2500
    }
    @{
        Name  = '03-search'
        Title = @{ de = 'Suchen und filtern'; en = 'Search and filter' }
        Use   = 'README + video: how you find the one entry that bothers you'
        Args  = @('--tab', 'categories', '--search', 'git')
        Wait  = 2000
    }
    @{
        Name  = '04-new-entry'
        Title = @{ de = 'Einen eigenen Eintrag anlegen'; en = 'Adding an entry of your own' }
        Use   = 'README: the form that puts a program of your own into the menu'
        # Opens the editor straight away, filled with an example. Nothing is
        # written: the dialog waits for a click this script never makes.
        Args  = @('--new', 'directory')
        Wait  = 4000
        # A dialog over the table. Waiting for window_placed rather than for
        # the list keeps this working if the dialog ever stops loading one.
        Ready = 'window_placed'
    }
    @{
        Name  = '05-file-types'
        Title = @{ de = 'Wo ein Dateityp seine Einträge herhat'; en = 'Where a file type gets its entries' }
        Use   = 'README: the resolution chain, the part no other tool shows'
        Args  = @('--tab', 'filetypes', '--ext', '.png')
        Wait  = 3000
    }
    @{
        Name  = '06-programs'
        Title = @{ de = 'Nach Programm gruppiert'; en = 'Grouped by program' }
        Use   = 'README: twenty keys of one program as one row, with its icon'
        Args  = @('--tab', 'programs')
        Wait  = 5000
        # No entry table here, so no startup_to_first_list_ms is ever printed.
        Ready = 'window_placed'
    }
    @{
        Name  = '07-favourites'
        Title = @{ de = 'Eigene Werkzeuge im Menü'; en = 'Your own tools in the menu' }
        Use   = 'README: programs and web tools the user put there'
        Args  = @('--tab', 'favourites')
        Wait  = 3000
        # No entry table here, so no startup_to_first_list_ms is ever printed.
        Ready = 'window_placed'
    }
    @{
        Name  = '08-services'
        Title = @{ de = 'Ein Webdienst wird zum Menü'; en = 'A web service becomes a menu' }
        Use   = 'README headline: an OpenAPI description turned into entries'
        # With the service picked, not just the tab open. Without it the panel
        # says "pick a service on the left", and the tab carrying this
        # program's most distinctive feature had its least useful picture.
        Args  = @('--service', 'snapotter')
        # The longest wait in the list, and the only one that depends on
        # something outside this machine: the description is fetched over HTTP
        # and the tools are grouped once it arrives.
        Wait  = 8000
        Ready = 'window_placed'
    }
    @{
        Name  = '09-backups'
        Title = @{ de = 'Jede Änderung ist gesichert'; en = 'Every change is backed up' }
        Use   = 'README: the promise that makes the rest safe to use'
        Args  = @('--tab', 'backups')
        Wait  = 3000
        # No entry table here, so no startup_to_first_list_ms is ever printed.
        Ready = 'window_placed'
    }
    @{
        Name  = '10-many-entries'
        Title = @{ de = 'Auch mit tausenden Zeilen flüssig'; en = 'Still smooth at thousands of rows' }
        Use   = 'README: the performance claim, with the row count visible'
        Args  = @('--tab', 'categories', '--synthetic', '2000')
        Wait  = 2500
    }
)

# ---------------------------------------------------------------------------
# Win32: DPI awareness, window rectangle, foreground, screen copy.
# ---------------------------------------------------------------------------
if (-not ('Shot.Win' -as [type])) {
    Add-Type -Language CSharp -ReferencedAssemblies System.Drawing, System.Windows.Forms @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

namespace Shot {
  public struct RECT { public int Left, Top, Right, Bottom; }

  public static class Win {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after,
        int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr value);

    // PER_MONITOR_AWARE_V2. Without this the rectangle comes back scaled and
    // the copy lands somewhere else entirely.
    static readonly IntPtr PER_MONITOR_V2 = new IntPtr(-4);
    static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
    static readonly IntPtr HWND_NOTOPMOST = new IntPtr(-2);
    const uint SWP_NOMOVE = 0x0002, SWP_NOSIZE = 0x0001, SWP_NOACTIVATE = 0x0010;

    public static bool AnnounceDpi() { return SetProcessDpiAwarenessContext(PER_MONITOR_V2); }

    public static void Raise(IntPtr h) {
      ShowWindow(h, 5); // SW_SHOW
      SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
      SetForegroundWindow(h);
    }

    public static void Lower(IntPtr h) {
      SetWindowPos(h, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }

    /// Copies just this window's rectangle off the screen. Never the desktop:
    /// the other monitors have private things on them.
    public static string Capture(IntPtr h, string path) {
      RECT r;
      if (!GetWindowRect(h, out r)) return "GetWindowRect failed";
      int w = r.Right - r.Left, ht = r.Bottom - r.Top;
      if (w <= 0 || ht <= 0) return "window has no size";

      using (var bmp = new Bitmap(w, ht, PixelFormat.Format32bppArgb))
      using (var g = Graphics.FromImage(bmp)) {
        g.CopyFromScreen(r.Left, r.Top, 0, 0, new Size(w, ht), CopyPixelOperation.SourceCopy);
        bmp.Save(path, ImageFormat.Png);
      }
      return string.Format("{0}x{1} at {2},{3}", w, ht, r.Left, r.Top);
    }
  }
}
'@
}

if (-not [Shot.Win]::AnnounceDpi()) {
    Write-Host "Hinweis: DPI-Anmeldung schlug fehl (schon gesetzt?). Koordinaten pruefen." -ForegroundColor Yellow
}

if (-not (Test-Path $Exe)) {
    Write-Host "Kein Programm unter $Exe. Erst bauen: cargo build --release" -ForegroundColor Red
    exit 1
}

$outDir = Join-Path $root $Out
New-Item -ItemType Directory -Force $outDir | Out-Null
$compareDir = $null
if ($Compare) {
    $compareDir = Join-Path $root 'tmp\screenshots_vergleich'
    if (Test-Path $compareDir) { Remove-Item $compareDir -Recurse -Force }
    New-Item -ItemType Directory -Force $compareDir | Out-Null
}

# Proof that the user's settings were not touched. This script exists to take
# pictures, not to change how the program is set up.
$settingsPath = Join-Path $env:LOCALAPPDATA 'ctxmenu\settings.json'
$settingsBefore = if (Test-Path $settingsPath) { (Get-FileHash $settingsPath).Hash } else { 'keine' }

Write-Host "Programm : $Exe"
Write-Host "Ziel     : $outDir"
Write-Host "Sprachen : $($Languages -join ', ')"
Write-Host ""

$made = @()
$failed = @()

foreach ($shot in $shots) {
    if ($shot.Name -notlike $Only) { continue }

    foreach ($lang in $Languages) {
        $name = "$($shot.Name)_$lang.png"
        $target = Join-Path $(if ($Compare) { $compareDir } else { $outDir }) $name

        $logFile = Join-Path $env:TEMP "ctxmenu_shot_$([guid]::NewGuid().ToString('N')).log"
        $arguments = @('--lang', $lang, '--window', $WindowSize) + $shot.Args

        $process = Start-Process -FilePath $Exe -ArgumentList $arguments -PassThru `
            -RedirectStandardError $logFile

        # Wait for the program to say it is up, then let the content land.
        # Which line counts depends on the tab: only the ones showing the entry
        # table report `startup_to_first_list_ms`. The others (programs,
        # favourites, services, backups) never do, and waiting for it there
        # times out on a window that has been ready for twenty seconds.
        $readyPattern = if ($shot.Ready) { $shot.Ready } else { 'startup_to_first_list_ms' }
        $ready = $false
        $waited = 0
        while ($waited -lt 25000) {
            Start-Sleep -Milliseconds 250
            $waited += 250
            if ($process.HasExited) { break }
            if ((Test-Path $logFile) -and (Select-String -Path $logFile -Pattern $readyPattern -Quiet)) {
                $ready = $true
                break
            }
        }

        if (-not $ready) {
            $failed += "$name (das Fenster meldete keine fertige Liste)"
            if (-not $process.HasExited) { $process | Stop-Process -Force }
            Remove-Item $logFile -ErrorAction SilentlyContinue
            continue
        }

        Start-Sleep -Milliseconds $shot.Wait

        $process.Refresh()
        $handle = $process.MainWindowHandle
        if ($handle -eq 0 -or -not [Shot.Win]::IsWindowVisible($handle)) {
            $failed += "$name (kein sichtbares Fenster)"
            $process | Stop-Process -Force
            Remove-Item $logFile -ErrorAction SilentlyContinue
            continue
        }

        [Shot.Win]::Raise($handle)
        Start-Sleep -Milliseconds 700   # let the compositor finish raising it
        $where = [Shot.Win]::Capture($handle, $target)
        [Shot.Win]::Lower($handle)

        $process | Stop-Process -Force
        Remove-Item $logFile -ErrorAction SilentlyContinue

        if (Test-Path $target) {
            $kb = [math]::Round((Get-Item $target).Length / 1KB)
            Write-Host ("  {0,-28} {1,6} KB  {2}" -f $name, $kb, $where)
            $made += $target
        }
        else {
            $failed += "$name (keine Datei entstanden)"
        }
    }
}

$settingsAfter = if (Test-Path $settingsPath) { (Get-FileHash $settingsPath).Hash } else { 'keine' }

Write-Host ""
Write-Host ("Aufnahmen / shots : {0}" -f $made.Count)
if ($failed.Count -gt 0) {
    Write-Host "Fehlgeschlagen / failed:" -ForegroundColor Red
    $failed | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
}
Write-Host ("settings.json     : {0}" -f $(if ($settingsBefore -eq $settingsAfter) { 'unveraendert / untouched' } else { 'VERAENDERT!' }))

if ($settingsBefore -ne $settingsAfter) {
    Write-Host "Die Einstellungen des Benutzers wurden veraendert. Das darf nicht passieren." -ForegroundColor Red
    exit 1
}

# ---------------------------------------------------------------------------
# Comparison run: what moved since last time?
# ---------------------------------------------------------------------------
if ($Compare) {
    # A fresh install does not reach an already-running shell's PATH, so look
    # where winget puts it before giving up.
    $magick = (Get-Command magick -ErrorAction SilentlyContinue).Source
    if (-not $magick) {
        $magick = Get-ChildItem 'C:\Program Files\ImageMagick*\magick.exe' -ErrorAction SilentlyContinue |
            Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $magick) {
        Write-Host "ImageMagick fehlt, kein Vergleich moeglich." -ForegroundColor Yellow
        Write-Host "  winget install ImageMagick.ImageMagick" -ForegroundColor Yellow
        exit 0
    }

    Write-Host ""
    Write-Host "Vergleich mit den vorhandenen Bildern:"
    $diffDir = Join-Path $root 'tmp\screenshots_diff'
    New-Item -ItemType Directory -Force $diffDir | Out-Null
    $changed = 0

    foreach ($fresh in Get-ChildItem $compareDir -Filter *.png) {
        $old = Join-Path $outDir $fresh.Name
        if (-not (Test-Path $old)) {
            Write-Host ("  {0,-28} neu" -f $fresh.Name) -ForegroundColor Yellow
            continue
        }
        $diffFile = Join-Path $diffDir $fresh.Name

        # Two strips are cut away before comparing, both of them noise rather
        # than interface:
        #
        # * The bottom 40 pixels are the status bar, which carries frames per
        #   second, frame time and startup milliseconds. Those differ on every
        #   run by design.
        # * The right 16 pixels are the scroll bar. egui fades it in and out
        #   depending on how long ago the last event was, so it is present in
        #   one run and gone in the next. Measured: this alone accounted for
        #   every difference between two runs, a 10x1390 strip at x=2390,
        #   the same 3065 pixels in all four pictures.
        #
        # What is left is the part where a real change to the interface shows.
        $cropped = @{}
        foreach ($side in @{ old = $old; new = $fresh.FullName }.GetEnumerator()) {
            $cut = Join-Path $env:TEMP ("cmp_{0}_{1}" -f $side.Key, $fresh.Name)
            & $magick $side.Value -gravity South -chop 0x40 -gravity East -chop 16x0 $cut
            $cropped[$side.Key] = $cut
        }

        # AE counts differing pixels. It reports on stderr and exits non-zero
        # whenever the images differ at all, which PowerShell would otherwise
        # turn into a terminating error.
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $out = (& $magick compare -metric AE $cropped.old $cropped.new $diffFile 2>&1 | Out-String)
        $ErrorActionPreference = $previous
        $cropped.Values | ForEach-Object { Remove-Item $_ -ErrorAction SilentlyContinue }

        # "3065" or "3065.24 (0.000874784)" depending on build: take the leading number.
        $pixels = if ($out -match '(\d+(?:\.\d+)?)') { [double]$Matches[1] } else { -1 }

        # Under one whole pixel there is nothing anyone could see. This build
        # of ImageMagick is Q16-HDRI and reports AE as a fraction rather than
        # a count: measured between two runs of the same shot, with the status
        # bar and the scroll bar already cut away, "0.294118 (8.4501e-08)".
        # The old check (> 0) called that a change and then printed it as
        # "0 Bildpunkte anders", so every run reported one phantom difference.
        if ($pixels -lt 1) {
            Write-Host ("  {0,-28} gleich / identical" -f $fresh.Name) -ForegroundColor Green
            Remove-Item $diffFile -ErrorAction SilentlyContinue
        }
        else {
            $share = 100 * $pixels / (2384 * 1460)
            Write-Host ("  {0,-28} {1:N0} Bildpunkte anders ({2:N2} %)" -f $fresh.Name, $pixels, $share) `
                -ForegroundColor Yellow
            $changed++
        }
    }
    Write-Host ""
    Write-Host ("{0} von {1} Bildern haben sich geaendert. Unterschiede: {2}" -f `
        $changed, (Get-ChildItem $compareDir -Filter *.png).Count, $diffDir)
}

exit $(if ($failed.Count -gt 0) { 1 } else { 0 })
