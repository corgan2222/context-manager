# ctxmenu

*[Deutsche Fassung](FEATURES_DE.md)*

A manager for the Windows right-click menu. It shows what is in it and where
it sits in the registry, removes entries that get in the way, and adds its
own: up to two hundred menu items generated from a web service's own
self-description.

Windows 10 and 11, 64-bit. A single `.exe` of 6.5 MB, no installer, no
runtime library, no background service.

---

## Reading

**Seven categories, three registry areas.** Files, folders, folder
background, desktop background, drives, filesystem objects, and the shell
namespace: read from `HKCU`, `HKLM`, and the 32-bit view `WOW6432Node`. On a
machine that has grown over time, that comes to around 930 entries.

**Static verbs and COM handlers, kept separate.** A verb is a key with a
command; a COM handler is a CLSID backed by a DLL. For the handler, the
program shows the CLSID's display name and the DLL behind it.

**Windows' own stock of verbs.** The `CommandStore` on this machine holds 229
verbs that appear in no menu until another entry names them in its
`SubCommands` list. They show up in the list with a lock icon, read-only.

**The resolution chain of a file type.** For `.jpg` that is seven levels:
user choice, ProgID, `PerceivedType`, `SystemFileAssociations`, and the
general entries. The program shows what the right-click actually displays,
not what is registered at any one location. For `.jpg` that comes to 58
entries, 39 of which apply to every file.

**Grouped by program.** A program registered in twenty file types appears as
a single group with all its occurrences and its icon in front. The name comes
from the `.exe`'s version resource, not from the key name. If an entry points
to a program that no longer exists, the row shows in red: that happens after
updates to Store apps, whose folder carries the version number in its name.

**Search.** A field above all tabs searches display name, command, and
registry path, even when nothing is selected on the left.

---

## Changing

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

**The action bar consists of four switches**, not nine buttons. *In menu*
(visible ↔ hidden), *Shift key* (always ↔ Shift-only), *System-wide* (free ↔
blocked), and *Position*. Whichever side the selection is on is highlighted;
clicking the other side moves it there. With a mixed selection, nothing
lights up, and the tooltip gives the counts. Whatever is currently
unavailable is grayed out and says why: for instance, that none of the
selected entries has a CLSID that could be blocked.

---

## Creating

**Custom entries** with display name, command, icon, position, and Shift
visibility. Always in `HKCU`, so without administrator rights and without
effect on other accounts.

**Submenus.** Instead of a command, the entry gets a list of sub-entries. The
order in the form is the order in the menu: Windows sorts registry keys
alphabetically, so the program numbers them when writing.

**Drag and drop.** Dragging an `.exe` into the window creates an entry with
it; the category it is dropped over decides where it lands. In the editor,
the command and icon fields also accept a dropped file.

**A check before writing.** In the background categories, `%1` stays empty:
`%V` belongs there instead. The program says so in advance, because an entry
that does nothing looks just like one that works.

---

## Favorites

A list that persists. Whatever is in it can be added, with one click, to
another place in the context menu: to a base category, to a file extension
(`.png`), or to an entire kind of file (`image` covers every image format
Windows knows).

A favorite does not have to be a program. If the tool lives in the browser,
there is an obstacle in the way that no registry can remove: **a web page
cannot read a local file.** `https://tool.example/?f=C:\bild.png` opens the
page, but the file never arrives. It has to be sent, then, and that needs a
sender. The menu entry calls `ctxmenu --favourite <kennung> "%1"` and then
does one of three things:

**Clipboard.** The file lands on the clipboard, the page opens, and Ctrl+V in
the browser is enough. The path for anything without an interface: Squoosh,
remove.bg. For a PNG, the image itself also goes on the clipboard, so pages
that expect an image rather than a file are satisfied too.

**Upload.** The file goes out as `multipart/form-data` or plain as the body,
and header lines for a key can be included. Whatever comes back is saved next
to the original file (`bild.png` → `bild.min.png`; the original is never
overwritten), opened in the browser, or just reported. The result address may
be given in the `Location` header or in a JSON field such as `output.url`.

**Open address.** Builds an address from `{name}`, `{stem}`, `{ext}`,
`{path}`, `{dir}`, and `{fileurl}` and opens it without transmitting
anything. For search, wiki, ticket forms.

Before the first upload, the program asks, once per tool, stating the
destination and file size. It refuses unencrypted `http://` until it has been
explicitly allowed for that favorite. WinHTTP handles the transfer, meaning
Windows's own client, with the system certificate store and the active proxy
settings.

---

## Services

Setting up a favorite by hand means filling in six fields. Nobody does that
for a service with two hundred tools.

The **Services** tab therefore takes the address that is already open in the
browser: the API documentation, anchor included. The program strips the
anchor and looks for the machine-readable document behind it: the page
itself, then `openapi.json`, `swagger.json`, `/v3/api-docs`, and the other
usual locations. The status code decides nothing here, because a
documentation page answers with 200 just as much as the document does.
Whether the response can be read as JSON is the criterion.

**Which endpoints qualify:** the ones that accept a file as
`multipart/form-data`. On a test service with 351 paths, that is 232.

