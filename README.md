# ctxmenu: Context Menu Manager

*[Deutsche Fassung](README_DE.md)*

A tool for the Windows right-click menu. It shows what is in it, where it sits
in the registry, and which program it belongs to, and it lets you hide
entries, show them only with the Shift key, sort them, delete them, and
create new ones. **Every change is backed up first**, not as a matter of
policy, but because the delete function cannot be invoked at all without
proof of a backup.

It also works the other way: custom entries, submenus, a toolbox of programs
and web services, all the way to **two hundred menu entries from a single
address** when a web application describes itself through OpenAPI.

Windows 10 and 11, 64-bit. A single `.exe` with no installation and no
runtime library.

*Interface in German and English, switchable at runtime; this README is
English only.*

---

## What It Can Do

- **See everything.** The seven base categories (files, folders, folder
  background, desktop, drives, ...) across three registry areas: `HKCU`,
  `HKLM`, and the 32-bit view `WOW6432Node`. Static verbs and COM handlers
  are shown separately. Plus Windows' own **verb store** (`CommandStore`):
  229 verbs on this machine that appear in no menu until another entry names
  them in its `SubCommands` list. Read-only, marked with a lock.
- **Resolve file types.** For an extension like `.jpg`, the complete chain
  from user choice, ProgID, `PerceivedType`, and `SystemFileAssociations`: in
  other words, what the right-click actually shows, not what is registered
  at one single spot.
- **Custom file extensions and the full scan.** The *File Types* tab shows a
  curated selection of 98 types; a field above it accepts any further
  extension, which then stays saved. Anyone who wants to see everything
  presses *All installed*: on a machine that has grown over the years, that
  is around 1700 types instead of 98, and reading them in takes
  correspondingly longer.
- **Group by program.** A program that registers itself in twenty file types
  appears as **one** group with every occurrence, with its icon in front.
  The name comes from the `.exe`'s version resource, not from the key name.
  **If an entry points to a program that no longer exists, the row shows in
  red**; this happens mainly after updates to Store apps, whose folder
  carries the version number in its name.
- **Change, in four steps from mild to severe:** hide (`LegacyDisable`), show
  only with the Shift key (`Extended`), set the position to top or bottom,
  block a COM handler machine-wide, delete.
- **Create your own entries** with a display name, command, icon, position,
  and Shift-key visibility. Always in `HKCU`, so without administrator
  rights and without risk to other accounts.
- **Also as a submenu.** Instead of a command, the entry gets a list of
  child entries that expands within the menu. The order in the form is the
  order in the menu: Windows sorts registry keys alphabetically, so the
  tool numbers them as it writes them.
- **Favourites: the toolbox.** Enter a program or a web tool once, and it
  stays. From there, one click places it in any category or for a
  specific file type. From the *Programs* tab, a program that keeps showing
  up anyway moves into the list with one click.
- **Web tools as a favourite.** A favourite does not have to be an `.exe`;
  an address is enough. Because a web page is not allowed to read a local
  file, it gets *sent* instead; more on that below.
- **Services: a hundred tools from one address.** If a web application
  describes itself through OpenAPI, the address of its documentation page
  is enough. The program looks for the machine-readable document behind it,
  reads out which endpoints accept a file, groups them the way the service
  itself groups them, and turns every checked one into a favourite. If a
  tool accepts settings, a form is generated for it, even when the service
  describes its options only as running text.
- **Drag and drop.** Dragging an `.exe` into the window creates an entry
  from it; which category it is dropped on decides where it lands. In the
  editor, the fields for command and icon also accept a dropped file.
- **Back up and restore.** Every action creates a backup beforehand, a group
  action exactly one backup for the whole group. The *Backups* tab shows the
  history and plays it back, and it has a **Back Up Everything** button that
  takes along every location this tool touches at all (on this machine, 1.2
  MB in under a second).
- **Submenus** are shown with their children, indented under the entry they
  hang from.
- **Look at an entry:** double-click a row, or right-click and choose *Look
  at this entry*, opens the form with everything that is actually in the
  registry. Nothing can be changed there yet.
