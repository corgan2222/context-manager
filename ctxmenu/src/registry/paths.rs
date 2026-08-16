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

/// What a location's path is measured from.
///
/// Almost everything hangs off a scope's classes root, which is also the only
/// place this tool is allowed to write. The CommandStore does not: it lives
/// under `SOFTWARE\Microsoft\Windows\…`, and keeping that distinction in the
/// type is what stops a read-only location from ever being handed to the
/// delete path by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// Below `…\Classes`.
    Classes,
    /// Below the hive itself, with this prefix in front of `relative`.
    Hive(&'static str),
}

/// One place to look, with the anchor its path is measured from.
///
/// `relative` is owned rather than `&'static str`: the file type chain builds
/// its locations at runtime from the extension and its ProgIDs, and leaking a
/// string per location on every rescan would be a slow memory leak.
#[derive(Debug, Clone)]
pub struct CategorySource {
    pub category: Category,
    /// Path below the anchor, e.g. `Directory\shell`.
    pub relative: String,
    pub kind: SourceKind,
    pub anchor: Anchor,
}

/// Scope and anchor together — everything a path needs besides its tail.
///
/// Carried through the scanner instead of a bare `Scope`, so that reading a
/// verb out of the CommandStore builds the CommandStore's own path rather than
/// silently claiming the entry lives under `…\Classes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub scope: Scope,
    pub anchor: Anchor,
}

impl Location {
    pub fn classes(scope: Scope) -> Self {
        Self {
            scope,
            anchor: Anchor::Classes,
        }
    }

    /// What comes between the hive and `relative`.
    fn base(&self) -> &str {
        match self.anchor {
            Anchor::Classes => self.scope.classes_path(),
            Anchor::Hive(prefix) => prefix,
        }
    }

    /// Path relative to the predefined key — what `Key::open` wants.
    pub fn key_path(&self, relative: &str) -> String {
        join(self.base(), relative)
    }

    /// Full path in `reg.exe` notation — what backup, restore and the UI want.
    pub fn display_path(&self, relative: &str) -> String {
        join(&join(self.scope.hive(), self.base()), relative)
    }

    /// Can anything here be changed at all?
    ///
    /// `false` for the CommandStore, whatever the key's own permissions say:
    /// those verbs belong to Windows, and a machine where they happen to be
    /// writable is not a reason to offer it (ToDo 5.5).
    pub fn writable_at_all(&self) -> bool {
        matches!(self.anchor, Anchor::Classes)
    }
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
            anchor: Anchor::Classes,
        })
        .collect()
}

/// Every place this tool ever reads or writes, for a backup of the lot.
///
/// Containers, not individual entries: `reg.exe` exports a key with everything
/// below it in one call, so fourteen locations per scope cover what nine
/// hundred single exports would — and a container also captures the entries
/// that were added *after* the last scan.
///
/// `SystemFileAssociations` is taken whole rather than per extension. It is
/// one key with a few hundred children on this machine, and enumerating the
/// 1674 registered extensions to name them individually would produce a
/// slower backup of the same data.
///
/// The blocked-CLSID list is included although it lives outside the classes
/// tree: it is the one thing this program writes that no category covers.
pub fn full_backup_paths() -> Vec<String> {
    let mut out = Vec::new();

    for scope in Scope::ALL {
        let at = Location::classes(scope);
        for source in base_sources() {
            out.push(at.display_path(&source.relative));
        }
        out.push(at.display_path("SystemFileAssociations"));
    }

    out.push(blocked_list_display_path());
    out
}

/// Windows' own stock of verbs, as a location to scan.
///
/// Not one of the base categories and never scanned per scope: the
/// CommandStore exists once, in HKLM. The entries it holds appear in no menu
/// by themselves — a `SubCommands` value elsewhere has to name them — which is
/// why they are shown as their own category rather than mixed in with entries
/// that really are on a menu.
pub fn command_store_source() -> CategorySource {
    CategorySource {
        category: Category::CommandStore,
        relative: String::new(),
        kind: SourceKind::Shell,
        anchor: Anchor::Hive(COMMAND_STORE),
    }
}

/// Where the CommandStore hangs, for resolving a `SubCommands` verb list.
pub fn command_store_location() -> Location {
    Location {
        scope: Scope::Machine,
        anchor: Anchor::Hive(COMMAND_STORE),
    }
}

