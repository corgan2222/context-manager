# ctxmenu: Windows Context Menu Manager

*[Deutsche Fassung](docs/README_DE.md)*

A tool for the Windows right-click menu. It shows what is in it, where it sits
in the registry, and which program it belongs to, and it lets you hide
entries, show them only with the Shift key, sort them, delete them, and
create new ones. **Every change is backed up first**, not as a matter of
policy, but because the delete function cannot be invoked at all without
proof of a backup.

It also works the other way: custom entries, submenus, a toolbox of programs
and web services, all the way to **two hundred menu entries from a single
address** when a web application describes itself through OpenAPI.

Windows 10 and 11, 64-bit. A single `.exe` written in Rust, no installation, no
runtime library, no background service.

*Interface in German and English, switchable at runtime; this README is
English only.*

---

## What It Can Do

- **See everything.** The seven base categories (files, folders, folder
  background, desktop background, drives, filesystem objects, and the shell
  namespace) across three registry areas: `HKCU`, `HKLM`, and the 32-bit view
  `WOW6432Node`. On a machine that has grown over the years, that comes to
  around 930 entries. Static verbs and COM handlers are shown separately: a
  verb is a key with a command, a COM handler is a CLSID backed by a DLL, and
  for the handler the program shows the CLSID's plain-text name and the DLL
  behind it. Plus Windows' own **verb store** (`CommandStore`): 229 verbs on
  this machine that appear in no menu until another entry names them in its
  `SubCommands` list. Read-only, marked with a lock.
- **Resolve file types.** For an extension like `.jpg`, the complete chain
  from user choice, ProgID, `PerceivedType`, and `SystemFileAssociations`,
  seven levels in all: in other words, what the right-click actually shows,
  not what is registered at one single spot. For `.jpg` that comes to 58
  entries, 39 of which apply to every file.
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
  registry.
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
- **Edit scanned entries.** The form shows everything that is in the
  registry but does not yet write anything back. Entries you created
  yourself are not affected by this.

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

### In the List

The arrow keys move the selection, Home and End jump to the start and the
end, holding Shift grows the selection, and Ctrl+A selects everything.
Clicking a column header sorts by it, a second click reverses the
direction. The **Appears On** column says in words where an entry shows
up: "All Files" instead of `*`, ".zip" instead of a path with
`SystemFileAssociations` in the middle; the real registry path sits in the
tooltip. A **right-click** offers, everywhere, exactly the actions that
would change something about the item clicked; with a multi-selection, the
ones that only make sense for a single entry drop out, and in the empty
area, *New*.

**Restart Explorer** sits in the top bar. Windows reads the context menu
keys when Explorer starts; an entry that absolutely refuses to show up
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

## Changing an Entry

Four levels, from gentle to hard:

| Level | What happens | Reversible |
|---|---|---|
| **Hide** | `LegacyDisable` at one location | yes |
| **Shift-only** | `Extended`, the entry appears on Shift+right-click | yes |
| **Position** | `Position=Top` or `Bottom` | yes |
| **System-wide block** | The CLSID goes on the block list, for all accounts | yes |
| **Delete** | The key disappears | only from the backup |

**Every change is backed up first.** That is not just a stated intent:
`delete_tree` requires a token, and a token is created only as the return
value of a successful backup. Without a backup, the delete function cannot be
called.

**One backup per group action**, not one per entry. Hiding twenty entries at
once creates one directory, not twenty.

**Elevated rights only when needed.** Entries under `HKLM` require them; the
program asks for exactly that step and restarts itself once for it. Anyone
who declines keeps the changes to their own entries.

**The action bar** above the table is not a set of buttons but four
switches: *In the menu* (visible <-> hidden), *Shift key* (always <-> only
with ⇧), *Machine-wide* (free <-> blocked), and *Position*. Highlighted is
where the selection currently stands; clicking the other side moves it
there. "Which button do I press?" becomes "where should it go?". With a
mixed selection, nothing lights up, and the tooltip gives the counts.
Whatever is not currently possible is greyed out and says why in the
tooltip: for instance, that none of the selected entries is a COM handler,
so there is nothing to block.

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
`/v3/api-docs`, and the other usual locations, both from this path and from
the root. The status code decides nothing here: a documentation page answers
with 200 just as the document does. Whether the response can be read as JSON
is the criterion.

Everything that can be read is then read out of the description:

- **Which endpoints even come into question**: the ones that accept a file
  as `multipart/form-data`. A right-click cannot serve anything else. On a
  test service with 351 paths, that is 232.
- **How they belong together.** Not by the OpenAPI `tag`: for many services
  that is the same for almost everything. Instead, every possible grouping
  competes against every other one, the tag and each segment of the path,
  measured against four criteria: how much of the service ends up in usable
  groups, how evenly, how close the group count is to the square root of the
  total, and whether the tool names repeat the group word. For a service
  with 232 tools, this yields *Image, Video, PDF, Audio, Files* instead of a
  single drawer called "Tools" with 225 entries in it, and it wins by a
  factor of 17.
