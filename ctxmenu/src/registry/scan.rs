//! Reads context menu entries out of the registry.
//!
//! The scanner never merges hives. `HKCR` is a merged view where `HKCU` wins,
//! but showing only the winner would hide both the origin of an entry and
//! whether it can be removed without elevation — so every hive contributes its
//! own entries and the scope travels with them.

use windows_registry::Key;

use super::paths::{self, CategorySource, SourceKind};
use crate::model::{Category, ContextEntry, EntryKind, ScanProgress, ScanResult, Scope, stable_id};

/// How deep cascading submenus are followed.
///
/// Three levels is past anything seen in the wild and stops a malformed key
/// that points at itself from recursing forever.
const MAX_SUBMENU_DEPTH: usize = 3;

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub scopes: Vec<Scope>,
    /// `None` means every base category.
    pub categories: Option<Vec<Category>>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            scopes: Scope::ALL.to_vec(),
            categories: None,
        }
    }
}

/// Scans the requested locations.
///
/// `progress` is called once per (location, scope) pair. Milestone 4 sends
/// those reports down an mpsc channel so the list fills visibly; the scanner
/// itself does not care who listens.
pub fn scan(options: &ScanOptions, mut progress: impl FnMut(ScanProgress)) -> ScanResult {
    let sources: Vec<CategorySource> = paths::base_sources()
        .into_iter()
        .filter(|s| {
            options
                .categories
                .as_ref()
                .is_none_or(|wanted| wanted.contains(&s.category))
        })
        .collect();

    let total = sources.len() * options.scopes.len();
    let mut entries = Vec::new();
    let mut done = 0;

    for source in &sources {
        for &scope in &options.scopes {
            done += 1;

            // A missing location is the normal case, not an error: no machine
            // has every category populated in every hive.
            if let Ok(key) = paths::root_key(scope).open(paths::key_path(scope, source.relative)) {
                collect(&key, source, scope, &mut entries);
            }

            progress(ScanProgress {
                done,
                total,
                label: paths::display_path(scope, source.relative),
                found: entries.len(),
            });
        }
    }

    ScanResult::new(entries)
}

fn collect(key: &Key, source: &CategorySource, scope: Scope, out: &mut Vec<ContextEntry>) {
    let Ok(names) = key.keys() else { return };

    for name in names {
        let relative = format!("{}\\{}", source.relative, name);
        let entry = match source.kind {
            SourceKind::Shell => read_verb(key, &name, &relative, scope, &source.category, 0),
            SourceKind::ShellEx => read_shellex(key, &name, &relative, scope, &source.category),
        };
        if let Some(entry) = entry {
            out.push(entry);
        }
    }
}

/// Reads a static verb key.
///
/// `relative` is the path below the classes root, used to build the full
/// registry path that backup and delete operate on.
fn read_verb(
    parent: &Key,
    name: &str,
    relative: &str,
    scope: Scope,
    category: &Category,
    depth: usize,
) -> Option<ContextEntry> {
    let key = parent.open(name).ok()?;

    // MUIVerb takes precedence over the default value. Resolving the
    // `@file,-id` indirection happens in milestone 2; until then the raw
    // string is shown, which is still better than nothing.
    let mui = non_empty(key.get_string("MUIVerb").ok());
    let default = non_empty(key.get_string("").ok());
    let raw_display = mui.or(default);
    let display_name = raw_display.clone().unwrap_or_else(|| name.to_string());

    let command = key
        .open("command")
        .ok()
        .and_then(|c| non_empty(c.get_string("").ok()));

    let sub_commands = if depth < MAX_SUBMENU_DEPTH {
        read_sub_commands(&key, relative, scope, category, depth + 1)
    } else {
        Vec::new()
    };

    let registry_path = paths::display_path(scope, relative);

    Some(ContextEntry {
        id: stable_id(scope, &registry_path),
        key_name: name.to_string(),
        display_name,
        raw_display,
        icon_ref: non_empty(key.get_string("Icon").ok()),
        position: non_empty(key.get_string("Position").ok()),
        // These are presence flags: the value is empty, only its existence
        // matters, and some installers write them as REG_NONE. Asking for the
        // type rather than the string keeps those visible.
        extended: key.get_type("Extended").is_ok(),
        hidden: key.get_type("LegacyDisable").is_ok()
            || key.get_type("ProgrammaticAccessOnly").is_ok(),
        applies_to: non_empty(key.get_string("AppliesTo").ok()),
        kind: EntryKind::Verb {
            command,
            sub_commands,
        },
        scope,
        category: category.clone(),
        registry_path,
        read_only: !is_writable(parent, name),
        program_key: None,
    })
}