/// Joins two path parts, tolerating an empty one.
///
/// The CommandStore is scanned at its own root, so its `relative` is empty and
/// a plain `format!` would produce `…\CommandStore\shell\\Verb` — a path that
/// `reg.exe` rejects and that would land in every backup manifest.
pub fn join(head: &str, tail: &str) -> String {
    match (head.is_empty(), tail.is_empty()) {
        (true, _) => tail.to_string(),
        (_, true) => head.to_string(),
        _ => format!("{head}\\{tail}"),
    }
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
///
/// That claim used to be false, and the fields being `pub` was why: anything
/// could assemble the struct and skip every check the constructors make. They
/// are private now, and the three ways in — [`parse`](Self::parse),
/// [`below_classes`](Self::below_classes) and deserialisation — all run the
/// same validation.
///
/// **Deserialisation matters most.** A plan travels to the elevated half of
/// this program as JSON in a temp file, and that half runs as administrator.
/// `try_from` means a hand-edited job file cannot name a path that the
/// unelevated side would have refused.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "UncheckedTarget")]
pub struct RegTarget {
    scope: Scope,
    /// Path below the classes root, e.g. `Directory\shell\cmd`.
    relative: String,
}

/// The shape `RegTarget` has on disk, before it is checked.
///
/// Exists only so `serde` has something to deserialise into that is allowed to
/// be invalid; the `TryFrom` below is where it becomes a `RegTarget`.
#[derive(serde::Deserialize)]
struct UncheckedTarget {
    scope: Scope,
    relative: String,
}

impl TryFrom<UncheckedTarget> for RegTarget {
    type Error = TargetError;

    fn try_from(raw: UncheckedTarget) -> Result<Self, Self::Error> {
        Self::below_classes(raw.scope, &raw.relative)
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TargetError {
    #[error("\x1ekein bekannter Classes-Pfad\x1fnot a known classes path\x1d: {0}")]
    NotAClassesPath(String),
    #[error("Sammelschlüssel, \x1ekein einzelner Eintrag\x1fcontainer key\x1d, not an entry: {0}")]
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

        Self::below_classes(scope, &relative).map_err(|error| match error {
            // Report the path the caller passed in, not the tail of it.
            TargetError::ContainerKey(_) => TargetError::ContainerKey(full.to_string()),
            other => other,
        })
    }

    /// Builds a target from a scope and a path below the classes root.
    ///
    /// The other way in, for callers that never had a full path to begin with
    /// — the editor knows the category and the key name, and composing a
    /// string only to parse it back would be a detour with a chance of error.
    ///
    /// Runs exactly the checks `parse` runs, because they are the same checks:
    /// an empty path names the classes root itself, and a path ending in a
    /// container key names a collection rather than an entry. Both would turn
    /// a single delete into a sweep.
    pub fn below_classes(scope: Scope, relative: &str) -> Result<Self, TargetError> {
        let relative = relative.trim().trim_matches('\\').to_string();
        if relative.is_empty() {
            return Err(TargetError::NotAClassesPath(format!(
                "{}\\{}",
                scope.hive(),
                scope.classes_path()
            )));
        }

        let last = relative.rsplit('\\').next().unwrap_or_default();
        if CONTAINER_KEYS.contains(&last.to_lowercase().as_str()) {
            return Err(TargetError::ContainerKey(relative));
        }

        Ok(Self { scope, relative })
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// Path below the classes root.
    pub fn relative(&self) -> &str {
        &self.relative
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
pub const SHELL_EXTENSIONS_BLOCKED: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Blocked";

/// The blocked list in `reg.exe` notation, for backup and restore.
///
/// It sits outside the classes tree, so it cannot be a [`RegTarget`] — which
/// is deliberate: nothing that walks entries should be able to reach it by
/// accident.
pub fn blocked_list_display_path() -> String {
    format!("HKLM\\{SHELL_EXTENSIONS_BLOCKED}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_backup_covers_every_place_this_tool_touches() {
        let paths = full_backup_paths();

        // Every base location, in every scope, plus the file type branch.
        assert_eq!(
            paths.len(),
            (base_sources().len() + 1) * Scope::ALL.len() + 1,
            "one path per source and scope, plus SystemFileAssociations and \
             the blocked list"
        );

        let unique: std::collections::HashSet<&String> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len(), "a key exported twice is wasted");

        for path in &paths {
            assert!(
                path.starts_with("HKCU\\") || path.starts_with("HKLM\\"),
                "{path} is not in reg.exe notation"
            );
        }

        // The three that would be missed by taking only what a scan returned.
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with(r"Classes\Directory\shell"))
        );
        assert!(paths.iter().any(|p| p.ends_with("SystemFileAssociations")));
        assert!(paths.contains(&blocked_list_display_path()));

        // The 32-bit view really is a third root and not a repeat of the first.
        assert!(paths.iter().any(|p| p.contains("WOW6432Node")));
    }

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

    #[test]
    fn a_job_file_cannot_smuggle_a_path_past_the_checks() {
        // The load-bearing one. A plan travels to the elevated half of this
        // program as JSON in a temp file, and that half runs as administrator.
        // Before `try_from`, hand-editing that file was enough to name a
        // container key — and have it deleted with full rights.
        let sweep = r#"{"scope":"User","relative":"Directory\\shell"}"#;
        let error = serde_json::from_str::<RegTarget>(sweep)
            .expect_err("a container key must not deserialise");
        assert!(
            error.to_string().contains("Sammelschlüssel"),
            "unexpected error: {error}"
        );

        assert!(
            serde_json::from_str::<RegTarget>(r#"{"scope":"User","relative":""}"#).is_err(),
            "the classes root itself must not deserialise"
        );

        // What a legitimate job file carries still goes through.
        let fine: RegTarget =
            serde_json::from_str(r#"{"scope":"User","relative":"Directory\\shell\\cmd"}"#)
                .expect("an ordinary entry must survive the round trip");
        assert_eq!(fine.relative(), r"Directory\shell\cmd");
    }

    #[test]
    fn a_target_survives_its_own_serialisation() {
        // The elevation hand-off depends on this: what the parent writes has
        // to be what the child reads.
        let original = RegTarget::below_classes(Scope::Machine, r"Directory\shell\cmd")
            .expect("names an entry");
        let json = serde_json::to_string(&original).expect("serialises");
        let back: RegTarget = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(original, back);
    }

    #[test]
    fn the_checking_constructor_refuses_what_parse_refuses() {
        // Two ways in, one set of rules. They used to be one way in and one
        // way around.
        for tail in [
            "shell",
            "SHELL",
            "command",
            "shellex",
            "ContextMenuHandlers",
        ] {
            let relative = format!(r"Directory\{tail}");
            assert!(
                matches!(
                    RegTarget::below_classes(Scope::User, &relative),
                    Err(TargetError::ContainerKey(_))
                ),
                "{relative} should be refused as a container"
            );
        }

        assert!(RegTarget::below_classes(Scope::User, "  ").is_err());
        assert!(RegTarget::below_classes(Scope::User, "\\").is_err());

        // A nested child is a single entry and stays allowed: that is the one
        // thing standing between here and editing submenu children.
        assert!(RegTarget::below_classes(Scope::User, r"Directory\shell\a\shell\b").is_ok());
    }

    #[test]
    fn the_command_store_builds_its_own_path_not_a_classes_one() {
        let at = command_store_location();
        let path = at.display_path("Windows.Rotate90");

        assert_eq!(
            path,
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\Windows.Rotate90"
        );
        assert!(
            !path.contains(r"\\"),
            "an empty source path must not double the separator: {path}"
        );
        assert!(
            !path.to_lowercase().contains("classes"),
            "the store is not below Classes: {path}"
        );
    }

    #[test]
    fn nothing_in_the_command_store_can_be_named_as_a_target() {
        // The safety net behind the read-only flag: even if a path from the
        // store reached the delete path, it cannot be turned into a target.
        let at = command_store_location();
        let path = at.display_path("Windows.Rotate90");
        assert!(matches!(
            RegTarget::parse(&path),
            Err(TargetError::NotAClassesPath(_))
        ));

        assert!(!at.writable_at_all());
        assert!(Location::classes(Scope::User).writable_at_all());
    }

    #[test]
    fn joining_tolerates_an_empty_part_on_either_side() {
        assert_eq!(join("a", "b"), r"a\b");
        assert_eq!(join("", "b"), "b");
        assert_eq!(join("a", ""), "a");
        assert_eq!(join("", ""), "");
    }

    #[test]
    fn a_classes_location_still_builds_what_it_always_did() {
        // The refactor to `Location` must not have moved anything: these are
        // the paths backup, restore and every manifest already contain.
        let at = Location::classes(Scope::Machine);
        assert_eq!(
            at.display_path(r"Directory\shell\cmd"),
            display_path(Scope::Machine, r"Directory\shell\cmd")
        );
        assert_eq!(
            at.key_path(r"Directory\shell\cmd"),
            key_path(Scope::Machine, r"Directory\shell\cmd")
        );
        assert_eq!(
            Location::classes(Scope::Machine32).display_path("x"),
            r"HKLM\SOFTWARE\WOW6432Node\Classes\x"
        );
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
