# ctxmenu: Windows Context Menu Manager

*[Deutsche Fassung](docs/README_DE.md)*

A tool for the Windows right-click menu. It shows what is in it, where it sits
in the registry, and which program it belongs to, and it lets you hide
entries, show them only with the Shift key, sort them, delete them, and
create new ones. **Every change is backed up first**, not as a matter of
policy, but because the delete function cannot be invoked at all without
proof of a backup.

It also works the other way: custom entries, submenus, a toolbox of programs
and web services, all the way to **two hundred tools from a single
address** when a web application describes itself through OpenAPI.

Windows 10 and 11, 64-bit. A single `.exe` written in Rust, no installation, no
runtime library, no background service.

*Interface in German and English, switchable at runtime; this README is
English only.*

![The Categories tab, showing 927 context menu entries across seven base categories](docs/images/01-overview_en.web.png)

*The starting point on a machine that has grown over the years: 927 entries,
131 of them in the seven base categories, another 229 in Windows' own verb
store. Static verbs and COM handlers stand side by side; the padlock marks
what cannot be changed without administrator rights.*

---

## What It Can Do

- **See everything.** The seven base categories (files, folders, folder
  background, desktop background, drives, filesystem objects, and the shell
  namespace) across three registry areas: `HKCU`, `HKLM`, and the 32-bit
  view `WOW6432Node`. On a machine that has grown over the years, those
  seven come to about 130 entries; resolve every file type as well and the
  whole scan reaches around 930. Static verbs and COM handlers are shown
  separately: a verb is a key with a command, a COM handler is a CLSID
  backed by a DLL, and for the handler the program shows the CLSID's
  plain-text name and the DLL behind it. Plus Windows' own **verb store**
  (`CommandStore`): 229 verbs on this machine that appear in no menu until
  another entry names them in its `SubCommands` list. Read-only, marked with
  a lock.
- **Resolve file types.** For an extension like `.jpg`, the complete chain
  from user choice, ProgID, `PerceivedType`, and `SystemFileAssociations`,
  seven levels in all: in other words, what the right-click actually shows,
  not what is registered at one single spot. For `.jpg` that comes to just
  under sixty entries, two thirds of which apply to every file — and because
  those two thirds are the same for every extension, the tab leaves them out
  until *Include entries for all files* asks for them.
- **Custom file extensions and the full scan.** The *File Types* tab shows a
  curated selection of 98 types; a field above it accepts any further
  extension, which then stays saved. Anyone who wants to see everything
  presses *All installed*: on a machine that has grown over the years, that
  is well over a thousand types instead of 98 -- 1674 on the machine this
  was written on -- and reading them in takes
  correspondingly longer.
- **Group by program.** A program that registers itself in twenty file types
  appears as **one** group with every occurrence, with its icon in front.
  The name comes from the `.exe`'s version resource, not from the key name.
  **If an entry points to a program that no longer exists, the row shows in
  red**; this happens mainly after updates to Store apps, whose folder
  carries the version number in its name.
- **Change, in five steps from mild to severe:** hide (`LegacyDisable`), show
  only with the Shift key (`Extended`), set the position to top or bottom,
  block a COM handler machine-wide, delete.
- **Create your own entries** with a display name, command, icon, position,
  and Shift-key visibility — for a base category, for a single extension, or
  for a whole kind of file. Browse buttons beside the command and icon
  fields open the ordinary Windows file dialog and quote what comes back;
  the icon a reference resolves to is drawn beside the field, so a wrong
  index shows before the entry does; the registry path the entry will land
  in stands under the form and follows what you type; and a folded *Help*
  carries the placeholder table and three working command lines. Always in
  `HKCU`, so without administrator rights and without risk to other
  accounts.
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
- **Services: two hundred tools from one address.** If a web application
  describes itself through OpenAPI, the address of its documentation page
  is enough. The program looks for the machine-readable document behind it,
  reads out which endpoints accept a file, groups them the way the service
  itself groups them, and turns every checked one into a favourite. If a
  tool accepts settings, a form is generated for it, even when the service
  describes its options only as running text.