- German and English, light and dark, or "Follow system", both without a
  restart; the title bar follows along.

## What It Deliberately Cannot Do

- **Change the text of a COM handler.** That text is generated at runtime in
  `IContextMenu::QueryContextMenu` and appears nowhere in the registry.
  What is shown is the key name, the CLSID's plain-text name, and the DLL
  behind it.
- **Rebuild the new Windows 11 main menu.** The tool works on the classic
  menu ("Weitere Optionen anzeigen", i.e. "Show more options"), which
  Windows 11 continues to run in full.
- **Freely determine the order.** Windows sorts subkeys alphabetically and
  only knows the coarse blocks `Position=Top` and `Position=Bottom`. Both
  have been measured; the system does not offer anything more.

---

## Getting Started

There is no installer. Starting the `.exe` is enough.

```
ctxmenu.exe
```

Without arguments, the window opens. Entirely without administrator rights:
they are only requested once a change actually needs them, and then only
for that one step.

### The Six Tabs

| Tab | Purpose |
|---|---|
| **Categories** | The starting point: what appears when you right-click a folder, a file, the desktop |
| **File Types** | Choose an extension and see the full resolution chain |
| **Programs** | Grouped by program: the fastest way to take a program out of the menu entirely |
| **Favourites** | Your own toolbox: enter it once, and it stays there |
| **Services** | Pull tools in from a web application's self-description |
| **Backups** | History of every backup, with a button to restore |

The search field works on every tab and searches the display name, the
command, and the registry path, even when nothing is selected on the left
yet.

**In the list:** the arrow keys move the selection, Home and End jump to the
start and the end, holding Shift grows the selection, and Ctrl+A selects
everything. Clicking a column header sorts by it, a second click reverses
the direction. The **Appears On** column says in words where an entry shows
up: "All Files" instead of `*`, ".zip" instead of a path with
`SystemFileAssociations` in the middle; the real registry path sits in the
tooltip. A **right-click** offers, everywhere, exactly the actions that
would change something about the item clicked, and in the empty area, *New*.

**The action bar** above the table is not a set of buttons but four
switches: *In the menu* (visible <-> hidden), *Shift key* (always <-> only
with ⇧), *Machine-wide* (free <-> blocked), and *Position*. Highlighted is
where the selection currently stands; clicking the other side moves it
there. "Which button do I press?" becomes "where should it go?". Whatever
is not currently possible is greyed out and says why in the tooltip: for
instance, that none of the selected entries is a COM handler, so there is
nothing to block.

**Restart Explorer** sits at the top of the bar. Windows reads the context
menu keys when Explorer starts; an entry that absolutely refuses to show up
needs this.

### A Typical Round

1. **Programs** tab, click the program that is causing trouble.
2. Check on the right what depends on it: path, command, scope.
3. **Hide** instead of delete. That is reversible and is almost always
   enough.
4. If Windows asks for administrator rights: those are the entries under
   `HKLM`, the ones for all accounts. Anyone who declines keeps the changes
   to their own entries; the others stay as they were.

---

## Favourites and Web Tools

The **Favourites** tab is a list that stays. Whatever is in it once can be
placed at another spot in the context menu at any time, without setting it
up again. "Add to menu" only asks for the where: one of the base
categories, a file extension (`.png`), or an entire kind of file (`image`
covers every image format Windows knows).

A favourite does not have to be a program. If the tool lives in the
browser, there is a problem that no registry solves: **a web page is not
allowed to read a local file.** An address like
`https://tool.example/?f=C:\bild.png` opens the page, but the file never
arrives there: no browser allows that, and that is a good thing. So the
file has to be sent, and that takes a sender. That sender is this program:
the menu entry calls `ctxmenu --favourite <id> "%1"` and then, depending on
the mode, does one of three things.

