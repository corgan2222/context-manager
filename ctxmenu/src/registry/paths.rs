//! Maps categories to concrete registry locations.
//!
//! Everything that knows *where* something lives belongs here, so the scanner
//! stays a loop over a table instead of a pile of string literals.

use windows_registry::{CURRENT_USER, Key, LOCAL_MACHINE};

use crate::model::{Category, Scope};

/// Whether a location holds static verbs or COM handler registrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// `…\shell` — verbs with display text and command line in the registry.
    Shell,
    /// `…\shellex\ContextMenuHandlers` — CLSIDs of COM handlers.
    ShellEx,
}

/// One place to look, relative to a scope's classes root.
///
/// `relative` is owned rather than `&'static str`: the file type chain builds
/// its locations at runtime from the extension and its ProgIDs, and leaking a
/// string per location on every rescan would be a slow memory leak.
#[derive(Debug, Clone)]
pub struct CategorySource {
    pub category: Category,
    /// Path below `…\Classes`, e.g. `Directory\shell`.
    pub relative: String,
    pub kind: SourceKind,
}

/// The base categories from ToDo section 5.1.
///
/// Deviation from that table, deliberately: it lists only `shell` for
/// `AllFilesystemObjects`, `Folder` and `Drive`, but all three carry
/// `shellex\ContextMenuHandlers` subkeys in practice — 7-Zip registers under
/// `Folder\shellex` on this machine. Scanning a superset costs one failed
/// key open when a location is absent.
pub fn base_sources() -> Vec<CategorySource> {
    use Category::*;
    use SourceKind::*;

    let table: &[(Category, &'static str, SourceKind)] = &[
        (AllFiles, r"*\shell", Shell),
        (AllFiles, r"*\shellex\ContextMenuHandlers", ShellEx),
        (AllFilesystemObjects, r"AllFilesystemObjects\shell", Shell),
        (
            AllFilesystemObjects,
            r"AllFilesystemObjects\shellex\ContextMenuHandlers",
            ShellEx,
        ),
        (Directory, r"Directory\shell", Shell),
        (Directory, r"Directory\shellex\ContextMenuHandlers", ShellEx),
        (DirectoryBackground, r"Directory\Background\shell", Shell),
        (
            DirectoryBackground,
            r"Directory\Background\shellex\ContextMenuHandlers",
            ShellEx,
        ),
        (Folder, r"Folder\shell", Shell),
        (Folder, r"Folder\shellex\ContextMenuHandlers", ShellEx),
        (DesktopBackground, r"DesktopBackground\Shell", Shell),
        (
            DesktopBackground,
            r"DesktopBackground\ShellEx\ContextMenuHandlers",
            ShellEx,
        ),
        (Drive, r"Drive\shell", Shell),
        (Drive, r"Drive\shellex\ContextMenuHandlers", ShellEx),
    ];

    table
        .iter()
        .map(|(category, relative, kind)| CategorySource {
            category: category.clone(),
            relative: (*relative).to_string(),
            kind: *kind,
        })
        .collect()
}

/// The predefined key a scope hangs off.
pub fn root_key(scope: Scope) -> &'static Key {
    match scope {
        Scope::User => CURRENT_USER,
        Scope::Machine | Scope::Machine32 => LOCAL_MACHINE,
    }
}

/// Path relative to the predefined key — what `Key::open` wants.
pub fn key_path(scope: Scope, relative: &str) -> String {
    format!("{}\\{}", scope.classes_path(), relative)
}

/// Full path in `reg.exe` notation — what backup, restore and the UI want.
pub fn display_path(scope: Scope, relative: &str) -> String {
    format!("{}\\{}\\{}", scope.hive(), scope.classes_path(), relative)
}

/// A validated writable location.
///
/// Constructing one is the only way to name a key for backup or deletion, and
/// it can only be constructed for a path below a classes root that points at
/// an individual entry. Locations this tool must never touch — the
/// `CommandStore`, anything outside `…\Classes`, or a container key such as
/// `Directory\shell` itself — cannot be expressed, so they need no separate
/// check further down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegTarget {
    pub scope: Scope,
    /// Path below the classes root, e.g. `Directory\shell\cmd`.
    pub relative: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TargetError {
    #[error("kein bekannter Classes-Pfad / not a known classes path: {0}")]
    NotAClassesPath(String),
    #[error("Sammelschlüssel, kein einzelner Eintrag / container key, not an entry: {0}")]
    ContainerKey(String),
}

/// Key names that hold other entries. Deleting one of these would remove every
/// entry underneath it, which is never what a single delete should do.
const CONTAINER_KEYS: [&str; 6] = [
    "shell",
    "shellex",
    "contextmenuhandlers",
    "command",
    "background",
    "classes",
];

impl RegTarget {
    /// Parses a full path in `reg.exe` notation, rejecting anything unsafe.
    pub fn parse(full: &str) -> Result<Self, TargetError> {
        let normalised = full.trim().trim_end_matches('\\');
        let lowered = normalised.to_lowercase();

        // Registry paths are case-insensitive, so the prefix match has to be
        // too — otherwise a hand-typed path is rejected for its capitalisation.
        // Longest prefix first: the 32-bit root also starts with "HKLM\".
        let candidates = [Scope::Machine32, Scope::Machine, Scope::User];

        let (scope, relative) = candidates
            .iter()
            .find_map(|&scope| {
                let prefix = format!("{}\\{}\\", scope.hive(), scope.classes_path()).to_lowercase();
                lowered
                    .starts_with(&prefix)
                    .then(|| normalised[prefix.len()..].to_string())
                    .filter(|rest| !rest.is_empty())
                    .map(|rest| (scope, rest))
            })
            .ok_or_else(|| TargetError::NotAClassesPath(full.to_string()))?;

        let last = relative.rsplit('\\').next().unwrap_or_default();
        if CONTAINER_KEYS.contains(&last.to_lowercase().as_str()) {
            return Err(TargetError::ContainerKey(full.to_string()));
        }

        Ok(Self { scope, relative })
    }