- **What a tool accepts besides the file.** If the description supplies a
  schema, a form with typed fields is built from it: a number with its
  allowed range shown in the empty field, a checkbox, a dropdown list. If it
  supplies none, only prose (the more common case), that gets read too, as
  long as it lists its fields:

  ```
  JSON string with options:
  - `left` (number, required) - Left offset in pixels (min 0)
  - `unit` (string, optional) - One of: px, percent
  ```

  The same kind of form results from this. On the test service, 113 of 227
  options descriptions yield a form, 431 fields in total. Where the prose is
  not unambiguous, it stays a text field with the description above it:
  better no field than a wrong one, because a wrong one sends nonsense to a
  real service, while an overlooked one costs a checkbox.
- **What would not work.** Endpoints that answer only with a job number and
  keep working in the background do not appear in the list: an entry made
  from one would report success and save nothing. On the test service, that
  is 52 of 232. Their count is shown anyway, with a button that reveals
  them.

You check items individually or by category, and create them in one batch.
What a service says about itself then lives in every favourite, and every
tool carries a link to its place in the service's documentation;
**the address and the key stay local** in
`%LOCALAPPDATA%\ctxmenu\services.json` and go nowhere.

No description can supply two fields, because they depend on the
installation: where in the response the finished file is named, and
whether unencrypted `http://` should be allowed. Templates exist for that:
one click fills them in, and the address and the key remain yours.

---

## From the Command Line

The same application is also a diagnostic tool. Output goes to the console
it was started from, in the language the window is set to — put `--lang en`
or `--lang de` in front of the command to change that for one run.

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
ctxmenu --service snapotter              services tab, that service selected and loaded
ctxmenu --new directory                  the editor for a new entry, filled with an example
ctxmenu --lang en scan                   this run in English, saved setting untouched
ctxmenu --version                        which version this is
ctxmenu --help                           the complete list
```

A note on creating entries: in the background categories (folder
background, desktop), `%1` stays **empty**. `%V` belongs there instead. The
tool warns about this, because an entry that does nothing looks exactly
like an entry that works.

---

## Backing Up and Restoring

Every action creates a backup beforehand. The **Backups** tab shows the
history, states what each one contains, and plays it back. The **Back Up
Everything** button captures every location this program touches at all: on
this machine, 26 of 46 keys, 1.2 MB, in under a second. The remaining 20 do
not exist here, 15 of them in the empty 32-bit view.

```
%LOCALAPPDATA%\ctxmenu\backups\<timestamp>_<action>\
    manifest.json      what was backed up, when, and what was missing
    01_….reg           one file per key, written by reg.exe
%LOCALAPPDATA%\ctxmenu\entries.json     entries you created yourself
%LOCALAPPDATA%\ctxmenu\favourites.json  the toolbox
%LOCALAPPDATA%\ctxmenu\services.json    registered services, including their keys
%LOCALAPPDATA%\ctxmenu\settings.json    language and appearance
%LOCALAPPDATA%\ctxmenu\ctxmenu.log      every error shown and every crash
```

The log is linked from the About window.

One more file is written, and it is the only one outside that folder:

```
%APPDATA%\Microsoft\Windows\Start Menu\Programs\ctxmenu.lnk
```

A shortcut to the running `.exe`, created the first time a favourite reports
its result. Windows only draws a notification of a desktop program on screen
if a Start menu shortcut carries the same identifier the notification was sent
under; without it the message is filed silently. Delete it and nothing breaks:
the result of every favourite still reaches the Action Center either way, and
the next run writes the shortcut again. It is also rewritten whenever the
`.exe` moves, so the entry never points at a file that is no longer there.

The keys in `favourites.json` and `services.json` sit there in plain text,
protected only by the permissions on your user profile, the same as in an
`.npmrc` or a `.gitconfig`. Anyone who does not want that should use a
separate key with restricted rights for this program.

The `.reg` files are ordinary registration files: they can be restored by
double-clicking even without this tool. That has one limit: `reg import`
adds and overwrites, it **removes nothing**. After a deletion, it restores
exactly the previous state; played back over a key that has since changed,
that key's new values remain in place.

The program itself goes one step further. Keys that did not exist at all at
the time of the backup are listed in `manifest.json` under `absent` and are
**removed** again on restore. There is no other way to undo a block, because
Windows does not ship the blocked list at all: it comes into being with the
first blocked handler. For the full backup this expressly does not apply: it
covers whole branches such as `Directory\shell`, which every other program
writes into as well, and it takes nothing away on restore.

A restore no longer stops at the first missing key: every entry is
attempted, and at the end it says how many are back and which are not. A
split action, one part here and one part with administrator rights, creates
two backups; the result window names both, and the *Restore* button plays
back both.

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

336 tests, `cargo clippy -- -D warnings` clean.

Deferred plans, the development status, the measured values, and the
places where Windows behaves differently than documented are kept by the
author in notes that are not part of this repository.

---

## Contributing, Security, Licence

- [Contributing](docs/CONTRIBUTING.md) ([deutsch](docs/CONTRIBUTING_DE.md))
- [Security policy](docs/SECURITY.md) ([deutsch](docs/SECURITY_DE.md))
- [Code of conduct](docs/CODE_OF_CONDUCT.md)
- [Third-party notices](docs/THIRD-PARTY-NOTICES.md)
  ([deutsch](docs/THIRD-PARTY-NOTICES_DE.md))
- [MIT licence](LICENSE)
