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
use super::mui::{self, MuiResolver};
use super::paths::{self, CategorySource, Location, SourceKind};
use crate::model::{
    Category, ContextEntry, EntryKind, FileTypeInfo, ScanProgress, ScanResult, ScanStats, Scope,
    stable_id,
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
    /// Extensions to walk the file type chain for (ToDo 10.1).
    ///
    /// Empty by default: the chain costs a few hundred extra key opens, and
    /// the command line has no use for it unless asked.
    pub file_types: Vec<String>,
    /// Read Windows' own verb stock as well (ToDo 5.5).
    ///
    /// On by default: it is one key open for 229 entries on this machine, and
    /// they are the only place a name from a `SubCommands` list can be looked
    /// up. Filtering by category turns it off unless it was asked for, so
    /// `--category directory` still means what it says.
    pub command_store: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            scopes: Scope::ALL.to_vec(),
            categories: None,
            file_types: Vec::new(),
            command_store: true,
        }
    }
}

impl ScanOptions {
    /// Everything, including the curated file types — what the window uses.
    ///
    /// `custom` is what the user added by hand; it was persisted from
    /// milestone 5 on and read by nobody until 2026-08-15, which made the
    /// setting a promise the program did not keep.
    pub fn with_file_types(custom: &[String]) -> Self {
        Self {
            file_types: super::filetypes::wanted(custom),
            ..Self::default()
        }
    }

    /// Every extension registered on this machine, not just the curated ones.
    ///
    /// Thirteen times the work of `with_file_types` on this machine, so it is
    /// something to ask for — a button in the window, a flag on the command
    /// line — and never what a start does on its own.
    pub fn with_every_installed_file_type() -> Self {
        Self {
            file_types: super::filetypes::installed(),
            ..Self::default()
        }
    }
}

/// Scans the requested locations.
///
/// `progress` is called once per (location, scope) pair. Milestone 4 sends
/// those reports down an mpsc channel so the list fills visibly; the scanner
/// itself does not care who listens.
pub fn scan(options: &ScanOptions, mut progress: impl FnMut(ScanProgress)) -> ScanResult {
    let wanted = |category: &Category| {
        options
            .categories
            .as_ref()
            .is_none_or(|list| list.contains(category))
    };

    let sources: Vec<CategorySource> = paths::base_sources()
        .into_iter()
        .filter(|s| wanted(&s.category))
        .collect();
    let with_command_store = options.command_store && wanted(&Category::CommandStore);

    // Levels 3 to 7 of the file type chain, one set of locations per
    // extension. Levels 1 and 2 are the base categories above and are
    // deliberately scanned once and reused (ToDo 10.4).
    let mut file_types: Vec<FileTypeInfo> = Vec::new();
    let mut file_type_sources = Vec::new();
    let mut seen_locations: rustc_hash::FxHashSet<(String, SourceKind)> = Default::default();
    for ext in &options.file_types {
        let resolution = super::filetypes::resolve(ext);
        // An unregistered type still belongs in the tree, with a count of
        // zero — "no entries" and "not looked at" must stay distinguishable.
        if resolution.registered {
            // Deduplicated, and this is not an optimisation but a
            // correctness fix. Thirteen image extensions all name
            // `SystemFileAssociations\image`, and several share a ProgID.
            // Scanning a location once per extension produced one copy of
            // each entry per extension, so `.jpg` reported 79 entries where
            // it has 19. Attribution below hands the single copy to every
            // type it belongs to, which is the correct sharing.
            for source in super::filetypes::sources_for(&resolution) {
                let fingerprint = (source.relative.to_lowercase(), source.kind);
                if seen_locations.insert(fingerprint) {
                    file_type_sources.push(source);
                }
            }
        }
        file_types.push(FileTypeInfo {
            group: super::filetypes::group_of(ext),
            resolution,
            // Filled in after the scan, by matching categories.
            entry_indices: Vec::new(),
        });
    }

    let total = (sources.len() + file_type_sources.len()) * options.scopes.len()
        + usize::from(with_command_store);
    let mut entries = Vec::new();
    let mut done = 0;

    for source in sources.iter().chain(file_type_sources.iter()) {
        for &scope in &options.scopes {
            done += 1;
            let at = paths::Location::classes(scope);

            // A missing location is the normal case, not an error: no machine
            // has every category populated in every hive.
            if let Ok(key) = paths::root_key(scope).open(at.key_path(&source.relative)) {
                collect(&key, source, at, &mut entries);
            }

            progress(ScanProgress {
                done,
                total,
                label: at.display_path(&source.relative),
                found: entries.len(),
            });
        }
    }

    // Once, not per scope: the CommandStore exists in HKLM only.
    if with_command_store {
        let source = paths::command_store_source();
        let at = paths::command_store_location();
        done += 1;
        if let Ok(key) = paths::root_key(at.scope).open(at.key_path(&source.relative)) {
            collect(&key, &source, at, &mut entries);
        }
        progress(ScanProgress {
            done,
            total,
            label: at.display_path(&source.relative),
            found: entries.len(),
        });
    }

    // Name resolution runs once over the finished tree rather than inside the
    // read path. That keeps reading and interpreting separable — and the two
    // resolvers share their caches across the whole scan, which is where the
    // saving is: a full scan hits the same shell32 references over and over.
    let mut mui = MuiResolver::new();
    let mut clsids = ClsidResolver::new();
    resolve_entries(&mut entries, &mut mui, &mut clsids);

    attribute_to_file_types(&entries, &mut file_types);

    let (mui_cache_hits, mui_cache_misses) = mui.stats();
    ScanResult::new(
        entries,
        file_types,
        ScanStats {
            mui_cache_hits,
            mui_cache_misses,
            blocked_clsids: clsids.blocked_count(),
        },
    )
}

