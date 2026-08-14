//! Creating one's own context menu entries.
//!
//! Written to `HKCU\SOFTWARE\Classes` and nowhere else: no elevation needed,
//! nothing system-wide broken if it goes wrong, and removable by the same user
//! who added it (ToDo 5.2).
//!
//! Every entry is *also* recorded in `entries.json`. That is not a cache — it
//! is preparation for ToDo 14, where a Windows 11 `IExplorerCommand` handler
//! reads this file and builds its entries from it. Writing it now means the
//! DLL has to be built and signed exactly once, and the interface keeps
//! writing nothing but JSON.

use std::path::PathBuf;

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
    /// common mistake in hand-written entries (ToDo 5.3).
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
    /// Both languages at once, for the console and the log.
    pub fn bilingual(&self) -> String {
        match self {
            Fault::MissingKeyName => "Schlüsselname fehlt / key name is missing".into(),
            Fault::BackslashInKeyName => {
                "Schlüsselname darf keinen Backslash enthalten / no backslash in a key name".into()
            }
            Fault::MissingDisplayName => "Anzeigename fehlt / display name is missing".into(),
            Fault::MissingCommand => "Befehl fehlt / command is missing".into(),
            Fault::PercentOneInBackground => {
                "In einer Hintergrund-Kategorie bleibt %1 leer — hier gehört %V hin. / \
                 %1 stays empty in a background category; %V belongs here."
                    .into()
            }
            Fault::AmpersandInDisplayName => {
                "Ein & erzeugt im Menü einen Zugriffsbuchstaben; für ein echtes \
                 Und-Zeichen && schreiben. / An & becomes an accelerator in the menu; \
                 write && for a literal ampersand."
                    .into()
            }
            Fault::UnusualPosition(value) => format!(
                "Position {value:?} ist ungewöhnlich; belegt sind Top und Bottom. / \
                 unusual Position; only Top and Bottom are verified."
            ),
            Fault::CommandBesideSubmenu => {
                "Ein Untermenü führt selbst nichts aus; der Befehl wird nicht geschrieben. / \
                 a submenu runs nothing itself; the command will not be written."
                    .into()
            }
            Fault::CategoryNotCreatable => "Hier kann kein eigener Eintrag angelegt werden. / \
                 no entry of one's own can be created here."
                .into(),
            Fault::UnusableKeyName => "Dieser Schlüsselname ist hier nicht erlaubt. / \
                 that key name is not allowed here."
                .into(),
            Fault::ChildMissingDisplayName(n) => {
                format!("Untereintrag {n} hat keinen Anzeigenamen / submenu entry {n} has no name")
            }
            Fault::ChildMissingCommand(n) => {
                format!("Untereintrag {n} hat keinen Befehl / submenu entry {n} has no command")
            }
            Fault::DuplicateChildKeyName(name) => format!(
                "Zwei Untereinträge heißen {name:?} / two submenu entries are called {name:?}"
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
        self.fault().bilingual()
    }
}

/// Categories where the clicked object is the *folder being looked at*, not a
/// selected item.
///
/// `%1` stays empty there and the entry silently does nothing — the single
/// most common mistake in hand-written entries (ToDo 5.3).
fn is_background(category: &Category) -> bool {
    matches!(
        category,
        Category::DirectoryBackground | Category::DesktopBackground
    )
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
/// on this machine not one uses the `System.ItemType:.txt` shape the ToDo
/// sketches — they filter by BitLocker state and storage provider. Placing the
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
                bail!("Ungültiger wahrgenommener Typ / invalid perceived type: {kind:?}");
            }
            format!(r"SystemFileAssociations\{kind}\shell")
        }

        other => bail!(
            "Für diese Kategorie können keine Einträge angelegt werden / cannot create entries for {other:?}"
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
        bail!("Keine gültige Dateiendung / not a valid extension: {ext:?}");
    }
    Ok(with_dot)
}

