# Screenshots: how they are taken, and what still needs work

*[Deutsche Fassung](SCREENSHOTS_DE.md)*

`tools\screenshots.ps1` takes the same set of pictures on every run, in German
and in English, so that after an update a diff shows what changed in the
interface rather than where the window happened to sit.

This file is the working note for that script: what it does, why each decision
was made, and what is left to do before a release.

## Running it

```powershell
pwsh tools\screenshots.ps1                          # all nine views, both languages
pwsh tools\screenshots.ps1 -Only '05-*'             # one view
pwsh tools\screenshots.ps1 -Languages de            # one language
pwsh tools\screenshots.ps1 -Compare                 # shoot again and diff against the last set
pwsh tools\screenshots.ps1 -WindowSize 3000x1900    # a different size
```

Output goes to `tmp\screenshots\`. That folder is in `.gitignore`: the pictures
are artefacts, and the ones a release actually uses get copied to `docs\images\`
by hand.

Runtime is about four minutes for all eighteen, because each picture starts the
program from scratch.

## What it photographs

Nine views, in the order a reader should meet the program.

| Name | Started with | For |
|---|---|---|
| `01-uebersicht` | `--tab categories` | The one picture that has to say what this is |
| `02-eintrag-im-detail` | `--tab categories --search 7-Zip` | Registry path, scope, program, flags |
| `03-suche` | `--tab categories --search git` | Finding the one entry that bothers you |
| `04-dateitypen` | `--tab filetypes --ext .png` | The resolution chain, which no other tool shows |
| `05-programme` | `--tab programs` | Twenty keys of one program as one row |
| `06-favoriten` | `--tab favourites` | Programs and web tools the user put there |
| `07-dienste` | `--tab services` | An OpenAPI description turned into entries |
| `08-sicherungen` | `--tab backups` | The promise that makes the rest safe to use |
| `09-viele-eintraege` | `--synthetic 2000` | The performance claim, with the row count visible |

The list lives at the top of the script as data, with a `Use` line per entry.
Adding a view means adding four lines there, nothing else.

## What makes the pictures repeatable

Measured across two runs with everything below in place: **every picture
identical to the pixel.**

* **`--window 2400x1500`** fixes the size, so nothing reflows between runs.
  Physical pixels: at 150% scaling this machine turns that into 1600x1000
  logical points, which is what the interface was laid out for. Measured at
  1600 physical (1067 logical): the status bar overlaps itself and the toolbar
  clips its own buttons, the same class of problem `CLAUDE.md` records at 1267
  logical points.
* **`--lang de|en`** fixes the language for the run **without touching
  `%LOCALAPPDATA%\ctxmenu\settings.json`**. The script hashes that file before
  and after and refuses to report success if it changed. Both arguments were
  added for this script; before them, switching language meant writing the
  user's settings.
* **`--synthetic <n>`** fills the table with generated rows where the content
  does not matter. The generator is deterministic: same number, same rows, on
  any machine.
* **Waiting for the right line on stderr.** Views with a table report
  `startup_to_first_list_ms`; the four without one (programs, favourites,
  services, backups) never do and report only `window_placed`. Each entry says
  which line to wait for, then waits again for the icon worker.
* **Two strips are cut away before comparing**, because both change on their
  own and would report a difference on every picture:
  * the bottom 40 pixels, the status bar, which carries frames per second,
    frame time and startup milliseconds;
  * the right 16 pixels, the scroll bar, which egui fades in and out depending
    on how long ago the last event was. Measured: this alone accounted for
    every difference between two runs, a 10x1390 strip at x=2390, the same
    3065 pixels in all four pictures tested.

## Three things this had to get right on this machine

All three are recorded in `CLAUDE.md` and cost time before they were understood.

* **DPI first.** Four screens at 3840x2160 with 150% scaling. A script that does
  not call `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` sees them as
  2560x1440 and gets two thirds of the real coordinates out of `GetWindowRect`.
  The older `tools\capture_window.ps1` still does not, which is why its numbers
  are off by a factor of 1.5 here.
* **`PrintWindow` returns black** for this OpenGL window: measured, not one
  bright pixel. So the screen is copied instead, which means the window has to
  be raised first and lowered again afterwards.
* **Only the window's own rectangle is copied**, never the whole desktop. The
  other monitors have private things on them.

## Why it re-launches itself under Windows PowerShell

PowerShell 7 dropped `System.Drawing` from the box; it lives in the
`System.Drawing.Common` package now, and asking for `Bitmap` there fails with
CS1069. Windows PowerShell 5.1 still has it and ships with every Windows, so the
capture half runs there. The script detects `$PSVersionTable.PSEdition -eq
'Core'` and re-launches itself once via `-Command`, not `-File`: with `-File`
every argument arrives as one string, so `-Languages de,en` turns into the
literal `"de,en"` and fails its `ValidateSet`.

## What still needs work before a release

1. **The services tab shows an empty panel.** `--tab services` opens the list
   with the service selected by nobody, so the picture shows one name on the
   left and "pick a service" on the right. It is the tab that carries the
   program's most distinctive feature and currently its least useful picture.
   Needs an argument along the lines of `--service <id>` that selects and loads
   one, the way `--ext` already preselects an extension.
2. **No dialogs.** The editor, the confirmation before a write, and the About
   window are all reachable only by clicking. Any of them would make a better
   picture than a tab. Same shape of fix: an argument that opens one.
3. **The backups tab is full of test leftovers.** 1274 of the 1289 directories
   under `%LOCALAPPDATA%\ctxmenu\backups` came from test runs. `08-sicherungen`
   shows them above the user's real backups. Run
   `tools\backups_aufraeumen.ps1 -Apply` before taking the release set.
4. **Nothing is annotated.** For the README, some pictures want a callout or a
   cropped detail. ImageMagick is installed and the script already uses it for
   the comparison, so `magick ... -annotate` is a small step from here.
5. **The video is not started.** ffmpeg is installed. The pieces for a
   walkthrough are here (deterministic states, fixed window, both languages),
   but nothing yet drives a sequence and records it. The natural shape is a
   list of steps like the `$shots` list, with a duration per step.
6. **Only this machine.** Everything above was measured on four 3840x2160
   screens at 150%. A run on a single 1920x1080 screen at 100% has not been
   tried, and `--window 2400x1500` does not fit there.