**Clipboard**: the file lands on the clipboard, the page opens, and a
Ctrl+V in the browser is enough. This is the way for anything that offers
no interface at all: Squoosh, the TinyPNG page, remove.bg. No key, no
endpoint, and it works even with tools that never planned for this. For a
PNG, the image itself is additionally placed on the clipboard, so pages
that expect an image rather than a file are satisfied too.

**Upload**: for tools with a real endpoint. The file goes out via
`multipart/form-data` (field name configurable) or as a raw body; header
lines for a key can be attached. What comes back is, depending on the
setting, saved next to the original file (`bild.png` → `bild.min.png`; the
original is **never** overwritten), opened in the browser, or just
reported. The result address may be given in the `Location` header or in a
JSON field such as `output.url`.

**Open address**: builds the address from placeholders and opens it without
transmitting anything. `{name}`, `{stem}`, `{ext}`, `{path}`, `{dir}`, and
`{fileurl}`, all correctly encoded. For search, wiki, ticket forms.

**You are asked before the first upload.** Once per tool, stating the
destination and the file size; the answer is remembered. The program
refuses unencrypted `http://` unless it has been explicitly allowed for
this favourite: sending a file across the network in the clear is meant to
be a decision, not a default. WinHTTP handles the transfer, which is to say
Windows' own client: with the system certificate store and the proxy
settings that already apply anyway.

---

## Services: A Hundred Tools, One Address

Setting up a favourite by hand means filling out six fields. For a
self-hosted service with two hundred tools, that is not work anyone is
going to do.

The **Services** tab therefore takes the address you already have open in
the browser: the API documentation, anchor included:

```
http://192.168.x.y:1349/api/docs/#tag/tools
```

The program strips off the anchor and looks for the machine-readable
document behind it: the page itself, then `openapi.json`, `swagger.json`,
and the other usual locations, both from this path and from the root. The
status code decides nothing here: a documentation page answers with 200
just as the document does. Whether the response can be read as JSON is the
criterion.

Everything that can be read is then read out of the description:

- **Which endpoints even come into question**: the ones that accept a file
  as `multipart/form-data`. A right-click cannot serve anything else.
- **How they belong together.** Not by the OpenAPI `tag`: for many services
  that is the same for everything. Instead, every possible grouping
  competes against every other one, the tag and each segment of the path,
  and whichever produces the most useful menu wins. For a service with 232
  tools, this yields *Image, Video, PDF, Audio, Files* instead of a single
  drawer called "Tools" with 225 entries in it.
- **What a tool accepts besides the file.** If the description supplies a
  schema, a form with typed fields is built from it: a number with its
  allowed range, a checkbox, a dropdown list. If it supplies none, only
  prose (the more common case), that gets read too, as long as it lists its
  fields:

  ```
  JSON string with options:
  - `left` (number, required) - Left offset in pixels (min 0)
  - `unit` (string, optional) - One of: px, percent
  ```

  The same kind of form results from this. Where the prose is not
  unambiguous, it stays a text field with the description above it: better
  no field than a wrong one, because a wrong one sends nonsense to a real
  service.
- **What would not work.** Endpoints that answer only with a job number and
  keep working in the background do not appear in the list: an entry made
  from one would report success and save nothing. Their count is shown
  anyway, with a button that reveals them.

You check items individually or by category, and create them in one batch.
What a service says about itself then lives in every favourite;
**the address and the key stay local** in
`%LOCALAPPDATA%\ctxmenu\services.json` and go nowhere.

No description can supply two fields, because they depend on the
installation: where in the response the finished file is named, and
whether unencrypted `http://` should be allowed. Templates exist for that:
one click fills them in, and the address and the key remain yours.

---

## From the Command Line

The same application is also a diagnostic tool. Output goes to the console
it was started from.