- **Drag and drop.** Dragging an `.exe` into the window opens the editor
  already filled in from it — name, command with the right placeholder, the
  program's own icon; which category it is dropped on decides which one the
  form starts in. Nothing is written until the button in the form is
  pressed. In the editor, the fields for command and icon also accept a
  dropped file.
- **Back up and restore.** Every action creates a backup beforehand, a group
  action exactly one backup for the whole group. A **Back up** button in the
  top bar makes one on demand and changes nothing: the selected rows, or
  everything currently listed when nothing is selected. The *Backups* tab
  shows the history and plays it back, and it has a **Back Up Everything**
  button that takes along every location this tool touches at all (on this
  machine, 1.2 MB in under a second).
- **Submenus** are shown with their children, indented under the entry they
  hang from.
- **Look at an entry:** double-click a row, or right-click and choose *Look
  at this entry*, opens the form with everything that is actually in the
  registry.
- **Say when a new version is out.** One request to GitHub as the window
  opens, and a dot on the logo button if there is something newer. Fetching
  it takes a second click, and only happens when the release signature and
  the published checksum both check out; the request can be switched off in
  the About window.
- German and English, light and dark, or "Follow system", both without a
  restart; the title bar follows along.

## What It Deliberately Cannot Do

- **Change the text of a COM handler.** That text is generated at runtime in
  `IContextMenu::QueryContextMenu` and appears nowhere in the registry.
  What is shown is the key name, the CLSID's plain-text name, and the DLL
  behind it.
- **Rebuild the new Windows 11 main menu.** The tool works on the classic
  menu ("Weitere Optionen anzeigen", i.e. "Show more options"), which
  Windows 11 continues to run in full. What it can do is switch Explorer
  over to that classic menu altogether — see *Getting Started*.
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

The search field searches the display name, the command, and the registry
path — for a COM handler its CLSID and DLL as well, and a submenu's child
matches for the entry it hangs under. It applies to the three tabs that show
scanned entries (Categories, File Types, Programs), even when nothing is
selected on the left yet. The *Services* tab brings a search of its own,
above the tool list; Favourites and Backups are short enough to be shown
whole.

![The search field narrowing 927 entries down to a single Git Bash entry](docs/images/03-search_en.web.png)

*Typing `git` leaves one of 927 entries standing, and the right-hand side
says where it lives: `Directory\Background\shell\git_shell`, under `HKCU`,
with `%V` rather than `%1` because it hangs on a folder background.*

![The File Types tab resolving .png into 27 entries](docs/images/05-file-types_en.web.png)

*`.png` resolved: 27 entries, collected from the extension itself, its
ProgID and `image` as a perceived type — no single registry key holds this
list. The entries that apply to every file are left out until "Include
entries for all files" above the tree asks for them. The field below takes
any further extension; "All installed" swaps the curated 98 types for every
type registered on the machine.*

### In the List

The arrow keys move the selection, Home and End jump to the start and the
end, holding Shift grows the selection, and Ctrl+A selects everything.
Clicking a column header sorts by it, a second click reverses the
direction, and a third puts the table back into the order the rows were
collected in — which is worth having: in *File Types* that order puts the
entries belonging to the chosen extension in front of the ones that apply
to every file. *Flags* and the icon column do not sort; a row of symbols has
no order worth the click. The **Appears On** column says in words where an
entry shows up: "All Files" instead of `*`, ".zip" instead of a path with
`SystemFileAssociations` in the middle; the real registry path sits in the
tooltip. A **right-click** offers, everywhere, exactly the actions that
would change something about the item clicked; with a multi-selection, the
ones that only make sense for a single entry drop out, and in the empty
area below the table, *New*. The trees on the left answer a right-click
too, each with the target it stands for: a category row creates in that
category, a row in the file type tree creates for that extension alone —
the shortest way to "this entry, but only for `.png`".

![The details panel for the 7-Zip COM handler, with CLSID, DLL and a read-only notice](docs/images/02-entry-detail_en.web.png)

*One COM handler, opened: registry path, CLSID, the DLL behind it, and three
short reasons why nothing here can be edited — the key belongs to `HKLM`,
the text is produced at runtime, and the entry is read-only for this
account.*

**Restart Explorer** sits in the top bar. Windows reads the context menu
keys when Explorer starts; an entry that absolutely refuses to show up
needs this.