**How they belong together.** Not by the OpenAPI `tag`: for many services,
that reads the same for almost everything. Instead, every possible grouping
competes against every other, the tag and each segment of the path, measured
against four criteria: how much of the service ends up in usable groups, how
evenly, how close the group count is to the square root of the total, and
whether the tool names repeat the group word. On the test service, this makes
*Image, Video, PDF, Audio, Files* win against a catch-all "Tools" bucket with
225 entries in it, by a factor of 17.

**What a tool accepts besides the file.** If the description provides a
schema, a form with typed fields results: a number with the allowed range
shown in the empty field, a checkbox, a dropdown. If it provides none, only
prose, that gets read too, as long as it lists its fields:

```
JSON string with options:
- `left` (number, required) - Left offset in pixels (min 0)
- `unit` (string, optional) - One of: px, percent
```

That produces the same kind of form. On the test service, 113 of 227 options
descriptions yield a form, 431 fields in total. Where the prose is not
unambiguous, it stays a text field: a misidentified field sends nonsense to a
real service, an overlooked one costs a checkbox.

**What would not work is not in the list.** Endpoints that answer only with a
job number and keep working in the background would produce an entry that
reports success and saves nothing. On the test service, that is 52 of 232.
Their count is still shown, though, with a button that reveals them.

Checking happens one at a time or by category, and creation happens in one
batch. Every tool also carries a link to its place in the service's
documentation.

The address and key stay in `%LOCALAPPDATA%\ctxmenu\services.json` and go
nowhere else.

---

## Backing Up and Restoring

Every action creates a backup beforehand. The **Backups** tab shows the
history, states what each one contains, and plays it back.

A **Back up everything** button captures every location this program touches
at all: on this machine, 26 of 46 keys, 1.2 MB, under one second. The
remaining 20 do not exist here, 15 of them in the empty 32-bit view.

```
%LOCALAPPDATA%\ctxmenu\backups\<timestamp>_<action>\
    manifest.json      what was backed up, when, and what was missing
    01_….reg           one file per key, written by reg.exe
```

The `.reg` files are ordinary registration files and can be played back with
a double-click, even without this program. That has a limit: `reg import`
adds and overwrites, it removes nothing. After a delete, it restores the old
state; played back over a key that has since changed, that key's new values
remain.

---

## Usage

- **Keyboard in the list:** arrow keys, Home and End, Shift for a range,
  Ctrl+A for everything.
- **Right-click** offers, everywhere, the actions that would change something
  for what was clicked. With a multi-selection, the ones that only make sense
  for a single entry drop out. In the empty area, it shows *New*.
- **Sorting** by clicking a column header; a second click reverses it.
- **The "Appears on" column** states in words where an entry shows up: "All
  Files" instead of `*`, ".zip" instead of a path with
  `SystemFileAssociations` in the middle. The real path is in the tooltip.
- **Restart Explorer** as a button in the top bar. Windows reads the
  context-menu keys when Explorer starts.
- **German and English**, light and dark, or "Follow system". Both switch
  without a restart; the title bar follows along.
- **An error log** at `%LOCALAPPDATA%\ctxmenu\ctxmenu.log`, linked from the
  About window. It contains every error shown and every crash.

---

## Command Line

The same `.exe` is also a diagnostic tool. Output goes to the console it was
started from.

```
ctxmenu scan --category directory        entries of one category
ctxmenu scan --all-types --json          full scan including file types, as JSON
ctxmenu scan --every-type                every registered extension instead of the selection
ctxmenu filetype .jpg                    resolution chain of an extension
ctxmenu programs                         grouped by program
ctxmenu hide "<key>" --yes               hide, with backup
ctxmenu delete "<key>" --yes             delete, with backup
ctxmenu backups                          list backups
ctxmenu backup-all                       back up everything the tool touches
ctxmenu restore "<directory>"            restore a backup
ctxmenu create --category directory --name "Mit Editor öffnen"
               --command "\"C:\Windows\notepad.exe\" \"%1\""
ctxmenu created                          list custom entries
ctxmenu favourites                       list favourites
ctxmenu favourite run <id> <file>        run as if clicked
ctxmenu --tab dienste                    open the window on a tab
ctxmenu --version
ctxmenu --help
```

---

## What It Cannot Do

**Change the text of a COM handler.** That text is generated at runtime in
`IContextMenu::QueryContextMenu` and appears nowhere in the registry. What is
shown instead is the key name, the CLSID's display name, and the DLL behind
it.

**Rebuild the Windows 11 main menu.** The program works on the classic menu
("Weitere Optionen anzeigen", Show more options), which Windows 11 still
fully maintains.

**Set the order freely.** Windows sorts subkeys alphabetically and only knows
the coarse blocks `Position=Top` and `Position=Bottom`. Measured directly;
the system does not offer more than that.

**Edit scanned entries.** The form shows everything that is in the registry
but does not yet write anything back. Custom entries are not affected by
this.

---

## Building

Rust 1.95, Visual Studio Build Tools with the C++ toolchain.

```powershell
cargo build --release
cargo test
```

The result ends up at `target\x86_64-pc-windows-msvc\release\ctxmenu.exe`,
not under `target\release\`: `.cargo\config.toml` names the target
explicitly, so the statically linked C runtime applies to the application and
not also to the compiler's macro libraries. The finished file therefore needs
no "Visual C++ Redistributable": verified on a freshly installed Windows 10
with no additional software.

336 tests, `cargo clippy -- -D warnings` clean.
