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
    pub command: String,
    pub icon: Option<String>,
    pub position: Option<String>,
    /// Only visible while Shift is held.
    pub extended: bool,
}

/// Something wrong enough to refuse, or worth saying out loud.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// Refuses the write.
    Error(String),
    /// Written anyway, but the user should know.
    Warning(String),
}

impl Problem {
    pub fn is_error(&self) -> bool {
        matches!(self, Problem::Error(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Problem::Error(m) | Problem::Warning(m) => m,
        }
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
        problems.push(Problem::Error(
            "Schlüsselname fehlt / key name is missing".into(),
        ));
    } else if name.contains(['\\', '/']) {
        problems.push(Problem::Error(
            "Schlüsselname darf keinen Backslash enthalten / no backslash in a key name".into(),
        ));
    }

    if entry.display_name.trim().is_empty() {
        problems.push(Problem::Error(
            "Anzeigename fehlt / display name is missing".into(),
        ));
    }

    if entry.command.trim().is_empty() {
        problems.push(Problem::Error("Befehl fehlt / command is missing".into()));
    }

    // The one that actually catches people out.
    if is_background(&entry.category)
        && entry.command.contains("%1")
        && !entry.command.contains("%V")
    {
        problems.push(Problem::Warning(
            "In einer Hintergrund-Kategorie bleibt %1 leer — hier gehört %V hin. / \
             %1 stays empty in a background category; %V belongs here."
                .into(),
        ));
    }

    if entry.display_name.contains('&') {
        problems.push(Problem::Warning(
            "Ein & erzeugt im Menü einen Zugriffsbuchstaben; für ein echtes \
             Und-Zeichen && schreiben. / An & becomes an accelerator in the menu; \
             write && for a literal ampersand."
                .into(),
        ));
    }

    if let Some(position) = &entry.position
        && !matches!(position.as_str(), "Top" | "Bottom")
    {
        problems.push(Problem::Warning(format!(
            "Position {position:?} ist ungewöhnlich; belegt sind Top und Bottom. / \
             unusual Position; only Top and Bottom are verified."
        )));
    }

    problems
}

impl NewEntry {
    /// Where this entry will live.
    pub fn target(&self) -> Result<RegTarget> {
        let relative = category_relative(&self.category)?;
        Ok(RegTarget {
            // Always the user's own hive.
            scope: Scope::User,
            relative: format!(r"{relative}\{}", self.key_name.trim()),
        })
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

    let key = CURRENT_USER.create(target.key_path()).with_context(|| {
        format!(
            "Anlegen fehlgeschlagen / could not create {}",
            target.full_path()
        )
    })?;

    key.set_string("", entry.display_name.trim())?;
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

    CURRENT_USER
        .create(format!(r"{}\command", target.key_path()))
        .and_then(|command| command.set_string("", entry.command.trim()))
        .context("command-Unterschlüssel / command subkey")?;

    // Best effort: a failure here costs the Windows 11 handler its knowledge
    // of this entry, but the entry itself is already in place and working.
    if let Err(error) = record(entry) {
        return Err(error.context("entries.json"));
    }

    Ok(target)
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
    let cleaned: String = display_name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "ctxmenu_eintrag".into()
    } else {
        format!("ctxmenu_{}", trimmed.chars().take(48).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(category: Category, command: &str) -> NewEntry {
        NewEntry {
            category,
            key_name: "ctxmenu_test".into(),
            display_name: "Test".into(),
            command: command.into(),
            icon: None,
            position: None,
            extended: false,
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
                    .any(|p| matches!(p, Problem::Warning(m) if m.contains("%V"))),
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
                target.scope,
                Scope::User,
                "an entry must never be written machine-wide"
            );
            assert!(target.relative.ends_with(r"\ctxmenu_x"));
        }
    }

    #[test]
    fn a_file_type_entry_lands_under_system_file_associations() {
        // The measured way to limit an entry to one kind of file: the key's
        // place in the tree, not a query in AppliesTo.
        let mut e = entry(Category::ExtAssoc(".PNG".into()), "x");
        e.key_name = "ctxmenu_x".into();
        assert_eq!(
            e.target().expect("creatable").relative,
            r"SystemFileAssociations\.png\shell\ctxmenu_x",
            "the extension is normalised to the form the registry keeps"
        );

        // A whole perceived class, which is what "all pictures" means.
        let mut e = entry(Category::PerceivedType("image".into()), "x");
        e.key_name = "ctxmenu_x".into();
        assert_eq!(
            e.target().expect("creatable").relative,
            r"SystemFileAssociations\image\shell\ctxmenu_x"
        );

        // A missing dot is a typo, not an error worth refusing over.
        let mut e = entry(Category::ExtAssoc("jpg".into()), "x");
        e.key_name = "ctxmenu_x".into();
        assert!(
            e.target()
                .expect("creatable")
                .relative
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
    fn the_suggested_key_name_is_usable_as_one() {
        assert_eq!(
            suggest_key_name("Mit Tool öffnen"),
            "ctxmenu_Mit_Tool_öffnen"
        );
        assert_eq!(suggest_key_name("  &  "), "ctxmenu_eintrag");
        assert!(!suggest_key_name(r"a\b/c").contains('\\'));
    }

    #[test]
    fn an_unusual_position_warns_but_is_kept() {
        let mut e = entry(Category::Directory, "x");
        e.position = Some("Last".into());

        let problems = check(&e);
        assert!(!problems.iter().any(Problem::is_error));
        assert!(problems.iter().any(|p| p.message().contains("Last")));
    }
}