**Switch the Windows 11 menu off.** On Windows 11 the top bar carries one
more control: *Menu: Windows 11 | classic*. "Classic" puts back the full
Windows 10 menu, with every entry visible at once instead of half of them
behind "Show more options". It is one key in your own account — no
administrator rights, nobody else affected — and it takes hold when Explorer
next starts, which the program offers to do straight away. On Windows 10 the
control is not there, because there would be nothing to switch.

### A Typical Round

1. **Programs** tab, click the program that is causing trouble.
2. Check on the right what depends on it: registry path, raw value,
   command or CLSID and DLL, where it appears, and the children of a
   submenu. Every field can be selected and copied. A folder button beside
   the name opens Explorer with the program itself picked out, and each
   symbol the table had room for — the lock, the arrow, the Shift sign — is
   spelled out in words further down.
3. **Hide** instead of delete. That is reversible and is almost always
   enough.
4. If Windows asks for administrator rights: those are the entries under
   `HKLM`, the ones for all accounts. Anyone who declines keeps the changes
   to their own entries; the others stay as they were.

![The Programs tab, grouped by program, with two missing programs marked in red](docs/images/06-programs_en.web.png)

*Grouped by program instead of by key: one editor holds 49 entries,
LibreOffice Draw 44. At the top in red sit two programs that are no longer
installed and whose 33 entries are still in the menu.*

---

## Keeping It Up To Date

Because there is no installer, there is also nothing that would notice a
copy has gone stale. So the program asks: one GET to
`api.github.com/repos/corgan2222/context-manager/releases/latest` while the
window opens, without an account and without a token. Nothing else goes out
unless somebody presses the button below, and then it is three more GETs: the
checksums, their signature, and the executable.

What comes of it stays quiet. When there is nothing newer, nothing appears;
when the request fails, nothing appears either and the reason goes to the
log — a program that opens a window to announce that it could not reach
GitHub is a program people learn to switch off. Only a newer version shows
itself, and only as a dot in the corner of the logo button in the top bar,
whose tooltip names the version. The About window behind that button carries
the version number, the release notes, and a **Fetch and restart** button.

That second click is the first moment anything is downloaded, and what it
does happens in this order:

1. `checksums.txt` and `checksums.txt.sig` are fetched.
2. The signature has to verify against the public key compiled into the
   running `.exe` (`ctxmenu/release-signing.pub.pem`; RSA PKCS#1 v1.5 over
   SHA-256, with the arithmetic left to Windows' own CNG rather than
   hand-written). Nothing below this line runs if it does not.
3. That file has to name the version being offered. The signature covers the
   digests and nothing else — not the tag, not the release it hangs on — so
   without this step a signed list from an older release could be attached to
   a release tagged `v99.0.0` and would check out, quietly putting a fixed
   hole back. What closes it is already in the list: the archive beside the
   `.exe` carries the version in its name, and a line for
   `ctxmenu_<offered version>_windows_amd64.zip` can only be in a list that
   was signed for exactly that version.
4. The digest for `ctxmenu.exe` is read out of the file that has now been
   proved to be both the author's and this version's.
5. The executable is fetched, and accepted only when its SHA-256 is that
   digest.

TLS already says the bytes came from GitHub. The signature says they came
from whoever holds the private key, and that is the half that still holds
when the first one does not: the private key lives in a repository secret
that one step of the release workflow uses, not in the account it protects,
so somebody who can publish a release cannot sign one. A release without
`checksums.txt.sig` is therefore never offered for installing — which is
every release before 1.4.0, and is meant that way: a set of assets that is
allowed to arrive short is one somebody gets to shorten.

Such a release is not folded into "you are up to date" either, and neither is
one whose assets are still being uploaded. Both are named as announced but not
finished publishing yet, because from the outside they are the same thing: a
version that exists and that this program will not install. For the few
minutes after every publish that sentence is literally true, and the button
that would fetch it is simply not there.

The swap itself works around the one thing Windows refuses: it will not
overwrite a running executable, but it will rename one. So the new bytes are
written beside the old file as `ctxmenu.exe.new`, the running file is renamed
to `ctxmenu.exe.old`, and `ctxmenu.exe.new` takes the original name. The new
copy is started with the same command line this one had, this window closes,
and `ctxmenu.exe.old` is deleted on the next start. If the `.exe` sits in a
folder this account may not write to — `C:\Program Files`, usually — the
message says exactly that instead of reporting a broken download.