```
ctxmenu scan --category directory        entries in one category
ctxmenu scan --all-types --json          full scan including file types, as JSON
ctxmenu scan --every-type                every registered extension instead of the curated list
ctxmenu filetype .jpg                    resolution chain of one file type
ctxmenu programs                         group by program
ctxmenu hide "<key>" --yes               hide, with a backup
ctxmenu delete "<key>" --yes             delete, with a backup
ctxmenu backups                          list backups
ctxmenu backup-all                       back up everything this tool touches
ctxmenu restore "<path>"                 restore a backup
ctxmenu create --category directory --name "Mit Editor öffnen"
               --command "\"C:\Windows\notepad.exe\" \"%1\""
ctxmenu create --category directory --name "Werkzeuge"
               --sub "Öffnen|\"C:\Windows\notepad.exe\" \"%1\""
               --sub "Anzeigen|cmd /c dir \"%1\" & pause"
                                         a submenu instead of a single command
ctxmenu created                          list entries created by this tool
ctxmenu favourites                       list favourites
ctxmenu favourite add --name "PNG verkleinern"
        --url https://squoosh.app --mode clipboard
ctxmenu favourite place <id> --ext .png
ctxmenu favourite run <id> <file>        run it like a click
ctxmenu --tab services                   open the window on a specific tab
ctxmenu --version                        which version this is
ctxmenu --help                           the complete list
```

A note on creating entries: in the background categories (folder
background, desktop), `%1` stays **empty**. `%V` belongs there instead. The
tool warns about this, because an entry that does nothing looks exactly
like an entry that works.

---

## Where the Backups Live

```
%LOCALAPPDATA%\ctxmenu\backups\<timestamp>_<action>\
    manifest.json      what was backed up, when, and what was missing
    01_….reg           one file per key, written by reg.exe
%LOCALAPPDATA%\ctxmenu\entries.json     entries you created yourself
%LOCALAPPDATA%\ctxmenu\favourites.json  the toolbox
%LOCALAPPDATA%\ctxmenu\services.json    registered services, including their keys
%LOCALAPPDATA%\ctxmenu\settings.json    language and appearance
```

The keys in `favourites.json` and `services.json` sit there in plain text,
protected only by the permissions on your user profile, the same as in an
`.npmrc` or a `.gitconfig`. Anyone who does not want that should use a
separate key with restricted rights for this program.

The `.reg` files are ordinary registration files: they can be restored by
double-clicking even without this tool. That has one limit: `reg import`
adds and overwrites, it **removes nothing**. After a deletion, it restores
exactly the previous state; played back over a key that has since changed,
that key's new values remain in place.

Das Programm selbst geht einen Schritt weiter. Schlüssel, die es beim Sichern
noch gar nicht gab, stehen im `manifest.json` unter `absent` und werden beim
Zurückspielen wieder **entfernt** — anders ließe sich ein Blockieren nicht
rückgängig machen, denn die Blocked-Liste liefert Windows nicht mit, sie
entsteht erst mit dem ersten blockierten Handler. Für die Gesamtsicherung gilt
das ausdrücklich nicht: sie umfasst ganze Zweige wie `Directory\shell`, in die
auch jedes andere Programm schreibt, und nimmt beim Zurückspielen nichts weg.

Ein Zurückspielen bricht nicht mehr beim ersten fehlenden Schlüssel ab: jeder
Eintrag wird versucht, und am Ende steht, wie viele zurück sind und welche
nicht. Eine geteilte Aktion — ein Teil hier, ein Teil mit Administratorrechten —
legt zwei Sicherungen an; das Ergebnisfenster nennt beide und der Knopf
*Wiederherstellen* spielt beide ein.

---

## Building It Yourself

Required are Rust 1.95 and the Visual Studio Build Tools with the C++
toolchain.

```powershell
cargo build --release
cargo test
```

The result is located at
`target\x86_64-pc-windows-msvc\release\ctxmenu.exe`, not at
`target\release\`, because `.cargo\config.toml` names the target
explicitly. That is deliberate: only this way does the statically linked C
runtime apply to the application and not also to the compiler's macro
libraries. The finished file therefore needs no "Visual C++
Redistributable", verified on a freshly installed Windows 10 with no
additional software at all.

Deferred plans, the development status, the measured values, and the
places where Windows behaves differently than documented are kept by the
author in notes that are not part of this repository.