/// Writes the entry into HKCU and records it in `entries.json`.
///
/// Refuses on any [`Problem::Error`]; warnings are the caller's to show and
/// the user's to overrule.
pub fn create(entry: &NewEntry) -> Result<RegTarget> {
    let problems = check(entry);
    if let Some(error) = problems.iter().find(|p| p.is_error()) {
        bail!("{}", error.message());
    }

    let target = entry.target()?;
    if super::write::exists(&target) {
        bail!(
            "Schlüssel existiert bereits / key already exists: {}",
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

    // Best effort: a failure here costs the Windows 11 handler its knowledge
    // of this entry, but the entry itself is already in place and working.
    if let Err(error) = record(entry) {
        return Err(error.context("entries.json"));
    }

    Ok(target)
}

/// Writes the key, its values and whatever hangs below it.
fn write_tree(entry: &NewEntry, target: &RegTarget) -> Result<()> {
    let key = CURRENT_USER.create(target.key_path()).with_context(|| {
        format!(
            "Anlegen fehlgeschlagen / could not create {}",
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
            .context("command-Unterschlüssel / command subkey")?;
    }

    Ok(())
}

/// One child of a submenu: `<parent>\shell\<key>` and its `command`.
fn write_child(parent: &RegTarget, child: &NewChild) -> Result<()> {
    let path = format!(r"{}\shell\{}", parent.key_path(), child.key_name.trim());

    let key = CURRENT_USER.create(&path).with_context(|| {
        format!(
            "Untereintrag anlegen fehlgeschlagen / could not create submenu entry: {}",
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
        .with_context(|| format!("command-Unterschlüssel / command subkey: {path}"))?;

    Ok(())
}

/// `%LOCALAPPDATA%\ctxmenu\entries.json`
pub fn entries_path() -> Result<PathBuf> {
    let base = dirs::data_local_dir().context("kein LOCALAPPDATA / no local data directory")?;
    Ok(base.join("ctxmenu").join("entries.json"))
}

/// Everything this tool created, in the order it was created.
pub fn recorded() -> Result<Vec<NewEntry>> {
    let path = entries_path()?;
    match std::fs::read_to_string(&path) {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        // No file yet is not an error; nothing has been created.
        Err(_) => Ok(Vec::new()),
    }
}

fn record(entry: &NewEntry) -> Result<()> {
    let path = entries_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut all = recorded()?;
    all.retain(|existing| {
        existing.key_name != entry.key_name || existing.category != entry.category
    });
    all.push(entry.clone());

    std::fs::write(&path, serde_json::to_string_pretty(&all)?)
        .with_context(|| format!("{path:?}"))?;
    Ok(())
}

/// Forgets whatever was recorded for this registry key.
///
/// Called after a successful delete. Without it `entries.json` keeps naming an
/// entry the user has removed — and that file is the input for the Windows 11
/// handler of ToDo 14, so a stale line there would eventually put the deleted
/// item back in the menu.
pub fn forget_target(target: &RegTarget) -> Result<()> {
    let wanted = target.full_path().to_lowercase();
    let mut list = recorded()?;
    let before = list.len();

    list.retain(|entry| {
        entry
            .target()
            .map(|t| t.full_path().to_lowercase() != wanted)
            .unwrap_or(true)
    });

    if list.len() != before {
        std::fs::write(entries_path()?, serde_json::to_string_pretty(&list)?)?;
    }
    Ok(())
}

/// Forgets an entry in `entries.json`. The registry key is removed elsewhere,
/// through the ordinary plan path with its backup.
pub fn forget(category: &Category, key_name: &str) -> Result<()> {
    let path = entries_path()?;
    let mut all = recorded()?;
    let before = all.len();
    all.retain(|entry| entry.key_name != key_name || &entry.category != category);

    if all.len() != before {
        std::fs::write(&path, serde_json::to_string_pretty(&all)?)?;
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
            let mut e = entry(category, "x");
            e.key_name = "ctxmenu_x".into();
            let target = e.target().expect("base categories are creatable");
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

    #[test]
    fn deleting_a_key_forgets_what_was_recorded_for_it() {
        // The record and the key have to disappear together: entries.json is
        // the input for the Windows 11 handler, so a line that outlives its
        // key would put a deleted item back into the menu.
        let mut e = entry(Category::Directory, r#""C:\t.exe" "%1""#);
        e.key_name = "ctxmenu_forget_selftest".into();
        let target = e.target().expect("creatable");

        // Recorded without touching the registry: this is about the file.
        let mut list = recorded().expect("readable");
        let before = list.len();
        list.push(e.clone());
        std::fs::write(
            entries_path().expect("path"),
            serde_json::to_string_pretty(&list).expect("json"),
        )
        .expect("write");

        forget_target(&target).expect("forgets");

        // Checked on the entry itself, not on the total: this file belongs to
        // the machine and holds whatever the user created. A count assertion
        // would fail for reasons that have nothing to do with forgetting.
        let after = recorded().expect("readable");
        assert!(
            !after.iter().any(|f| f.key_name == e.key_name),
            "the deleted entry is still listed"
        );
        assert!(
            after.len() < before + 1,
            "forgetting must remove exactly the one entry"
        );
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
            create(&e).is_err(),
            "a 600 character key name cannot be written"
        );
        assert!(
            !crate::registry::write::exists(&target),
            "the half-written submenu must be gone, parent included"
        );
        assert!(
            !recorded()
                .expect("readable")
                .iter()
                .any(|r| r.key_name == e.key_name),
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