*Look for new versions on start* in the About window is on by default and
stops the request entirely when it is unticked; **Look now**, at the foot of
the same block, works either way, because pressing it is the decision the
setting otherwise makes.
There is no background service, no scheduled task, nothing measured or
reported back, and nothing downloaded or installed without that second click.

One limit, because the two are easy to confuse: the `.exe` is still not
Authenticode-signed, and SmartScreen still warns about it when it is
downloaded in a browser. Authenticode is what Windows checks before running a
downloaded file; the release signature is what *this program* checks before
replacing itself. Neither stands in for the other.

---

## Changing an Entry

Five levels, from gentle to hard:

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

**The action bar** above the table is built around four switches rather than
a row of verbs: *In the menu* (visible <-> hidden), *Shift key* (always <->
only with ⇧), *Machine-wide* (free <-> blocked), and *Position*. Highlighted
is where the selection currently stands; clicking the other side moves it
there. "Which button do I press?" becomes "where should it go?". With a
mixed selection, nothing lights up, and the tooltip gives the counts.
Whatever is not currently possible is greyed out and says why in the
tooltip: for instance, that none of the selected entries is a COM handler,
so there is nothing to block. Beside the switches sit the two selection
buttons, and at the far end, behind a separator and in red, **Delete** — the
only control in the bar that keeps its word instead of shrinking to a
symbol. Flush right, the bar says whether this run has administrator rights.
On the Favourites, Services and Backups tabs it is not drawn at all.

*New* in the top bar, or a right-click in the empty area, opens the same
form the other way round: not changing an entry but writing one.

![The form for a new entry, with category, command, icon, position and Shift visibility](docs/images/04-new-entry_en.web.png)

*The form names the key it is about to write before it writes it, offers
"Submenu" instead of "Single entry" for a whole list of children, and lists
underneath what this tool has already created — so that nothing is left
behind that nobody remembers making.*

---

## Favourites and Web Tools

The **Favourites** tab is a list that stays, in the order you put it in:
each row carries *Add to menu*, two arrows that move it up or down, *Edit*
and *Remove*, and that order is saved. The keyboard works too — arrows, Home
and End move the cursor, Enter places the favourite, Delete takes it out.
Whatever is in it once can be placed at another spot in the context menu at
any time, without setting it up again. "Add to menu" only asks for the
where: one of the base categories, a file extension (`.png`), or an entire
kind of file (`image` covers every image format Windows knows).

![The Favourites tab with eight web tools, each with an Add to menu button](docs/images/07-favourites_en.web.png)

*Eight web tools that stay. Each row keeps its mode — here "Upload" — and
its endpoint; "Add to menu" is the only step that ever has to be repeated,
and only to say where.*

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
lines for a key can be attached. A multipart request can carry plain form
fields beside the file, which is where a tool's settings travel: one field
holding the JSON block the service asked for, or one field per option when
the service names them separately. What comes back is, depending on the
setting, saved next to the original file (`bild.png` → `bild.min.png`; the
original is **never** overwritten), opened in the browser, or just
reported. The result address may be given in the `Location` header of a
successful answer or in a JSON field such as `output.url`.

**Redirects are not followed.** A `3xx` ends the request and says which
address it pointed at instead. The question before the upload named one
host; a service that answers by pointing somewhere else is asking for a
decision that was never taken. If that other address is the right one, it
belongs in the endpoint.

**A queued job is waited out.** A busy service answers with a receipt
instead of a file — a `202`, or a `200` carrying `"async": true` — and which
of the two arrives depends on how busy it is, not on the endpoint, so it
cannot be settled when the favourite is made. The program reads the job
number out of the receipt, asks the service's own progress path about it
every one and a half seconds for at most two minutes, and then saves the
finished file as if it had come back straight away. A frame that reports the
job failed ends the wait at once rather than running out the clock. This
needs the description to name a progress path, and the favourite to say
where the answer names the finished file.

