//! Creating one's own context menu entries.
//!
//! Written to `HKCU\SOFTWARE\Classes` and nowhere else: no elevation needed,
//! nothing system-wide broken if it goes wrong, and removable by the same user
//! who added it.
//!
//! Every entry is *also* recorded in `entries.json`. That is not a cache — it
//! is preparation for a planned Windows 11 `IExplorerCommand` handler, which
//! reads this file and builds its entries from it. Writing it now means the
//! DLL has to be built and signed exactly once, and the interface keeps
//! writing nothing but JSON.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use windows_registry::CURRENT_USER;

use super::paths::RegTarget;
use crate::model::{Category, Scope};

/// What the editor collects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewEntry {
    pub category: Category,
    /// Registry key name. Also the sort key, since Windows enumerates
    /// subkeys alphabetically and nothing else influences the base order.
    pub key_name: String,
    pub display_name: String,
    /// Empty when this entry is a submenu: a cascading parent runs nothing
    /// itself, its children do.
    pub command: String,
    pub icon: Option<String>,
    pub position: Option<String>,
    /// Only visible while Shift is held.
    pub extended: bool,
    /// Children of a cascading menu, or empty for an ordinary entry.
    ///
    /// `serde(default)` because every entry written before 2026-08-15 has no
    /// such field, and an `entries.json` from then still has to read.
    #[serde(default)]
    pub children: Vec<NewChild>,
}

/// One entry inside a submenu.
///
/// Its own type rather than a nested [`NewEntry`]: a child has no category (it
/// lives wherever its parent does), no `Position` (the submenu is positioned,
/// not its contents), and no children of its own. Windows does allow deeper
/// nesting — the scanner follows it — but this editor offers exactly one
/// level, and a type that cannot express more cannot write more by accident.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewChild {
    /// Derived from the position in the list, not typed: subkeys come back
    /// alphabetically, so the name *is* the order (see
    /// [`suggest_child_key_name`]).
    pub key_name: String,
    pub display_name: String,
    pub command: String,
    pub icon: Option<String>,
}

impl NewEntry {
    /// The form filled in from an entry that already exists.
    ///
    /// The way back from a scan result to the shape the editor works in, so a
    /// double click can show what an entry really consists of instead of
    /// sending the reader to `regedit`. Nothing here writes: what comes out is
    /// a description, and whether it may be written is decided by
    /// [`check`] and [`NewEntry::target`] exactly as for a fresh one.
    ///
    /// A COM handler has no command line at all — its text is produced at run
    /// time by `IContextMenu` — so the command stays empty rather than being
    /// invented.
    pub fn from_scanned(entry: &crate::model::ContextEntry) -> Self {
        let (command, children) = match &entry.kind {
            crate::model::EntryKind::Verb {
                command,
                sub_commands,
            } => (
                command.clone().unwrap_or_default(),
                sub_commands
                    .iter()
                    .map(|child| NewChild {
                        key_name: child.key_name.clone(),
                        display_name: child.display_name.clone(),
                        command: match &child.kind {
                            crate::model::EntryKind::Verb { command, .. } => {
                                command.clone().unwrap_or_default()
                            }
                            // Neither has a command line: a COM handler's text
                            // comes from IContextMenu, a packaged verb's from
                            // IExplorerCommand.
                            crate::model::EntryKind::ShellEx { .. }
                            | crate::model::EntryKind::PackagedVerb { .. } => String::new(),
                        },
                        icon: child.icon_ref.clone(),
                    })
                    .collect(),
            ),
            crate::model::EntryKind::ShellEx { .. }
            | crate::model::EntryKind::PackagedVerb { .. } => (String::new(), Vec::new()),
        };

        Self {
            category: entry.category.clone(),
            key_name: entry.key_name.clone(),
            // The resolved name, not the raw value: it is what the menu draws,
            // and the raw form is one line further down in the detail pane.
            display_name: entry.display_name.clone(),
            command,
            icon: entry.icon_ref.clone(),
            position: entry.position.clone(),
            extended: entry.extended,
            children,
        }
    }

    /// A cascading menu rather than a single entry.
    pub fn is_submenu(&self) -> bool {
        !self.children.is_empty()
    }

    /// Every command line this entry is going to write.
    ///
    /// A submenu's own `command` field is deliberately not among them: it does
    /// not reach the registry, so checking its contents would be checking
    /// something nobody will ever run.
    fn commands(&self) -> impl Iterator<Item = &str> {
        let own = (!self.is_submenu()).then_some(self.command.as_str());
        own.into_iter()
            .chain(self.children.iter().map(|c| c.command.as_str()))
    }
}

/// What is wrong, without saying it in any particular language.
///
/// This module cannot know which language the window is showing — it also
/// feeds the command line and the log, where the message wants to stay put.
/// So it reports the *cause*, and whoever displays it puts it into words.
#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    MissingKeyName,
    BackslashInKeyName,
    MissingDisplayName,
    MissingCommand,
    /// `%1` in a background category, where it stays empty. The single most
    /// common mistake in hand-written entries.
    PercentOneInBackground,
    /// An `&` becomes an accelerator in the menu.
    AmpersandInDisplayName,
    /// A `Position` value other than Top or Bottom.
    UnusualPosition(String),
    /// A submenu carries no command of its own; the value would be dropped.
    CommandBesideSubmenu,
    /// The category cannot hold an entry of one's own — a ProgID, the
    /// CommandStore, or a file type field that is still empty.
    CategoryNotCreatable,
    /// Everything else [`NewEntry::target`] refuses, `shell` as a key name
    /// being the one that is actually reachable from the form.
    UnusableKeyName,
    /// Numbers are 1-based: they name a row of the form, not an index.
    ChildMissingDisplayName(usize),
    ChildMissingCommand(usize),
    /// Two children would end up in the same registry key.
    DuplicateChildKeyName(String),
}

impl Fault {
    /// Both languages, marked, so the reader is shown the one
    /// they read. See [`crate::bilingual`].
    pub fn marked(&self) -> String {
        match self {
            Fault::MissingKeyName => "\x1eSchlüsselname fehlt\x1fkey name is missing\x1d".into(),
            Fault::BackslashInKeyName => {
                "\x1eSchlüsselname darf keinen Backslash enthalten\x1fno backslash in a key name\x1d".into()
            }
            Fault::MissingDisplayName => "\x1eAnzeigename fehlt\x1fdisplay name is missing\x1d".into(),
            Fault::MissingCommand => "\x1eBefehl fehlt\x1fcommand is missing\x1d".into(),
            Fault::PercentOneInBackground => {
                "\x1eIn einer Hintergrund-Kategorie bleibt %1 leer — hier gehört %V hin.\
                 \x1f%1 stays empty in a background category; %V belongs here.\x1d"
                    .into()
            }
            Fault::AmpersandInDisplayName => {
                "\x1eEin & erzeugt im Menü einen Zugriffsbuchstaben; für ein echtes \
                 Und-Zeichen && schreiben.\x1fAn & becomes an accelerator in the menu; \
                 write && for a literal ampersand.\x1d"
                    .into()
            }
            Fault::UnusualPosition(value) => format!(
                "\x1ePosition {value:?} ist ungewöhnlich; belegt sind Top und Bottom.\
                 \x1fPosition {value:?} is unusual; only Top and Bottom are verified.\x1d"
            ),
            Fault::CommandBesideSubmenu => {
                "\x1eEin Untermenü führt selbst nichts aus; der Befehl wird nicht geschrieben.\
                 \x1fa submenu runs nothing itself; the command will not be written.\x1d"
                    .into()
            }
            Fault::CategoryNotCreatable => "\x1eHier kann kein eigener Eintrag angelegt werden.\
                 \x1fno entry of one's own can be created here.\x1d"
                .into(),
            Fault::UnusableKeyName => "\x1eDieser Schlüsselname ist hier nicht erlaubt.\
                 \x1fthat key name is not allowed here.\x1d"
                .into(),
            Fault::ChildMissingDisplayName(n) => {
                format!("\x1eUntereintrag {n} hat keinen Anzeigenamen\x1fsubmenu entry {n} has no name\x1d")
            }
            Fault::ChildMissingCommand(n) => {
                format!("\x1eUntereintrag {n} hat keinen Befehl\x1fsubmenu entry {n} has no command\x1d")
            }
            Fault::DuplicateChildKeyName(name) => format!(
                "\x1eZwei Untereinträge heißen {name:?}\
                 \x1ftwo submenu entries are called {name:?}\x1d"
            ),
        }
    }
}