/// Works out which entries belong to which file type.
///
/// Driven by the category an entry already carries, rather than by
/// remembering which slice of the location list produced it: the category is
/// the durable fact, the ordering is an implementation detail.
///
/// One entry can belong to several types — a `SystemFileAssociations\image`
/// entry is shared by every image extension — which is exactly right and the
/// reason this is not a partition.
fn attribute_to_file_types(entries: &[ContextEntry], file_types: &mut [FileTypeInfo]) {
    for (index, entry) in entries.iter().enumerate() {
        for info in file_types.iter_mut() {
            let ext = &info.resolution.ext;
            let belongs = match &entry.category {
                Category::ExtAssoc(other) | Category::ExtDirect(other) => other == ext,
                // Matched on membership, not on `from_ext`. A ProgID is
                // scanned once even when several extensions list it — the
                // Store PDF viewer is registered for both .jpg and .pdf —
                // and `from_ext` then only records whichever extension asked
                // first. Attributing by that would silently drop the entry
                // from every other type that offers it.
                Category::ProgId { prog_id, .. } => info
                    .resolution
                    .all_progids()
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(prog_id)),
                Category::PerceivedType(perceived) => {
                    info.resolution.perceived_type.as_deref() == Some(perceived.as_str())
                }
                // Levels 1 and 2 apply to every file and are counted
                // separately, so they must not inflate a single type.
                _ => false,
            };
            if belongs {
                info.entry_indices.push(index);
            }
        }
    }
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
                // Resolve the reference first, then take the accelerator out:
                // the `&` usually arrives from the string table, not from the
                // registry value, so doing it the other way round would miss
                // every system entry.
                display: entry
                    .raw_display
                    .as_deref()
                    .map(|raw| mui::strip_accelerator(&mui.resolve(raw))),
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
                // Only a quarter of all entries carry an `Icon` value, and a
                // menu of mostly blank squares tells nobody anything. Windows
                // itself falls back to the icon of the program the command
                // names, so this does too — measured on this machine: 202 of
                // 763 rows had a picture before, 654 after.
                if entry.icon_ref.is_none() {
                    entry.icon_ref = program_key.as_deref().map(fallback_icon);
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
                // A COM handler never has an `Icon` value at all — the menu
                // text and picture are produced at run time. Its DLL is the
                // next best thing, and two thirds of them do carry an icon.
                if entry.icon_ref.is_none() {
                    entry.icon_ref = info.program_key.as_deref().map(fallback_icon);
                }

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

/// Turns a program path into an icon reference.
///
/// The explicit `,0` matters: an icon reference is split at its **last**
/// comma, so a program whose name happens to contain one — `C:\\Werkzeug,
/// Version 2\\tool.exe` — would otherwise lose everything after it and be
/// read as an index. Appending the index removes the ambiguity for every path.
fn fallback_icon(program_key: &str) -> String {
    format!("{program_key},0")
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
pub(crate) fn subkey_names(key: &Key) -> Vec<String> {
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

fn collect(key: &Key, source: &CategorySource, at: Location, out: &mut Vec<ContextEntry>) {
    for name in subkey_names(key) {
        // `paths::join`, not `format!`: the CommandStore is scanned at its own
        // root, so its source path is empty, and a plain join produced
        // `…\CommandStore\shell\\Verb` — a doubled separator that reached both
        // the detail pane and the id.
        let relative = paths::join(&source.relative, &name);
        let entry = match source.kind {
            SourceKind::Shell => read_verb(key, &name, &relative, at, &source.category, 0),
            SourceKind::ShellEx => read_shellex(key, &name, &relative, at, &source.category),
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
    at: Location,
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

    let mut sub_commands = if depth < MAX_SUBMENU_DEPTH {
        read_sub_commands(&key, relative, at, category, depth + 1)
    } else {
        Vec::new()
    };

    // The other kind of cascading menu: a `SubCommands` value naming verbs
    // that live in the CommandStore. An empty value is not that — it is the
    // marker that says "this is a submenu, the children are in my own `shell`
    // subkey", which the branch above already read. Measured on this machine:
    // 15 entries carry `SubCommands` and every single one of them is empty, so
    // this path is exercised by tests rather than by hardware (ToDo 5.5).
    if sub_commands.is_empty()
        && depth < MAX_SUBMENU_DEPTH
        && let Some(list) = non_empty(key.get_string("SubCommands").ok())
    {
        sub_commands = read_store_commands(&list, category, depth + 1);
    }

    let registry_path = at.display_path(relative);

    Some(ContextEntry {
        id: stable_id(at.scope, &registry_path),
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
        scope: at.scope,
        category: category.clone(),
        registry_path,
        read_only: !at.writable_at_all() || !is_writable(parent, name),
        program_key: None,
    })
}

/// Resolves a `SubCommands` verb list against the CommandStore.
///
/// The value is semicolon-separated and names verbs by key name, e.g.
/// `Windows.Rotate90;Windows.Rotate270`. A name that is not in the store is
/// skipped rather than shown as an empty row: Windows leaves it out of the
/// menu too, and inventing a row for it would misreport what the menu holds.
///
/// The children keep the CommandStore's own registry path, because that is
/// where they are — showing them under the parent would name a key that does
/// not exist and hand the delete path something it must never touch.
fn read_store_commands(list: &str, category: &Category, depth: usize) -> Vec<ContextEntry> {
    let names: Vec<&str> = list
        .split(';')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    if names.is_empty() {
        return Vec::new();
    }

    let at = paths::command_store_location();
    let Ok(store) = paths::root_key(at.scope).open(at.key_path("")) else {
        return Vec::new();
    };

    names
        .iter()
        .filter_map(|name| read_verb(&store, name, name, at, category, depth))
        .collect()
}

/// Follows a cascading menu: children live under `<verb>\shell\<child>`.
fn read_sub_commands(
    key: &Key,
    relative: &str,
    at: Location,
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
            read_verb(&shell, &name, &child_relative, at, category, depth)
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
    at: Location,
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

    let registry_path = at.display_path(relative);

    Some(ContextEntry {
        id: stable_id(at.scope, &registry_path),
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
        scope: at.scope,
        category: category.clone(),
        registry_path,
        read_only: !at.writable_at_all() || !is_writable(parent, name),
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
    fn the_window_walks_the_users_own_extensions_as_well() {
        // What the window asks for when it starts. Before 2026-08-15 this was
        // the curated list and nothing else, and the extensions a user had
        // added were saved, shown in no tree and scanned by nobody.
        let curated = super::super::filetypes::CURATED.len();

        let plain = ScanOptions::with_file_types(&[]);
        assert_eq!(plain.file_types.len(), curated);

        let mine = ScanOptions::with_file_types(&[".ctxmenu_probe".into()]);
        assert_eq!(mine.file_types.len(), curated + 1);
        assert!(mine.file_types.iter().any(|e| e == ".ctxmenu_probe"));

        // And the full sweep is a different, larger set — the point of having
        // it at all.
        let every = ScanOptions::with_every_installed_file_type();
        assert!(
            every.file_types.len() > plain.file_types.len(),
            "every installed type: {}, curated: {}",
            every.file_types.len(),
            plain.file_types.len()
        );
    }

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

    /// A ProgID shared by two extensions must appear under both.
    ///
    /// Locations are scanned once, so the entry exists a single time; without
    /// membership-based attribution the second extension would silently lose
    /// it. Regression guard for exactly that.
    #[test]
    fn a_shared_progid_is_attributed_to_every_extension_that_lists_it() {
        use crate::registry::filetypes;

        // Find two curated extensions that genuinely share a ProgID on this
        // machine; without such a pair there is nothing to check here.
        let curated: Vec<(String, Vec<String>)> = filetypes::CURATED
            .iter()
            .take(40)
            .map(|d| {
                let r = filetypes::resolve(d.ext);
                (d.ext.to_string(), r.all_progids())
            })
            .filter(|(_, ids)| !ids.is_empty())
            .collect();

        let Some((a, b, shared)) = curated.iter().enumerate().find_map(|(i, (ext_a, ids_a))| {
            curated[i + 1..].iter().find_map(|(ext_b, ids_b)| {
                ids_a
                    .iter()
                    .find(|id| ids_b.contains(id))
                    .map(|id| (ext_a.clone(), ext_b.clone(), id.clone()))
            })
        }) else {
            return;
        };

        let options = ScanOptions {
            file_types: vec![a.clone(), b.clone()],
            ..ScanOptions::default()
        };
        let result = scan(&options, |_| {});

        for ext in [&a, &b] {
            let info = result
                .file_types
                .iter()
                .find(|f| f.ext() == ext)
                .expect("both types scanned");

            let has_shared = info.entry_indices.iter().any(|&i| {
                matches!(&result.entries[i].category,
                    Category::ProgId { prog_id, .. } if prog_id.eq_ignore_ascii_case(&shared))
            });
            let shared_exists = result.entries.iter().any(|e| {
                matches!(&e.category,
                    Category::ProgId { prog_id, .. } if prog_id.eq_ignore_ascii_case(&shared))
            });

            assert_eq!(
                has_shared, shared_exists,
                "{ext} must see the shared ProgID {shared} if it produced any entries"
            );
        }
    }

    /// Reads a location that exists on every Windows installation. Guards
    /// against the scanner silently returning nothing.
    #[test]
    fn scanning_directory_finds_entries_on_this_machine() {
        let options = ScanOptions {
            scopes: Scope::ALL.to_vec(),
            categories: Some(vec![Category::Directory]),
            ..ScanOptions::default()
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