**Open address**: builds the address from placeholders and opens it without
transmitting anything. `{name}`, `{stem}`, `{ext}`, `{path}`, `{dir}`, and
`{fileurl}`, all correctly encoded. For search, wiki, ticket forms.

**You are asked before the first upload.** Once per tool, stating the
destination and the file size; the answer is remembered — and it can be
taken back: the favourite's form says in a line that sending is confirmed
for this tool, and the button beside that line clears it, so the next click
asks again. Tools created from a service are the exception, because the
service was set up with its address and its key in one deliberate step:
they count as agreed to from the start and send on the first click. The
program refuses unencrypted `http://` unless it has been explicitly allowed for
this favourite: sending a file across the network in the clear is meant to
be a decision, not a default. WinHTTP handles the transfer, which is to say
Windows' own client: with the system certificate store and the proxy
settings that already apply anyway.

**Six files, one question and one message.** Windows reads a context menu
command ending in `"%1"` as "once per file", so six selected files start six
copies of this program, none of which knows about the others. They now agree
among themselves. **One** of them asks the question before the first upload,
the other five wait for that answer and act on it — a no included, in which
case nothing is sent by any of them. At the end they share **one**
notification instead of six: headed with the name of the tool, listing the
file names one under the other, updated as each file finishes rather than
popping up again. A single file still reads exactly as it did, with the
whole sentence and no counter. A file that fails keeps a message of its own,
because the reason is worth more than the tidiness. And if the six cannot
reach each other, every one of them asks and reports alone: six messages are
a nuisance, a file that was never sent is a fault.

---

## Services: Two Hundred Tools, One Address

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
  with 232 tools, this yields *Image, Video, PDF, Audio, Files* — plus two
  strays of one tool each — instead of a single drawer called "Tools" with
  225 entries in it, and it wins by a factor of 26.
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
- **Queued answers are not a dead end.** Endpoints that answer with a job
  number first are listed like any other and marked "works in the
  background": a favourite made of one follows the job over the service's
  progress path and saves the finished file. The declaration alone decides
  nothing — on the test service, the same endpoint answered directly and
  with a job number in turns — so the runtime looks at the real answer.

You check items individually or by category, and create them in one batch.

![The Services tab listing the tools read out of one OpenAPI description](docs/images/08-services_en.web.png)

*One address, read out: the tools grouped the way the service groups
itself — "Image" alone holds 81. "Settings" opens the form built from the
tool's own options, the arrow opens its page in the service's documentation.
Tools that answer with a job number first carry a "works in the background"
badge and can be created like any other.*

What a service says about itself then lives in every favourite — the
address, the key, where the answer names the finished file — and every tool
carries a link to its place in the service's documentation;
**the address and the key stay local** in
`%LOCALAPPDATA%\ctxmenu\services.json` and go nowhere. Because each
favourite holds its own copy, changing the service afterwards does not reach
the tools already made from it: tick the same ones again and create them a
second time. They are replaced rather than duplicated, which is also how a
service that has grown a new tool is caught up with.

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
ctxmenu show "<key>" --yes               undo that
ctxmenu shift-only "<key>" --yes         only on Shift+right-click
ctxmenu always-show "<key>" --yes        undo that
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
ctxmenu favourite place <id> --ext .png  also --category or --perceived
ctxmenu favourite remove <id>            take one out of the toolbox
ctxmenu favourite run <id> <file>        run it like a click
ctxmenu --tab services                   open the window on a specific tab
ctxmenu --ext .png                       file types tab, that extension selected
ctxmenu --search 7-zip                   the window, with the search filled in
ctxmenu --service snapotter              services tab, that service selected and loaded
ctxmenu --new directory                  the editor for a new entry, filled with an example
ctxmenu --lang en scan                   this run in English, saved setting untouched
ctxmenu --version                        which version this is
ctxmenu --help                           the list of commands and switches
```

`<key>` is the full path below a Classes root, the way `reg.exe` writes
it: `HKCU\SOFTWARE\Classes\Directory\shell\MeinEintrag`. Anything above
that root, and any path ending in a collecting key such as `shell`, is
refused. The plain `scan` table does not print it — `ctxmenu scan --json`
carries it as `registry_path`, and the window shows it in the detail pane.

**Leave `--yes` off and nothing is written.** The command names the key it
would touch and, for the four flag verbs, whether that step would need
administrator rights. It is the cheapest way to check a key typed by hand.

One trap, and it is Windows' own: the released `.exe` is a windowed program,
so the shell does not wait for it, and `ctxmenu scan --json > scan.json`
leaves the file empty — measured, with no error to show for it. Read the
output in the console, or capture it with `Start-Process ctxmenu
-ArgumentList 'scan','--json' -Wait -RedirectStandardOutput scan.json`.

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

![The Backups tab, one line per backup with a timestamp and a key count](docs/images/09-backups_en.web.png)

*One line per action, with the number of keys behind it in brackets. The
lines reading 26 are full backups; the rest come from single changes and
from test runs on the developing machine.*

```
%LOCALAPPDATA%\ctxmenu\backups\<timestamp>_<action>\
    manifest.json      what was backed up, when, and what was missing
    01_….reg           one file per key, written by reg.exe