/// Follows a cascading menu: children live under `<verb>\shell\<child>`.
fn read_sub_commands(
    key: &Key,
    relative: &str,
    scope: Scope,
    category: &Category,
    depth: usize,
) -> Vec<ContextEntry> {
    let Ok(shell) = key.open("shell") else {
        return Vec::new();
    };
    let Ok(names) = shell.keys() else {
        return Vec::new();
    };

    names
        .filter_map(|name| {
            let child_relative = format!("{relative}\\shell\\{name}");
            read_verb(&shell, &name, &child_relative, scope, category, depth)
        })
        .collect()
}

/// Reads a COM handler registration.
///
/// Only the registration is visible here. The text the user actually sees is
/// generated at runtime by `IContextMenu::QueryContextMenu` and is not stored
/// anywhere in the registry, so it cannot be shown or edited (ToDo 5.4).
fn read_shellex(
    parent: &Key,
    name: &str,
    relative: &str,
    scope: Scope,
    category: &Category,
) -> Option<ContextEntry> {
    let key = parent.open(name).ok()?;
    let default = non_empty(key.get_string("").ok());

    // Usually the CLSID is the default value, but plenty of installers name
    // the subkey after the CLSID and leave the value empty.
    let clsid = match default.clone() {
        Some(v) if looks_like_guid(&v) => v,
        other => {
            if looks_like_guid(name) {
                name.to_string()
            } else {
                other.unwrap_or_default()
            }
        }
    };

    let registry_path = paths::display_path(scope, relative);

    Some(ContextEntry {
        id: stable_id(scope, &registry_path),
        key_name: name.to_string(),
        display_name: name.to_string(),
        raw_display: default,
        icon_ref: None,
        position: None,
        extended: false,
        hidden: false,
        applies_to: None,
        kind: EntryKind::ShellEx {
            clsid,
            // Both are filled in milestone 2 once CLSID resolution exists.
            server_path: None,
            blocked: false,
        },
        scope,
        category: category.clone(),
        registry_path,
        read_only: !is_writable(parent, name),
        program_key: None,
    })
}

/// Can this key be opened for writing?
///
/// Asking the registry beats guessing from the elevation state: a HKLM key may
/// be writable for an elevated process, and a HKCU key may still be locked
/// down by an ACL.
fn is_writable(parent: &Key, name: &str) -> bool {
    parent.options().read().write().open(name).is_ok()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

fn looks_like_guid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && bytes[9] == b'-'
        && bytes[14] == b'-'
        && bytes[19] == b'-'
        && bytes[24] == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_shape_is_recognised() {
        assert!(looks_like_guid("{23170F69-40C1-278A-1000-000100020000}"));
        assert!(!looks_like_guid("7-Zip"));
        assert!(!looks_like_guid("{23170F69-40C1-278A-1000-00010002000}"));
        assert!(!looks_like_guid(""));
    }

    #[test]
    fn blank_values_are_treated_as_absent() {
        assert_eq!(non_empty(Some("   ".into())), None);
        assert_eq!(non_empty(Some(String::new())), None);
        assert_eq!(non_empty(Some("x".into())), Some("x".into()));
        assert_eq!(non_empty(None), None);
    }

    /// Reads a location that exists on every Windows installation. Guards
    /// against the scanner silently returning nothing.
    #[test]
    fn scanning_directory_finds_entries_on_this_machine() {
        let options = ScanOptions {
            scopes: Scope::ALL.to_vec(),
            categories: Some(vec![Category::Directory]),
        };
        let mut calls = 0;
        let result = scan(&options, |_| calls += 1);

        assert_eq!(calls, 2 * 3, "two locations times three scopes");
        assert!(
            !result.entries.is_empty(),
            "HKLM\\SOFTWARE\\Classes\\Directory\\shell is never empty"
        );
        assert!(result.entries.iter().all(|e| !e.id.is_empty()));
        assert!(
            result
                .entries
                .iter()
                .all(|e| e.category == Category::Directory)
        );
    }
}
