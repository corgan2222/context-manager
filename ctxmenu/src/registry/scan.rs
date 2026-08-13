//! Reads context menu entries out of the registry.
//!
//! The scanner never merges hives. `HKCR` is a merged view where `HKCU` wins,
//! but showing only the winner would hide both the origin of an entry and
//! whether it can be removed without elevation — so every hive contributes its
//! own entries and the scope travels with them.

use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Registry::{HKEY, RegEnumKeyExW};
use windows::core::PWSTR;
use windows_registry::Key;

use super::clsid::{ClsidInfo, ClsidResolver};
use super::mui::MuiResolver;
use super::paths::{self, CategorySource, SourceKind};
use crate::model::{
    Category, ContextEntry, EntryKind, ScanProgress, ScanResult, ScanStats, Scope, stable_id,
};

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

    // Name resolution runs once over the finished tree rather than inside the
    // read path. That keeps reading and interpreting separable — and the two
    // resolvers share their caches across the whole scan, which is where the
    // saving is: a full scan hits the same shell32 references over and over.
    let mut mui = MuiResolver::new();
    let mut clsids = ClsidResolver::new();
    resolve_entries(&mut entries, &mut mui, &mut clsids);

    let (mui_cache_hits, mui_cache_misses) = mui.stats();
    ScanResult::new(
        entries,
        ScanStats {
            mui_cache_hits,
            mui_cache_misses,
            blocked_clsids: clsids.blocked_count(),
        },
    )
}

/// Turns raw registry values into what the user should actually read.
///
/// Split out from reading so the scanner stays a plain traversal, and so this
/// pass can be pointed at any entry tree in tests.
fn resolve_entries(
    entries: &mut [ContextEntry],
    mui: &mut MuiResolver,
    clsids: &mut ClsidResolver,
) {
    enum Resolved {
        Verb {
            display: Option<String>,
            program_key: Option<String>,
        },
        ShellEx {
            info: ClsidInfo,
            blocked: bool,
        },
    }

    for entry in entries.iter_mut() {
        let resolved = match &entry.kind {
            EntryKind::Verb { command, .. } => Resolved::Verb {
                display: entry.raw_display.as_deref().map(|raw| mui.resolve(raw)),
                // Which program this entry belongs to. Costs a few file
                // system probes per entry, which is why it runs here in the
                // scan worker and never in the frame path.
                program_key: command
                    .as_deref()
                    .and_then(crate::program::cmdline::program_key),
            },
            EntryKind::ShellEx { clsid, .. } => Resolved::ShellEx {
                info: clsids.resolve(clsid),
                blocked: clsids.is_blocked(clsid),
            },
        };

        match resolved {
            Resolved::Verb {
                display,
                program_key,
            } => {
                if let Some(display) = display {
                    entry.display_name = display;
                }
                entry.program_key = program_key;
            }
            Resolved::ShellEx { info, blocked } => {
                // Falls back to the subkey name set while reading. That is
                // still the best available label for a handler whose CLSID is
                // registered without a friendly name.
                if let Some(name) = &info.friendly_name {
                    entry.display_name = name.clone();
                }
                // The server DLL is the grouping key for the program view
                // (ToDo 5.4), so it is filled here rather than in milestone 8.
                entry.program_key = info.program_key.clone();

                if let EntryKind::ShellEx {
                    server_path,
                    blocked: is_blocked,
                    ..
                } = &mut entry.kind
                {
                    *server_path = info.server_path;
                    *is_blocked = blocked;
                }
            }
        }

        if let EntryKind::Verb { sub_commands, .. } = &mut entry.kind {
            resolve_entries(sub_commands, mui, clsids);
        }
    }
}

/// Enumerates subkey names, tolerating changes made while enumerating.
///
/// Deliberately not `Key::keys()` from `windows-registry`. That iterator sizes
/// its buffer once from `RegQueryInfoKeyW` and treats any later error as the
/// end of the enumeration. If a longer key name appears in between — an
/// installer running during a scan is enough — `RegEnumKeyExW` answers
/// `ERROR_MORE_DATA`, and the iterator silently stops early in release builds
/// while tripping a `debug_assert` in debug ones. Silently returning fewer
/// context menu entries than exist is the worst failure this tool could have,
/// so the enumeration grows its buffer and carries on instead.
fn subkey_names(key: &Key) -> Vec<String> {
    // Documented maximum key name length is 255 characters.
    const INITIAL: usize = 256;

    let handle = HKEY(key.as_raw());
    let mut buffer = vec![0u16; INITIAL + 1];
    let mut names = Vec::new();
    let mut index = 0u32;

    loop {
        let mut len = buffer.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                handle,
                index,
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };

        match status {
            ERROR_SUCCESS => {
                names.push(String::from_utf16_lossy(&buffer[..len as usize]));
                index += 1;
            }
            // Retry the same index with room to spare.
            ERROR_MORE_DATA => buffer.resize(buffer.len() * 2, 0),
            // ERROR_NO_MORE_ITEMS, or the key vanished mid-scan.
            _ => break,
        }
    }

    names
}

fn collect(key: &Key, source: &CategorySource, scope: Scope, out: &mut Vec<ContextEntry>) {
    for name in subkey_names(key) {
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

    subkey_names(&shell)
        .into_iter()
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

    /// Long key names must survive enumeration.
    ///
    /// Regression guard for the buffer sizing described on `subkey_names`:
    /// the previous implementation sized its buffer from a snapshot and
    /// dropped everything from the first oversized name onwards.
    #[test]
    fn enumeration_survives_names_at_the_registry_limit() {
        use windows_registry::CURRENT_USER;

        let class = r"SOFTWARE\Classes\ctxmenu_selftest_enum";
        let long = "l".repeat(255);
        let root = CURRENT_USER.create(class).expect("HKCU is writable");
        root.create("short").expect("short name");
        root.create(&long).expect("255 character name");

        let names = subkey_names(&root);

        assert!(names.iter().any(|n| n == "short"));
        assert!(
            names.iter().any(|n| n == &long),
            "the 255 character name must not be dropped"
        );
        assert_eq!(names.len(), 2, "and nothing may be lost either");

        let _ = CURRENT_USER.remove_tree(class);
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