    /// Path in `reg.exe` notation.
    pub fn full_path(&self) -> String {
        display_path(self.scope, &self.relative)
    }

    /// Path relative to the predefined key.
    pub fn key_path(&self) -> String {
        key_path(self.scope, &self.relative)
    }
}

/// Windows' own verbs. Owned by TrustedInstaller, read-only for this tool.
#[allow(dead_code)] // scanned from milestone 2, guarded against in milestone 3
pub const COMMAND_STORE: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell";

/// The blocked-CLSID list. One value here disables a handler everywhere,
/// which beats deleting the same handler under twenty classes.
#[allow(dead_code)] // used by the block action in milestone 9
pub const SHELL_EXTENSIONS_BLOCKED: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Blocked";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_base_category_has_at_least_one_source() {
        let sources = base_sources();
        for cat in Category::BASE {
            assert!(
                sources.iter().any(|s| s.category == cat),
                "no source for {cat:?}"
            );
        }
    }

    #[test]
    fn each_category_has_a_shell_and_a_shellex_source() {
        let sources = base_sources();
        for cat in Category::BASE {
            for kind in [SourceKind::Shell, SourceKind::ShellEx] {
                assert!(
                    sources.iter().any(|s| s.category == cat && s.kind == kind),
                    "{cat:?} is missing a {kind:?} source"
                );
            }
        }
    }

    /// Guards the load-bearing assumption of `Scope::Machine32`.
    ///
    /// Registry redirection applies to the *view*, not to the physical path,
    /// so a 64-bit process reaches the 32-bit classes root by plain path and
    /// needs no `KEY_WOW64_32KEY`. If that ever stopped holding, every
    /// `Machine32` scan would silently come back empty rather than fail — and
    /// an empty result is indistinguishable from a machine that simply has no
    /// 32-bit entries. Hence the assertion.
    ///
    /// `CLSID` is used as the probe because the context menu categories are
    /// genuinely absent from the 32-bit root on many machines, while `CLSID`
    /// is populated wherever a 32-bit program has ever been installed.
    #[test]
    fn the_32_bit_classes_root_is_reachable_by_plain_path() {
        let key = root_key(Scope::Machine32)
            .open(key_path(Scope::Machine32, "CLSID"))
            .expect(r"HKLM\SOFTWARE\WOW6432Node\Classes\CLSID must be readable");

        assert!(
            key.keys().expect("enumerable").next().is_some(),
            "the 32-bit CLSID root should not be empty"
        );
    }

    #[test]
    fn targets_round_trip_through_their_full_path() {
        for scope in Scope::ALL {
            let full = display_path(scope, r"Directory\shell\cmd");
            let target = RegTarget::parse(&full).expect("valid target");
            assert_eq!(target.scope, scope);
            assert_eq!(target.relative, r"Directory\shell\cmd");
            assert_eq!(target.full_path(), full);
        }
    }

    #[test]
    fn target_parsing_ignores_capitalisation() {
        let target = RegTarget::parse(r"hkcu\software\classes\Directory\shell\cmd")
            .expect("registry paths are case-insensitive");
        assert_eq!(target.scope, Scope::User);
        assert_eq!(target.relative, r"Directory\shell\cmd");
    }

    #[test]
    fn paths_outside_the_classes_roots_are_refused() {
        for path in [
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\Windows.take",
            r"HKLM\SYSTEM\CurrentControlSet\Services\Foo",
            r"HKCU\Software\Microsoft\Windows",
            r"HKCU\SOFTWARE\Classes",
            "",
        ] {
            assert!(
                matches!(
                    RegTarget::parse(path),
                    Err(TargetError::NotAClassesPath(_) | TargetError::ContainerKey(_))
                ),
                "should have been refused: {path}"
            );
        }
    }

    #[test]
    fn container_keys_cannot_be_targeted() {
        for path in [
            r"HKCU\SOFTWARE\Classes\Directory\shell",
            r"HKLM\SOFTWARE\Classes\Directory\Background",
            r"HKLM\SOFTWARE\Classes\*\shellex\ContextMenuHandlers",
            r"HKCU\SOFTWARE\Classes\Directory\shell\cmd\command",
        ] {
            assert_eq!(
                RegTarget::parse(path),
                Err(TargetError::ContainerKey(path.to_string())),
                "should have been refused as a container: {path}"
            );
        }
    }

    #[test]
    fn the_32_bit_scope_resolves_into_the_wow_node() {
        assert_eq!(
            key_path(Scope::Machine32, r"Directory\shell"),
            r"SOFTWARE\WOW6432Node\Classes\Directory\shell"
        );
        assert_eq!(
            display_path(Scope::Machine32, r"Directory\shell"),
            r"HKLM\SOFTWARE\WOW6432Node\Classes\Directory\shell"
        );
        assert_eq!(
            display_path(Scope::User, r"Directory\shell"),
            r"HKCU\SOFTWARE\Classes\Directory\shell"
        );
    }
}