/// Something wrong enough to refuse, or worth saying out loud.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// Refuses the write.
    Error(Fault),
    /// Written anyway, but the user should know.
    Warning(Fault),
}

impl Problem {
    pub fn is_error(&self) -> bool {
        matches!(self, Problem::Error(_))
    }

    pub fn fault(&self) -> &Fault {
        match self {
            Problem::Error(f) | Problem::Warning(f) => f,
        }
    }

    /// Both languages, for anywhere without a language setting.
    pub fn message(&self) -> String {
        self.fault().marked()
    }
}

/// Categories where the clicked object is the *folder being looked at*, not a
/// selected item.
///
/// `%1` stays empty there and the entry silently does nothing — the single
/// most common mistake in hand-written entries.
pub fn is_background(category: &Category) -> bool {
    matches!(
        category,
        Category::DirectoryBackground | Category::DesktopBackground
    )
}

/// The entry a file dropped onto `category` would become.
///
/// The point of dropping a program onto a category is that the form comes up
/// already right, and "right" differs by category: the placeholder that carries
/// the clicked path is `%1` almost everywhere and `%V` in the two background
/// categories, where `%1` stays empty. Getting that wrong by hand is the
/// mistake `check` warns about; getting it right here means the warning never
/// has to appear.
///
/// Nothing is written — this fills in a form the user still has to accept.
pub fn from_dropped_file(path: &std::path::Path, category: Category) -> NewEntry {
    let display_name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    let placeholder = match is_background(&category) {
        true => "%V",
        false => "%1",
    };

    NewEntry {
        key_name: suggest_key_name(&display_name),
        // The program's own icon, by naming the file: Windows takes the first
        // icon resource when no index follows, which is the one every launcher
        // shows for it.
        icon: Some(path.display().to_string()),
        command: format!(r#""{}" "{placeholder}""#, path.display()),
        display_name,
        category,
        position: None,
        extended: false,
        children: Vec::new(),
    }
}

/// The name the example entry carries, in both languages.
///
/// Marked rather than picked here, because this module never learns which
/// language is on screen — see `bilingual`. The one place the example's
/// wording is written down.
pub const EXAMPLE_NAME: &str = "\x1eBeispiel: mit Editor öffnen\x1fExample: open with Notepad\x1d";

/// The program the example runs.
///
/// Notepad, spelled through `%SystemRoot%`: it is on every Windows, it is on
/// the drive Windows was installed to even when that is not `C:`, and a reader
/// recognises it as an example rather than as something this program set up
/// for them. It is also harmless if the form is accepted by mistake — the
/// worst it can do is open a text editor.
pub const EXAMPLE_PROGRAM: &str = r"%SystemRoot%\System32\notepad.exe";

/// The form `--new <category>` opens: filled in, and nothing written.
///
/// A form that opens empty is exactly the problem `--service` was added to
/// solve on the services tab — a picture of a dialog with nothing in it shows
/// the frame and none of the point. So the example is filled in here, in one
/// place, and the fields say out loud that they are an example.
///
/// `%V` in the two background categories and `%1` everywhere else, for the
/// same reason as [`from_dropped_file`]: `%1` is empty on a folder background,
/// and `check` would rightly warn about it.
///
/// Nothing is written. This builds a value; the user still has to press the
/// button in the form.
pub fn example_entry(category: Category, language: crate::settings::Language) -> NewEntry {
    let placeholder = match is_background(&category) {
        true => "%V",
        false => "%1",
    };
    let display_name = crate::bilingual::pick(EXAMPLE_NAME, language).into_owned();

    NewEntry {
        key_name: suggest_key_name(&display_name),
        display_name,
        command: format!(r#""{EXAMPLE_PROGRAM}" "{placeholder}""#),
        icon: Some(EXAMPLE_PROGRAM.to_string()),
        category,
        position: None,
        extended: false,
        children: Vec::new(),
    }
}

/// The key name this program uses for its own context menu entry.
///
/// Fixed rather than derived from the display name: the entry has to be found
/// again to say whether it is there and to take it away, and the display name
/// changes with the interface language.
pub const SELF_KEY: &str = "ctxmenu_manage";

/// The entry that opens this program from a right-click.
///
/// Two places, and only two: the background of a folder and the background of
/// the desktop. Those are where somebody stands when they think "what is even
/// in this menu" — on a file the entry would only be in the way of the menu it
/// is meant to tidy up.
///
/// No `%1` or `%V`: the program takes no file, it opens its own window.
pub fn self_entry(category: Category, display_name: &str) -> Result<NewEntry> {
    let exe = std::env::current_exe().context("\x1eeigener Pfad\x1fown path\x1d")?;
    Ok(NewEntry {
        category,
        key_name: SELF_KEY.to_string(),
        display_name: display_name.to_string(),
        command: format!("\"{}\"", exe.display()),
        icon: Some(exe.display().to_string()),
        position: None,
        extended: false,
        children: Vec::new(),
    })
}

/// Where this program offers to put itself.
pub const SELF_CATEGORIES: [Category; 2] =
    [Category::DirectoryBackground, Category::DesktopBackground];

/// Whether the entry is in place, per category.
pub fn self_entry_present() -> Vec<(Category, bool)> {
    SELF_CATEGORIES
        .into_iter()
        .map(|category| {
            let there = self_entry(category.clone(), "x")
                .and_then(|entry| entry.target())
                .map(|target| super::write::exists(&target))
                .unwrap_or(false);
            (category, there)
        })
        .collect()
}

/// Takes this program's own entry back out of the menu.
///
/// Through the ordinary road — back up, delete, forget — and not through
/// `write::remove_own_new_key`, which exists for undoing a half-written create
/// and skips the backup on purpose. "Backed up before every change" has no
/// exception for the entry this program made for itself.
pub fn remove_self(target: &super::paths::RegTarget) -> Result<()> {
    let token = super::backup::export("ctxmenu-Eintrag", &[target.full_path()])?;
    super::write::delete_tree(target, &token)?;
    // Best effort, in the same words as the plan path and `ctxmenu delete`:
    // the key is gone, and failing to tidy the bookkeeping afterwards is not a
    // failed removal. It became worth saying once `recorded` started reporting
    // a damaged file instead of reading it as an empty list.
    let _ = forget_target(target);
    Ok(())
}

/// Checks an entry before anything is written.
pub fn check(entry: &NewEntry) -> Vec<Problem> {
    let mut problems = Vec::new();

    let name = entry.key_name.trim();
    if name.is_empty() {
        problems.push(Problem::Error(Fault::MissingKeyName));
    } else if name.contains(['\\', '/']) {
        problems.push(Problem::Error(Fault::BackslashInKeyName));
    }

    if entry.display_name.trim().is_empty() {
        problems.push(Problem::Error(Fault::MissingDisplayName));
    }

    if entry.is_submenu() {
        // Not an error: the entry is writable, the command is simply not part
        // of it. Said out loud rather than swallowed, because a command line
        // that vanishes without a word is indistinguishable from one that was
        // written and does not work.
        if !entry.command.trim().is_empty() {
            problems.push(Problem::Warning(Fault::CommandBesideSubmenu));
        }
        problems.extend(child_problems(entry));
    } else if entry.command.trim().is_empty() {
        problems.push(Problem::Error(Fault::MissingCommand));
    }

    // The one that actually catches people out. Reported once however many
    // commands carry it: the cause and the cure are the same for all of them,
    // and ten identical lines below a form are read as one anyway.
    if is_background(&entry.category) && entry.commands().any(uses_percent_one_wrongly) {
        problems.push(Problem::Warning(Fault::PercentOneInBackground));
    }

    if entry.display_name.contains('&')
        || entry.children.iter().any(|c| c.display_name.contains('&'))
    {
        problems.push(Problem::Warning(Fault::AmpersandInDisplayName));
    }

    // Whatever `target()` will refuse, said in the same voice as everything
    // above. Until 2026-08-15 the editor had to print that error verbatim
    // instead, and `paths` writes in both languages at once because it also
    // serves the console — so the form showed a sentence half of which nobody
    // had asked for, usually right above the plain reason for it.
    if !category_is_creatable(&entry.category) {
        problems.push(Problem::Error(Fault::CategoryNotCreatable));
    } else if entry.target().is_err() && !problems.iter().any(Problem::is_error) {
        problems.push(Problem::Error(Fault::UnusableKeyName));
    }

    if let Some(position) = &entry.position
        && !matches!(position.as_str(), "Top" | "Bottom")
    {
        problems.push(Problem::Warning(Fault::UnusualPosition(position.clone())));
    }

    problems
}

/// `%1` where the clicked object never provides one.
fn uses_percent_one_wrongly(command: &str) -> bool {
    command.contains("%1") && !command.contains("%V")
}

/// What is wrong inside a submenu.
///
/// The children are what a cascading entry consists of, so a missing name or
/// command in one of them is as much an error as it is on a single entry —
/// only it has to say *which* one, hence the 1-based numbers.
fn child_problems(entry: &NewEntry) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for (index, child) in entry.children.iter().enumerate() {
        let number = index + 1;

        if child.display_name.trim().is_empty() {
            problems.push(Problem::Error(Fault::ChildMissingDisplayName(number)));
        }
        if child.command.trim().is_empty() {
            problems.push(Problem::Error(Fault::ChildMissingCommand(number)));
        }

        let key = child.key_name.trim();
        if key.is_empty() {
            problems.push(Problem::Error(Fault::MissingKeyName));
        } else if key.contains(['\\', '/']) {
            problems.push(Problem::Error(Fault::BackslashInKeyName));
        }

        // Registry key names are case-insensitive, so two children differing
        // only in capitalisation are one key: the second would overwrite the
        // first and the menu would be one entry short, with nothing to say why.
        let folded = key.to_lowercase();
        if !folded.is_empty() {
            if seen.contains(&folded) {
                problems.push(Problem::Error(Fault::DuplicateChildKeyName(key.into())));
            } else {
                seen.push(folded);
            }
        }
    }

    problems
}

/// Can an entry be created for this category at all?
///
/// Answered without a key name, so the editor can grey out a choice before
/// anything else is filled in. `target()` says the same thing but needs the
/// whole entry, and "why is this button dead" should not depend on a field
/// three rows further down.
pub fn category_is_creatable(category: &Category) -> bool {
    category_relative(category).is_ok()
}

impl NewEntry {
    /// Where this entry will live.
    pub fn target(&self) -> Result<RegTarget> {
        let relative = category_relative(&self.category)?;
        // Always the user's own hive. Through the checking constructor, so a
        // key name of "shell" — or of nothing at all — is refused here rather
        // than creating a key that later swallows a delete of its siblings.
        Ok(RegTarget::below_classes(
            Scope::User,
            &format!(r"{relative}\{}", self.key_name.trim()),
        )?)
    }
}

/// The `…\shell` path of a category.
///
/// The file type cases are how an entry is limited to one kind of file. Not
/// `AppliesTo`: that value takes a structured query, and of the 27 instances
/// on this machine not one uses the textbook `System.ItemType:.txt` shape —
/// they filter by BitLocker state and storage provider. Placing the
/// key under `SystemFileAssociations` is the documented mechanism, is what
/// every image tool on this machine actually does, and has the side benefit
/// that the entry then appears in this program's own file type view.
fn category_relative(category: &Category) -> Result<String> {
    Ok(match category {
        Category::AllFiles => r"*\shell".into(),
        Category::AllFilesystemObjects => r"AllFilesystemObjects\shell".into(),
        Category::Directory => r"Directory\shell".into(),
        Category::DirectoryBackground => r"Directory\Background\shell".into(),
        Category::Folder => r"Folder\shell".into(),
        Category::DesktopBackground => r"DesktopBackground\Shell".into(),
        Category::Drive => r"Drive\shell".into(),

        // Scannable and hideable, but not creatable yet: every shipped
        // version reads `entries.json` strictly, and a category it does not
        // know makes that reader set the whole file aside as damaged — the
        // Win11 handler's input included. These four open up once a tolerant
        // reader has been shipped for a release.
        Category::Unknown
        | Category::DirectoryAudio
        | Category::DirectoryImage
        | Category::DirectoryVideo => bail!(
            "\x1eFür diese Kategorie können keine Einträge angelegt werden\x1fcannot create entries for\x1d {category:?}"
        ),

        // One extension: `.png` only.
        Category::ExtAssoc(ext) => {
            let ext = check_ext(ext)?;
            format!(r"SystemFileAssociations\{ext}\shell")
        }
        // A whole class of file: `image` covers every extension Windows
        // considers a picture, which is usually what somebody adding an image
        // tool means.
        Category::PerceivedType(kind) => {
            let kind = kind.trim().to_lowercase();
            if kind.is_empty() || kind.contains('\\') || kind.contains('/') {
                bail!("\x1eUngültiger wahrgenommener Typ\x1finvalid perceived type\x1d: {kind:?}");
            }
            format!(r"SystemFileAssociations\{kind}\shell")
        }

        other => bail!(
            "\x1eFür diese Kategorie können keine Einträge angelegt werden\x1fcannot create entries for\x1d {other:?}"
        ),
    })
}

/// An extension in the form the registry keeps it: leading dot, lowercase.
fn check_ext(ext: &str) -> Result<String> {
    let trimmed = ext.trim().to_lowercase();
    let with_dot = if trimmed.starts_with('.') {
        trimmed
    } else {
        format!(".{trimmed}")
    };

    if with_dot.len() < 2 || with_dot[1..].contains('.') || with_dot.contains(['\\', '/', ' ']) {
        bail!("\x1eKeine gültige Dateiendung\x1fnot a valid extension\x1d: {ext:?}");
    }
    Ok(with_dot)
}

/// What [`create`] left behind.
///
/// Its own type rather than a bare [`RegTarget`], because writing the entry
/// and recording it are two steps and only the first decides whether the entry
/// exists. The registry tree is complete before `entries.json` is opened at
/// all, so a failure there is not a failed create: the item is in the menu and
/// it works. It costs the planned Windows 11 handler its knowledge of this
/// entry, which is worth a sentence beside the success — and is not worth
/// throwing the success away for, which is what returning `Err` used to do.
/// The user then saw a red box, the list was not refreshed, and the second
/// attempt failed with "key already exists".
#[derive(Debug, Clone)]
pub struct Created {
    /// Where the key landed.
    pub target: RegTarget,
    /// What went wrong beside the entry, marked bilingual and ready to show;
    /// `None` when nothing did.
    pub note: Option<String>,
}

/// Writes the entry into HKCU and records it in `entries.json`.
///
/// Refuses on any [`Problem::Error`]; warnings are the caller's to show and
/// the user's to overrule.
pub fn create(entry: &NewEntry) -> Result<Created> {
    // Where the record lives is settled before the registry is touched. The
    // alternative is a written entry and no way to say where its record should
    // have gone, and on Windows a missing `%LOCALAPPDATA%` does not happen.
    create_in(&entries_path()?, entry)
}

/// The same, recording into a named file, so that "the entry is written even
/// when the record cannot be" can be tested without a full disk.
fn create_in(file: &Path, entry: &NewEntry) -> Result<Created> {
    let problems = check(entry);
    if let Some(error) = problems.iter().find(|p| p.is_error()) {
        bail!("{}", error.message());
    }

    let target = entry.target()?;
    if super::write::exists(&target) {
        bail!(
            "\x1eSchlüssel existiert bereits\x1fkey already exists\x1d: {}",
            target.full_path()
        );
    }

    // All or nothing: a submenu whose children failed halfway through is a
    // menu item that opens onto an empty box, and nobody asked for that. The
    // `exists` check above refused an existing key, so everything below the
    // target was written by this call and taking it back destroys nothing.
    if let Err(error) = write_tree(entry, &target) {
        let _ = super::write::remove_own_new_key(&target);
        return Err(error);
    }

    // Best effort, and now the code says so too: a failure here costs the
    // Windows 11 handler its knowledge of this entry, but the entry itself is
    // already in place and working. The note travels with the success instead
    // of replacing it.
    let note = match record_in(file, entry) {
        Ok(note) => note,
        Err(error) => Some(format!(
            "\x1eDer Eintrag steht, aber entries.json ließ sich nicht schreiben\
             \x1fthe entry is in place, but entries.json could not be written\x1d: {error:#}"
        )),
    };

    Ok(Created { target, note })
}

/// Writes the key, its values and whatever hangs below it.
fn write_tree(entry: &NewEntry, target: &RegTarget) -> Result<()> {
    let key = CURRENT_USER.create(target.key_path()).with_context(|| {
        format!(
            "\x1eAnlegen fehlgeschlagen\x1fcould not create\x1d {}",
            target.full_path()
        )
    })?;

    if entry.is_submenu() {
        // Measured across every `SubCommands` key under both classes roots on
        // this machine: all 15 submenu parents name themselves in `MUIVerb`,
        // not one of them has a default value, and not one has a `command`
        // subkey. A single entry keeps the default value — that is the form
        // the milestone 10 acceptance test photographed in the VM.
        key.set_string("MUIVerb", entry.display_name.trim())?;
        // The marker itself. Empty means "this is a submenu and my children
        // are in my own `shell` subkey"; a *non-empty* value would name verbs
        // in the CommandStore, which is a different mechanism.
        key.set_string("SubCommands", "")?;
    } else {
        key.set_string("", entry.display_name.trim())?;
    }

    if let Some(icon) = &entry.icon
        && !icon.trim().is_empty()
    {
        key.set_string("Icon", icon.trim())?;
    }
    if let Some(position) = &entry.position {
        key.set_string("Position", position)?;
    }
    if entry.extended {
        key.set_string("Extended", "")?;
    }

    if entry.is_submenu() {
        for child in &entry.children {
            write_child(target, child)?;
        }
    } else {
        CURRENT_USER
            .create(format!(r"{}\command", target.key_path()))
            .and_then(|command| command.set_string("", entry.command.trim()))
            .context("\x1ecommand-Unterschlüssel\x1fcommand subkey\x1d")?;
    }

    Ok(())
}

/// One child of a submenu: `<parent>\shell\<key>` and its `command`.
fn write_child(parent: &RegTarget, child: &NewChild) -> Result<()> {
    let path = format!(r"{}\shell\{}", parent.key_path(), child.key_name.trim());

    let key = CURRENT_USER.create(&path).with_context(|| {
        format!(
            "\x1eUntereintrag anlegen fehlgeschlagen\x1fcould not create submenu entry\x1d: {}",
            child.display_name.trim()
        )
    })?;

    // `MUIVerb` again, for the same measured reason: all 21 children of the
    // real submenus on this machine carry it, none carries a default value.
    key.set_string("MUIVerb", child.display_name.trim())?;
    if let Some(icon) = &child.icon
        && !icon.trim().is_empty()
    {
        key.set_string("Icon", icon.trim())?;
    }

    CURRENT_USER
        .create(format!(r"{path}\command"))
        .and_then(|command| command.set_string("", child.command.trim()))
        .with_context(|| format!("\x1ecommand-Unterschlüssel\x1fcommand subkey\x1d: {path}"))?;

    Ok(())
}

/// `%LOCALAPPDATA%\ctxmenu\entries.json`
pub fn entries_path() -> Result<PathBuf> {
    let base =
        dirs::data_local_dir().context("\x1ekein LOCALAPPDATA\x1fno local data directory\x1d")?;
    Ok(base.join("ctxmenu").join("entries.json"))
}

/// Everything this tool created, in the order it was created.
///
/// No file at all is an empty list: nothing has been created yet. A file that
/// does not parse is an **error**, and used to be an empty list as well — with
/// `record_in` then writing that empty list straight back plus the one new
/// entry, so a single damaged byte cost the record of everything this tool had
/// made. The registry keys survive that; the knowledge of which of them are
/// ours does not, and that is what the planned Windows 11 handler reads.
pub fn recorded() -> Result<Vec<NewEntry>> {
    recorded_in(&entries_path()?)
}

/// The same, from a named file, so the case above can be tested without
/// touching the record the user's own entries are in.
fn recorded_in(file: &Path) -> Result<Vec<NewEntry>> {
    match std::fs::read_to_string(file) {
        // Nothing but whitespace is what an interrupted write leaves behind
        // and says exactly as much as no file at all.
        Ok(raw) if raw.trim().is_empty() => Ok(Vec::new()),
        Ok(raw) => serde_json::from_str(&raw).with_context(|| format!("{file:?}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(anyhow::Error::from(error).context(format!("{file:?}"))),
    }
}

/// Notes the entry, replacing an earlier line for the same key in the same
/// category.
///
/// Hands back what the user should be told about the file itself, or `None`
/// when there is nothing to tell.
fn record_in(file: &Path, entry: &NewEntry) -> Result<Option<String>> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // A damaged file is carried out of the way rather than written over. What
    // stands in it is the only record of what this tool created, and a copy
    // the user can still repair by hand is worth more than a tidy directory.
    let (mut all, note) = match recorded_in(file) {
        Ok(all) => (all, None),
        Err(_) => {
            let aside = set_aside(file)?;
            let note = format!(
                "\x1eentries.json war beschädigt und liegt jetzt daneben als\
                 \x1fentries.json was damaged and has been put aside as\x1d: {}",
                aside.display()
            );
            (Vec::new(), Some(note))
        }
    };

    all.retain(|existing| {
        existing.key_name != entry.key_name || existing.category != entry.category
    });
    all.push(entry.clone());

    store(file, &all)?;
    Ok(note)
}

/// Moves a file that no longer parses beside itself, and says where to.
///
/// One rescue copy, replaced by the next one: a growing pile of
/// `entries.json.beschaedigt-3` in the user's directory would be a mess of its
/// own, and the copy worth having is the one that just failed.
fn set_aside(file: &Path) -> Result<PathBuf> {
    let aside = file.with_extension("json.beschaedigt");
    std::fs::rename(file, &aside).with_context(|| format!("{aside:?}"))?;
    Ok(aside)
}

/// Writes the whole record, through a temporary file.
///
/// The same road as [`crate::favourites::save`] and for the same reason: a
/// plain `fs::write` truncates the file first, and the release profile builds
/// with `panic = "abort"`, so an interruption in the middle of it leaves half
/// a JSON document behind — the very damage the paragraph above has to cope
/// with. Writing beside the file and renaming means a reader sees either the
/// old record or the new one.
fn store(file: &Path, all: &[NewEntry]) -> Result<()> {
    let temporary = file.with_extension("json.neu");
    std::fs::write(&temporary, serde_json::to_string_pretty(all)?)
        .with_context(|| format!("{temporary:?}"))?;
    std::fs::rename(&temporary, file).with_context(|| format!("{file:?}"))?;
    Ok(())
}

/// Forgets whatever was recorded for this registry key.
///
/// Called after a successful delete. Without it `entries.json` keeps naming an
/// entry the user has removed — and that file is the input for the planned
/// Windows 11 handler, so a stale line there would eventually put the deleted
/// item back in the menu.
pub fn forget_target(target: &RegTarget) -> Result<()> {
    forget_target_in(&entries_path()?, target)
}

fn forget_target_in(file: &Path, target: &RegTarget) -> Result<()> {
    let wanted = target.full_path().to_lowercase();
    let mut list = recorded_in(file)?;
    let before = list.len();

    list.retain(|entry| {
        entry
            .target()
            .map(|t| t.full_path().to_lowercase() != wanted)
            .unwrap_or(true)
    });

    if list.len() != before {
        store(file, &list)?;
    }
    Ok(())
}

/// Forgets an entry in `entries.json`. The registry key is removed elsewhere,
/// through the ordinary plan path with its backup.
pub fn forget(category: &Category, key_name: &str) -> Result<()> {
    forget_in(&entries_path()?, category, key_name)
}

fn forget_in(file: &Path, category: &Category, key_name: &str) -> Result<()> {
    let mut all = recorded_in(file)?;
    let before = all.len();
    all.retain(|entry| entry.key_name != key_name || &entry.category != category);

    if all.len() != before {
        store(file, &all)?;
    }
    Ok(())
}

/// A suggestion for the key name, derived from the display name.
///
/// Prefixed because the key name is the only lever on the base order, and a
/// tool's own entries should at least sort together.
pub fn suggest_key_name(display_name: &str) -> String {
    format!("ctxmenu_{}", sanitise(display_name))
}

/// The key name for the n-th child of a submenu, counted from zero.
///
/// Numbered, and not left to the user, because the key name *is* the order:
/// the registry hands subkeys back alphabetically no matter which order they
/// were written in, so a child moved up in the form has to be renamed to move
/// up in the menu. Both foreign tools on this machine that ship a submenu do
/// exactly this — MarkItDown numbers `01_save, 02_clip, 03_open`, the
/// Attributes menu letters `aShow, bReset, cReadOnlySet`.
///
/// Two digits, so ten children do not sort in front of two.
pub fn suggest_child_key_name(index: usize, display_name: &str) -> String {
    format!("{:02}_{}", index + 1, sanitise(display_name))
}

/// A display name reduced to something a registry key can be called.
fn sanitise(display_name: &str) -> String {
    let cleaned: String = display_name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "eintrag".into()
    } else {
        trimmed.chars().take(48).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_freely_chosen_file_type_is_checked_before_it_is_offered() {
        // The editor lets a file type be typed in now, so these are one click
        // and one keystroke away rather than unreachable.
        assert!(category_is_creatable(&Category::Directory));
        assert!(category_is_creatable(&Category::ExtAssoc(".png".into())));
        assert!(category_is_creatable(&Category::PerceivedType(
            "image".into()
        )));

        // Nothing typed yet: the commonest state of a fresh choice, and the
        // one that must not reach the registry as `SystemFileAssociations\shell`.
        assert!(!category_is_creatable(&Category::ExtAssoc(String::new())));
        assert!(!category_is_creatable(&Category::ExtAssoc("   ".into())));
        assert!(!category_is_creatable(&Category::ExtAssoc(".".into())));
        assert!(!category_is_creatable(&Category::ExtAssoc("a b".into())));
        assert!(!category_is_creatable(&Category::PerceivedType(
            String::new()
        )));

        // Still refused, and still on purpose: several extensions share one
        // ProgID, so an entry there would appear for all of them.
        assert!(!category_is_creatable(&Category::ProgId {
            prog_id: "pngfile".into(),
            from_ext: ".png".into(),
        }));
        assert!(!category_is_creatable(&Category::CommandStore));
    }

    fn entry(category: Category, command: &str) -> NewEntry {
        NewEntry {
            category,
            key_name: "ctxmenu_test".into(),
            display_name: "Test".into(),
            command: command.into(),
            icon: None,
            position: None,
            extended: false,
            children: Vec::new(),
        }
    }

    fn child(display_name: &str, command: &str) -> NewChild {
        NewChild {
            key_name: suggest_child_key_name(0, display_name),
            display_name: display_name.into(),
            command: command.into(),
            icon: None,
        }
    }

    #[test]
    fn a_dropped_program_arrives_with_the_placeholder_its_category_needs() {
        let path = std::path::Path::new(r"C:\Program Files\Tool\tool.exe");

        let on_files = from_dropped_file(path, Category::AllFiles);
        assert_eq!(on_files.display_name, "tool");
        assert_eq!(on_files.key_name, "ctxmenu_tool");
        assert_eq!(on_files.command, r#""C:\Program Files\Tool\tool.exe" "%1""#);
        assert_eq!(
            on_files.icon.as_deref(),
            Some(r"C:\Program Files\Tool\tool.exe"),
            "the program brings its own icon"
        );

        // The whole point: on a background category %1 would stay empty, and
        // the entry would appear and do nothing.
        for category in [Category::DirectoryBackground, Category::DesktopBackground] {
            let dropped = from_dropped_file(path, category.clone());
            assert!(
                dropped.command.contains("%V") && !dropped.command.contains("%1"),
                "{category:?} needs %V"
            );
            // And what comes out passes the check that would have caught it.
            assert!(
                !check(&dropped)
                    .iter()
                    .any(|p| matches!(p.fault(), Fault::PercentOneInBackground))
            );
        }
    }

    #[test]
    fn the_example_form_is_complete_in_both_languages() {
        use crate::settings::Language;

        for language in [Language::German, Language::English] {
            let example = example_entry(Category::Directory, language);

            // A picture of the form is the point, so nothing in it may be
            // empty and nothing may carry a complaint underneath it.
            assert!(!example.display_name.trim().is_empty());
            assert!(!example.key_name.trim().is_empty());
            assert!(example.command.contains("notepad.exe"), "{example:?}");
            assert!(
                check(&example).is_empty(),
                "{language:?}: {:?}",
                check(&example)
            );
            // The marked form belongs in the source, never in a field the
            // user reads.
            assert!(
                !example.display_name.contains(crate::bilingual::is_marker),
                "{}",
                example.display_name
            );
        }

        // Two languages, two names -- otherwise the English picture would
        // show a German example.
        assert_ne!(
            example_entry(Category::Directory, Language::German).display_name,
            example_entry(Category::Directory, Language::English).display_name
        );

        // Same placeholder rule as a dropped file, and for the same reason.
        for category in [Category::DirectoryBackground, Category::DesktopBackground] {
            let example = example_entry(category.clone(), Language::English);
            assert!(
                example.command.contains("%V") && !example.command.contains("%1"),
                "{category:?} needs %V"
            );
            assert!(check(&example).is_empty(), "{:?}", check(&example));
        }
    }

    #[test]
    fn percent_one_in_a_background_category_is_flagged() {
        // The classic mistake: the entry appears and silently does nothing.
        for category in [Category::DirectoryBackground, Category::DesktopBackground] {
            let problems = check(&entry(category.clone(), r#""C:\t.exe" "%1""#));
            assert!(
                problems
                    .iter()
                    .any(|p| matches!(p, Problem::Warning(Fault::PercentOneInBackground))),
                "{category:?} did not warn about %1"
            );
        }

        // %V is correct there and must not be nagged about.
        let problems = check(&entry(Category::DirectoryBackground, r#""C:\t.exe" "%V""#));
        assert!(problems.is_empty(), "got {problems:?}");

        // In a normal category %1 is exactly right.
        let problems = check(&entry(Category::Directory, r#""C:\t.exe" "%1""#));
        assert!(problems.is_empty(), "got {problems:?}");
    }

    #[test]
    fn missing_pieces_are_refused_rather_than_written() {
        let mut e = entry(Category::Directory, "");
        assert!(check(&e).iter().any(Problem::is_error));

        e.command = "x".into();
        e.display_name = "  ".into();
        assert!(check(&e).iter().any(Problem::is_error));

        e.display_name = "Test".into();
        e.key_name = r"a\b".into();
        assert!(check(&e).iter().any(Problem::is_error));
    }

    #[test]
    fn an_ampersand_is_a_warning_not_a_refusal() {
        let mut e = entry(Category::Directory, "x");
        e.display_name = "Öffnen & Prüfen".into();

        let problems = check(&e);
        assert!(!problems.iter().any(Problem::is_error));
        assert!(problems.iter().any(|p| matches!(p, Problem::Warning(_))));
    }

    #[test]
    fn entries_always_land_in_the_users_own_hive() {
        for category in Category::BASE {
            // The four newest base categories are deliberately not creatable
            // until a tolerant entries.json reader has shipped — see
            // category_relative.
            if !category_is_creatable(&category) {
                continue;
            }
            let mut e = entry(category, "x");
            e.key_name = "ctxmenu_x".into();
            let target = e.target().expect("creatable base categories create");
            assert_eq!(
                target.scope(),
                Scope::User,
                "an entry must never be written machine-wide"
            );
            assert!(target.relative().ends_with(r"\ctxmenu_x"));
        }
    }

    #[test]
    fn a_file_type_entry_lands_under_system_file_associations() {
        // The measured way to limit an entry to one kind of file: the key's
        // place in the tree, not a query in AppliesTo.
        let mut e = entry(Category::ExtAssoc(".PNG".into()), "x");
        e.key_name = "ctxmenu_x".into();
        assert_eq!(
            e.target().expect("creatable").relative(),
            r"SystemFileAssociations\.png\shell\ctxmenu_x",
            "the extension is normalised to the form the registry keeps"
        );

        // A whole perceived class, which is what "all pictures" means.
        let mut e = entry(Category::PerceivedType("image".into()), "x");
        e.key_name = "ctxmenu_x".into();
        assert_eq!(
            e.target().expect("creatable").relative(),
            r"SystemFileAssociations\image\shell\ctxmenu_x"
        );

        // A missing dot is a typo, not an error worth refusing over.
        let mut e = entry(Category::ExtAssoc("jpg".into()), "x");
        e.key_name = "ctxmenu_x".into();
        assert!(
            e.target()
                .expect("creatable")
                .relative()
                .starts_with(r"SystemFileAssociations\.jpg\")
        );
    }

    #[test]
    fn nonsense_extensions_are_refused_rather_than_written() {
        for bad in [".", "", "a b", r"a\b", "a.b"] {
            let mut e = entry(Category::ExtAssoc(bad.into()), "x");
            e.key_name = "ctxmenu_x".into();
            assert!(e.target().is_err(), "{bad:?} must not become a key path");
        }
    }

    #[test]
    fn the_progid_categories_stay_refused() {
        // Writing into a ProgID would change what happens for every extension
        // pointing at it, which is a different decision from "for PNG files".
        let mut e = entry(
            Category::ProgId {
                prog_id: "pngfile".into(),
                from_ext: ".png".into(),
            },
            "x",
        );
        e.key_name = "ctxmenu_x".into();
        assert!(e.target().is_err());
    }

    /// A directory of one's own for the tests that write a record.
    ///
    /// `%LOCALAPPDATA%\ctxmenu\entries.json` belongs to the user and holds
    /// what they created; a test that writes there is a test that can lose it.
    /// `Drop` rather than a line at the end of the body, for the same reason
    /// as [`Branch`]: a failing assertion unwinds.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ctxmenu-create-test-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a temporary directory");
            Self(dir)
        }

        /// The record file inside it, which need not exist yet.
        fn entries(&self) -> PathBuf {
            self.0.join("entries.json")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn deleting_a_key_forgets_what_was_recorded_for_it() {
        // The record and the key have to disappear together: entries.json is
        // the input for the Windows 11 handler, so a line that outlives its
        // key would put a deleted item back into the menu.
        let scratch = Scratch::new("forget");
        let file = scratch.entries();

        let mut e = entry(Category::Directory, r#""C:\t.exe" "%1""#);
        e.key_name = "ctxmenu_forget_selftest".into();
        let target = e.target().expect("creatable");

        let mut other = entry(Category::Drive, r#""C:\t.exe" "%1""#);
        other.key_name = "ctxmenu_bleibt".into();

        // Recorded without touching the registry: this is about the file.
        record_in(&file, &other).expect("records");
        record_in(&file, &e).expect("records");
        assert_eq!(recorded_in(&file).expect("readable").len(), 2);

        forget_target_in(&file, &target).expect("forgets");

        let after = recorded_in(&file).expect("readable");
        assert!(
            !after.iter().any(|f| f.key_name == e.key_name),
            "the deleted entry is still listed"
        );
        assert_eq!(
            after.len(),
            1,
            "forgetting must remove exactly the one entry"
        );

        // And by category as well, which is the road the editor takes.
        forget_in(&file, &other.category, &other.key_name).expect("forgets");
        assert!(recorded_in(&file).expect("readable").is_empty());
    }

    #[test]
    fn a_deleted_all_files_entry_is_forgotten_despite_the_star_in_its_path() {
        // The star is the one category path with a metacharacter in it, and
        // the CLI reaches `forget_target` through `parse` on a hand-typed
        // path rather than through the entry itself. Both stood accused of
        // losing this record once (Win11 VM, 2026-08-20 — the record turned
        // out to be re-created after the delete, not kept by it), and the
        // twin below is what an overly loose comparison would take with it.
        let scratch = Scratch::new("forget-star");
        let file = scratch.entries();

        let mut e = entry(Category::AllFiles, r#""C:\t.exe" "%1""#);
        e.key_name = "ctxmenu_snapotter__metadaten_entfernen".into();
        record_in(&file, &e).expect("records");

        // The same favourite, placed for one perceived type as well: same key
        // name, different category, different registry key. That twin was in
        // the VM's file too and has to survive.
        let mut twin = entry(
            Category::PerceivedType("image".into()),
            r#""C:\t.exe" "%1""#,
        );
        twin.key_name = e.key_name.clone();
        record_in(&file, &twin).expect("records");

        let mut other = entry(Category::Directory, r#""C:\t.exe" "%1""#);
        other.key_name = "ctxmenu_bleibt".into();
        record_in(&file, &other).expect("records");

        // Typed by hand, therefore lowercase: the registry does not care and
        // the comparison must not either.
        let typed = r"HKCU\SOFTWARE\Classes\*\shell\ctxmenu_snapotter__metadaten_entfernen";
        let target = RegTarget::parse(typed).expect("the CLI accepts this path");
        forget_target_in(&file, &target).expect("forgets");

        let after = recorded_in(&file).expect("readable");
        assert!(
            !after
                .iter()
                .any(|f| f.key_name == e.key_name && f.category == Category::AllFiles),
            "the deleted all-files entry is still recorded"
        );
        assert!(
            after
                .iter()
                .any(|f| f.key_name == e.key_name && f.category == twin.category),
            "the same key name in another category belongs to another registry \
             key and must survive"
        );
        assert_eq!(after.len(), 2, "only the deleted entry may disappear");
    }

    #[test]
    fn a_record_that_does_not_parse_is_reported_rather_than_read_as_empty() {
        // It used to come back as `Ok(vec![])`, and the next `record_in` wrote
        // that empty list back with one entry appended — every earlier line
        // gone, and nothing in the `Result` to say so.
        let scratch = Scratch::new("damaged-read");
        let file = scratch.entries();

        std::fs::write(&file, "[{\"key_name\": \"halb").expect("write");
        assert!(recorded_in(&file).is_err(), "damage has to be reported");

        // A file that is not there, and one that holds nothing but the
        // remains of an interrupted write, still mean "nothing created yet".
        let missing = scratch.0.join("nie-angelegt.json");
        assert!(recorded_in(&missing).expect("readable").is_empty());
        std::fs::write(&file, "   \r\n").expect("write");
        assert!(recorded_in(&file).expect("readable").is_empty());
    }

    #[test]
    fn a_damaged_record_is_carried_aside_instead_of_written_over() {
        // The user's own bookkeeping. Overwriting it costs the list of what
        // this tool created; moving it aside costs a file name.
        let scratch = Scratch::new("damaged-write");
        let file = scratch.entries();
        let damaged = "[{\"category\": \"Directory\", \"key_name\": \"ctxmenu_alt\"";
        std::fs::write(&file, damaged).expect("write");

        let mut e = entry(Category::Directory, r#""C:\t.exe" "%1""#);
        e.key_name = "ctxmenu_neu".into();

        let note = record_in(&file, &e).expect("a damaged file must not stop a create");
        assert!(note.is_some(), "the user has to be told the file was moved");

        let aside = file.with_extension("json.beschaedigt");
        assert_eq!(
            std::fs::read_to_string(&aside).expect("the rescue copy is there"),
            damaged,
            "byte for byte, or there was nothing to rescue"
        );

        let after = recorded_in(&file).expect("the new file parses");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].key_name, "ctxmenu_neu");
    }

    #[test]
    fn the_record_is_written_beside_itself_and_renamed_into_place() {
        // `fs::write` truncates first, and the release profile aborts on
        // panic: an interruption in the middle of it is exactly the damage
        // above. Every writer here goes through `store`, so no reader ever
        // sees half a document -- and none of them leaves the working file
        // behind either.
        let scratch = Scratch::new("atomic");
        let file = scratch.entries();
        let mut e = entry(Category::Directory, r#""C:\t.exe" "%1""#);
        e.key_name = "ctxmenu_atomar".into();

        record_in(&file, &e).expect("records");
        assert!(!file.with_extension("json.neu").exists());

        forget_in(&file, &e.category, &e.key_name).expect("forgets");
        assert!(!file.with_extension("json.neu").exists());
        assert!(recorded_in(&file).expect("readable").is_empty());
    }

    #[test]
    fn a_record_written_twice_keeps_one_line_per_key() {
        let scratch = Scratch::new("replace");
        let file = scratch.entries();

        let mut first = entry(Category::Directory, r#""C:\a.exe" "%1""#);
        first.key_name = "ctxmenu_doppelt".into();
        record_in(&file, &first).expect("records");

        let mut again = first.clone();
        again.command = r#""C:\b.exe" "%1""#.into();
        record_in(&file, &again).expect("records");

        let after = recorded_in(&file).expect("readable");
        assert_eq!(after.len(), 1, "the same key must not be listed twice");
        assert_eq!(after[0].command, r#""C:\b.exe" "%1""#);
    }

    #[test]
    fn an_entry_whose_record_cannot_be_written_is_still_created() {
        // The finding this fixes: the registry tree is complete before
        // entries.json is even opened, and the create used to come back as a
        // failure anyway. The user then saw a red box, the list was not
        // refreshed, and the second attempt failed with "key already exists"
        // for an entry they were told had not been made.
        const EXT: &str = ".ctxmenu_selftest_record";
        let _branch = Branch(EXT);
        let scratch = Scratch::new("unwritable");

        // A record whose parent directory is a file: creating it fails the way
        // a full disk or a locked file would, and does so on every machine.
        let blocker = scratch.0.join("blocker");
        std::fs::write(&blocker, b"not a directory").expect("write");
        let file = blocker.join("entries.json");

        let mut e = entry(Category::ExtAssoc(EXT.into()), "cmd /c echo ok");
        e.key_name = "ctxmenu_record_selftest".into();

        let made = create_in(&file, &e).expect("the entry itself is what counts");
        assert!(
            crate::registry::write::exists(&made.target),
            "the key has to be in the registry"
        );
        assert!(
            made.note.is_some(),
            "and the user has to be told the bookkeeping failed"
        );
        assert!(!file.exists());
    }

    #[test]
    fn an_entry_that_exists_fills_the_form_it_would_have_been_made_from() {
        use crate::model::{ContextEntry, EntryKind};

        let mut scanned: ContextEntry = crate::synthetic::scan_result(1).entries.remove(0);
        scanned.category = Category::Directory;
        scanned.key_name = "ctxmenu_probe".into();
        scanned.display_name = "Angesehen".into();
        scanned.icon_ref = Some(r"C:\Windows\system32\shell32.dll,-244".into());
        scanned.position = Some("Top".into());
        scanned.extended = true;
        scanned.kind = EntryKind::Verb {
            command: Some(r#""C:\t.exe" "%1""#.into()),
            sub_commands: Vec::new(),
        };

        let form = NewEntry::from_scanned(&scanned);
        assert_eq!(form.category, Category::Directory);
        assert_eq!(form.key_name, "ctxmenu_probe");
        assert_eq!(form.display_name, "Angesehen");
        assert_eq!(form.command, r#""C:\t.exe" "%1""#);
        assert_eq!(form.position.as_deref(), Some("Top"));
        assert!(form.extended);
        assert!(!form.is_submenu());

        // A cascading entry comes back as a submenu, children and all.
        let mut child = scanned.clone();
        child.key_name = "01_Kind".into();
        child.display_name = "Kind".into();
        child.icon_ref = None;
        child.kind = EntryKind::Verb {
            command: Some("cmd /c echo kind".into()),
            sub_commands: Vec::new(),
        };
        scanned.kind = EntryKind::Verb {
            command: None,
            sub_commands: vec![child],
        };

        let form = NewEntry::from_scanned(&scanned);
        assert!(form.is_submenu());
        assert_eq!(form.command, "", "a submenu parent runs nothing");
        assert_eq!(form.children.len(), 1);
        assert_eq!(form.children[0].key_name, "01_Kind");
        assert_eq!(form.children[0].command, "cmd /c echo kind");

        // A COM handler has no command line anywhere in the registry — its
        // text is made at run time — so none is invented for it.
        scanned.kind = EntryKind::ShellEx {
            clsid: "{00000000-0000-0000-0000-000000000000}".into(),
            server_path: Some(r"C:\Windows\system32\shell32.dll".into()),
            blocked: false,
        };
        let form = NewEntry::from_scanned(&scanned);
        assert_eq!(form.command, "");
        assert!(form.children.is_empty());
    }

    #[test]
    fn what_the_target_refuses_is_said_in_words_here() {
        // The editor shows this list and nothing else now, so everything
        // `target()` would refuse has to turn up in it — otherwise the button
        // is dead with no reason on screen.
        let e = entry(Category::ExtAssoc(String::new()), "x");
        assert!(e.target().is_err(), "no extension typed yet");
        assert!(
            check(&e)
                .iter()
                .any(|p| matches!(p.fault(), Fault::CategoryNotCreatable))
        );

        // `shell` passes every check above — not empty, no backslash — and is
        // still refused, because that key holds the entries rather than being
        // one. It is the one way to reach this case from the form.
        let mut e = entry(Category::Directory, "x");
        e.key_name = "shell".into();
        assert!(e.target().is_err());
        assert!(
            check(&e)
                .iter()
                .any(|p| matches!(p.fault(), Fault::UnusableKeyName))
        );

        // The everyday case keeps its plain wording: an empty key name is
        // "key name is missing", not "that key name is not allowed".
        let mut e = entry(Category::Directory, "x");
        e.key_name = String::new();
        let problems = check(&e);
        assert!(
            problems
                .iter()
                .any(|p| matches!(p.fault(), Fault::MissingKeyName))
        );
        assert!(
            !problems
                .iter()
                .any(|p| matches!(p.fault(), Fault::UnusableKeyName)),
            "one cause, one line: {problems:?}"
        );

        // A complete entry stays quiet.
        let mut e = entry(Category::Directory, r#""C:\t.exe" "%1""#);
        e.key_name = "ctxmenu_ok".into();
        assert!(check(&e).is_empty(), "{:?}", check(&e));
    }

    #[test]
    fn a_submenu_needs_no_command_of_its_own_but_its_children_do() {
        let mut e = entry(Category::Directory, "");
        // Without children this is the old rule and still an error.
        assert!(check(&e).iter().any(Problem::is_error));

        e.children = vec![child("Erster", r#""C:\t.exe" "%1""#)];
        assert!(
            !check(&e).iter().any(Problem::is_error),
            "a submenu is complete without a command line: {:?}",
            check(&e)
        );

        // A child without one is exactly as broken as an entry without one,
        // and the message has to say which row.
        e.children.push(child("Zweiter", "  "));
        e.children[1].key_name = suggest_child_key_name(1, "Zweiter");
        assert!(
            check(&e)
                .iter()
                .any(|p| matches!(p.fault(), Fault::ChildMissingCommand(2))),
            "got {:?}",
            check(&e)
        );

        // Nameless likewise, and numbered the same way: rows, not indices.
        e.children[1] = child("", "x");
        e.children[1].key_name = suggest_child_key_name(1, "");
        assert!(
            check(&e)
                .iter()
                .any(|p| matches!(p.fault(), Fault::ChildMissingDisplayName(2)))
        );
    }

    #[test]
    fn a_command_beside_a_submenu_is_said_out_loud_rather_than_dropped() {
        // It is not written — a submenu parent has no `command` subkey — and a
        // command line that disappears without a word looks exactly like one
        // that was written and does not work.
        let mut e = entry(Category::Directory, r#""C:\t.exe" "%1""#);
        e.children = vec![child("Erster", "x")];

        let problems = check(&e);
        assert!(!problems.iter().any(Problem::is_error));
        assert!(
            problems
                .iter()
                .any(|p| matches!(p.fault(), Fault::CommandBesideSubmenu))
        );
    }

    #[test]
    fn two_children_that_would_share_one_key_are_refused() {
        // Registry key names are case-insensitive, so these are one key: the
        // second would overwrite the first and the menu would be one entry
        // short with nothing to say why.
        let mut e = entry(Category::Directory, "");
        e.children = vec![child("Doppelt", "a"), child("Doppelt", "b")];
        e.children[1].key_name = e.children[0].key_name.to_uppercase();

        assert!(
            check(&e)
                .iter()
                .any(|p| matches!(p.fault(), Fault::DuplicateChildKeyName(_)))
        );
    }

    #[test]
    fn the_percent_one_warning_follows_the_commands_that_are_really_written() {
        // The trap is in the children now, and the parent's own command field
        // is not written at all — so it must neither trigger the warning nor
        // hide it.
        let mut e = entry(Category::DirectoryBackground, r#""C:\t.exe" "%V""#);
        e.children = vec![child("Kind", r#""C:\t.exe" "%1""#)];
        assert!(
            check(&e)
                .iter()
                .any(|p| matches!(p.fault(), Fault::PercentOneInBackground)),
            "a child's %1 is the same trap as the parent's"
        );

        // The other way round: the ignored parent command carries %1, every
        // child is correct. Warning about it would be warning about a value
        // nobody will ever run.
        let mut e = entry(Category::DirectoryBackground, r#""C:\t.exe" "%1""#);
        e.children = vec![child("Kind", r#""C:\t.exe" "%V""#)];
        assert!(
            !check(&e)
                .iter()
                .any(|p| matches!(p.fault(), Fault::PercentOneInBackground))
        );
    }

    #[test]
    fn child_key_names_carry_the_order_the_user_chose() {
        // The registry hands subkeys back alphabetically whatever order they
        // were written in, so the number in front is the whole mechanism.
        let names = ["Zebra", "Anton"];
        let keys: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, name)| suggest_child_key_name(i, name))
            .collect();

        assert_eq!(keys, ["01_Zebra", "02_Anton"]);
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            sorted, keys,
            "sorted alphabetically, Zebra still comes first"
        );

        // Two digits, or ten children would sort in front of two.
        assert_eq!(suggest_child_key_name(9, "x"), "10_x");
        assert!(suggest_child_key_name(1, "10_x") < suggest_child_key_name(9, "a"));

        // A row with no name yet must still produce a usable key.
        assert_eq!(suggest_child_key_name(0, "  "), "01_eintrag");
        assert!(!suggest_child_key_name(0, r"a\b").contains('\\'));
    }

    #[test]
    fn an_entries_json_written_before_submenus_still_reads() {
        // `entries.json` is the user's file and predates this field; a strict
        // deserialiser would make the editor's "already created" list empty
        // and, later, cost the Windows 11 handler every entry it knows.
        let old = r#"[{
            "category": "Directory",
            "key_name": "ctxmenu_alt",
            "display_name": "Alt",
            "command": "cmd /c echo alt",
            "icon": null,
            "position": null,
            "extended": false
        }]"#;

        let parsed: Vec<NewEntry> = serde_json::from_str(old).expect("old records still read");
        assert_eq!(parsed.len(), 1);
        assert!(!parsed[0].is_submenu());
        assert!(parsed[0].children.is_empty());
    }

    #[test]
    fn the_suggested_key_name_is_usable_as_one() {
        assert_eq!(
            suggest_key_name("Mit Tool öffnen"),
            "ctxmenu_Mit_Tool_öffnen"
        );
        assert_eq!(suggest_key_name("  &  "), "ctxmenu_eintrag");
        assert!(!suggest_key_name(r"a\b/c").contains('\\'));
    }

    /// Removes an invented file-type branch when the test ends.
    ///
    /// `Drop` rather than a line at the end of the test body: a failing
    /// assertion unwinds, and a key under a made-up extension would then stay
    /// in the user's registry for good. The extension is invented precisely so
    /// that these tests write where no real menu can see them — unlike
    /// `Directory\shell`, which is the menu the user gets on every folder.
    struct Branch(&'static str);

    impl Drop for Branch {
        fn drop(&mut self) {
            let _ = CURRENT_USER.remove_tree(format!(
                r"SOFTWARE\Classes\SystemFileAssociations\{}",
                self.0
            ));
        }
    }

    #[test]
    fn a_submenu_is_written_as_muiverb_with_an_empty_subcommands() {
        // The measured shape, not an invented one: of the 15 submenu parents
        // on this machine every single one names itself in `MUIVerb`, carries
        // an empty `SubCommands` and has no `command` subkey at all.
        const EXT: &str = ".ctxmenu_selftest_submenu";
        let _branch = Branch(EXT);

        let mut e = entry(Category::ExtAssoc(EXT.into()), "");
        e.key_name = "ctxmenu_submenu".into();
        e.display_name = "Selbsttest".into();
        e.children = vec![
            child("Zebra", "cmd /c echo z"),
            child("Anton", "cmd /c echo a"),
        ];
        for (index, c) in e.children.iter_mut().enumerate() {
            c.key_name = suggest_child_key_name(index, &c.display_name);
        }
        e.children[1].icon = Some(r"%SystemRoot%\system32\shell32.dll,-244".into());

        let target = e.target().expect("creatable");
        // `write_tree` rather than `create`: this is about the registry shape,
        // and `create` would also write the user's `entries.json`.
        write_tree(&e, &target).expect("HKCU is writable without elevation");

        let key = CURRENT_USER
            .open(target.key_path())
            .expect("parent written");
        assert_eq!(
            key.get_string("MUIVerb").ok().as_deref(),
            Some("Selbsttest")
        );
        assert_eq!(
            key.get_string("SubCommands").ok().as_deref(),
            Some(""),
            "empty is the marker; a value would name CommandStore verbs instead"
        );
        assert!(
            key.get_string("").ok().is_none_or(|v| v.is_empty()),
            "a submenu parent has no default value"
        );
        assert!(
            CURRENT_USER
                .open(format!(r"{}\command", target.key_path()))
                .is_err(),
            "a submenu runs nothing itself"
        );

        // The children, in the order they were entered rather than the order
        // the registry would sort them in on its own.
        let shell = CURRENT_USER
            .open(format!(r"{}\shell", target.key_path()))
            .expect("children live in the parent's own shell subkey");
        let names: Vec<String> = shell.keys().expect("enumerable").collect();
        assert_eq!(names, ["01_Zebra", "02_Anton"]);

        let first = shell.open("01_Zebra").expect("first child");
        assert_eq!(first.get_string("MUIVerb").ok().as_deref(), Some("Zebra"));
        assert_eq!(
            first
                .open("command")
                .and_then(|c| c.get_string(""))
                .ok()
                .as_deref(),
            Some("cmd /c echo z")
        );
        assert!(
            shell
                .open("02_Anton")
                .and_then(|c| c.get_string("Icon"))
                .is_ok(),
            "a child keeps its own icon"
        );
    }

    #[test]
    fn a_child_that_cannot_be_written_takes_the_half_built_submenu_with_it() {
        // Otherwise the user is left with a menu item that opens onto an empty
        // box — and nobody asked for one.
        const EXT: &str = ".ctxmenu_selftest_rollback";
        let _branch = Branch(EXT);
        let scratch = Scratch::new("rollback");

        let mut e = entry(Category::ExtAssoc(EXT.into()), "");
        e.key_name = "ctxmenu_rollback".into();
        e.children = vec![
            child("Gut", "cmd /c echo ok"),
            NewChild {
                // Past the 255 character limit for a key name, so the write
                // fails at the second child with the first already in place.
                key_name: "x".repeat(600),
                display_name: "Zu lang".into(),
                command: "cmd /c echo no".into(),
                icon: None,
            },
        ];

        let target = e.target().expect("creatable");
        assert!(
            create_in(&scratch.entries(), &e).is_err(),
            "a 600 character key name cannot be written"
        );
        assert!(
            !crate::registry::write::exists(&target),
            "the half-written submenu must be gone, parent included"
        );
        assert!(
            recorded_in(&scratch.entries())
                .expect("readable")
                .is_empty(),
            "a failed write must not be recorded as created"
        );
    }

    #[test]
    fn an_unusual_position_warns_but_is_kept() {
        let mut e = entry(Category::Directory, "x");
        e.position = Some("Last".into());

        let problems = check(&e);
        assert!(!problems.iter().any(Problem::is_error));
        assert!(
            problems
                .iter()
                .any(|p| matches!(p.fault(), Fault::UnusualPosition(value) if value == "Last"))
        );
    }
}
