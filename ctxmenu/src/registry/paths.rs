//! Maps categories to concrete registry locations.
//!
//! Everything that knows *where* something lives belongs here, so the scanner
//! stays a loop over a table instead of a pile of string literals.

use windows_registry::{CURRENT_USER, Key, LOCAL_MACHINE};

use crate::model::{Category, Scope};

/// Whether a location holds static verbs or COM handler registrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// `…\shell` — verbs with display text and command line in the registry.
    Shell,
    /// `…\shellex\ContextMenuHandlers` — CLSIDs of COM handlers.
    ShellEx,
}

/// One place to look, relative to a scope's classes root.
#[derive(Debug, Clone)]
pub struct CategorySource {
    pub category: Category,
    /// Path below `…\Classes`, e.g. `Directory\shell`.
    pub relative: &'static str,
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
            relative,
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