%LOCALAPPDATA%\ctxmenu\entries.json     entries you created yourself
%LOCALAPPDATA%\ctxmenu\favourites.json  the toolbox
%LOCALAPPDATA%\ctxmenu\services.json    registered services, including their keys
%LOCALAPPDATA%\ctxmenu\settings.json    language, appearance, update check
%LOCALAPPDATA%\ctxmenu\ctxmenu.log      every error shown and every crash
```

The log is linked from the About window — which does one more thing worth
knowing about. It offers to put *this* program into the folder background
and desktop background menus, says for each whether the entry is already
there, and takes it out again on the same button. It is the one entry nobody
can write by hand without first knowing where their own `.exe` lives, and
removing it is backed up like every other deletion.

Three more things are written outside that folder by a favourite being
clicked, and two more by an update installing itself: `ctxmenu.exe.new` and
`ctxmenu.exe.old`, both beside the running `.exe`, both gone again within
seconds. The first of the three is a Start menu shortcut:

```
%APPDATA%\Microsoft\Windows\Start Menu\Programs\ctxmenu.lnk
```

A shortcut to the running `.exe`, created the first time a favourite reports
its result. Before Windows will draw a desktop program's notification on the
screen, it wants a Start menu shortcut naming the same identifier the
notification is sent under; without one the message is filed in the Action
Center and nothing appears. What Windows learns this way it keeps, though:
measured with the shortcut deleted again, the banner still arrived. So delete
it and nothing breaks, and the next run writes it back anyway. It is also
rewritten whenever the `.exe` moves, so the entry never points at a file
that is no longer there.

The second is a scratch file per favourite and per day under
`%TEMP%\ctxmenu-batch\`: three lines of text saying when the run started,
how the one question was answered, and which files are done. It is what the
six processes of one click agree through, it is a few dozen bytes, and the
next run on another day sweeps it away. The third is a single registry
value, `HKCU\SOFTWARE\Classes\AppUserModelId\ctxmenu.ContextMenuManager\DisplayName`,
which is where Windows reads the name it writes above a notification.

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

## Speed

Measured on this machine, four screens at 3840x2160: **714 to 724 ms** from
process creation to the first visible list with 927 real entries, and 1113
to 1277 ms the very first time a freshly built `.exe` runs. Scrolling a
table of 2000 rows costs **16.7 ms per frame on average**, 18.5 ms in the
worst frame of 300 (`--synthetic 2000 --bench 300`).

![The table filled with 2000 generated entries](docs/images/10-many-entries_en.web.png)

*`--synthetic 2000` fills the table with generated rows so the list can be
judged without owning a machine that really has that many. The flags column
shows the four states side by side: hidden, Shift-only, blocked, and pinned
to the top or bottom.*

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

536 tests, `cargo clippy -- -D warnings` clean.

Deferred plans, the development status, the measured values, and the
places where Windows behaves differently than documented are kept by the
author in notes that are not part of this repository.

---

## Contributing, Security, Licence

- [Contributing](CONTRIBUTING.md)
- [AI policy](AI_POLICY.md)
- [Security policy](SECURITY.md)
- [Code of conduct](CODE_OF_CONDUCT.md)
- [Third-party notices](docs/THIRD-PARTY-NOTICES.md)
- [MIT licence](LICENSE)
