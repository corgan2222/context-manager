//! Data model shared by scanner, CLI and (later) GUI.
//!
//! Deliberately free of any `windows` dependency so it can be unit tested
//! without touching the registry. The mapping from these types to actual
//! registry locations lives in [`crate::registry::paths`].

use rustc_hash::FxHashMap;
use serde::Serialize;

/// Which registry hive an entry was found in.
///
/// `HKCR` is a merged view, not a hive of its own, so the scanner reads the
/// contributing hives separately. Without that split we could show neither
/// where an entry comes from nor whether it can be removed without elevation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
pub enum Scope {
    /// `HKCU\SOFTWARE\Classes` — writable without elevation.
    User,
    /// `HKLM\SOFTWARE\Classes` — system wide, needs elevation to change.
    Machine,
    /// `HKLM\SOFTWARE\WOW6432Node\Classes` — where 32-bit programs register.
    Machine32,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::User, Scope::Machine, Scope::Machine32];

    /// Short label used in output, IDs and backup manifests.
    pub fn label(self) -> &'static str {
        match self {
            Scope::User => "HKCU",
            Scope::Machine => "HKLM",
            Scope::Machine32 => "HKLM32",
        }
    }

    /// Hive prefix in the notation `reg.exe` expects.
    pub fn hive(self) -> &'static str {
        match self {
            Scope::User => "HKCU",
            Scope::Machine | Scope::Machine32 => "HKLM",
        }
    }

    /// Path of the classes root below the hive.
    ///
    /// The 32-bit view is reachable from a 64-bit process through the plain
    /// path — registry redirection applies to the *view*, not to the physical
    /// key — so no `KEY_WOW64_32KEY` is needed anywhere in this code base.
    pub fn classes_path(self) -> &'static str {
        match self {
            Scope::User | Scope::Machine => r"SOFTWARE\Classes",
            Scope::Machine32 => r"SOFTWARE\WOW6432Node\Classes",
        }
    }

    // Used by the CLI round-trip tests today and by settings persistence in
    // milestone 5; kept next to `from_slug` so the pair stays in sync.
    #[allow(dead_code)]
    pub fn slug(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Machine => "machine",
            Scope::Machine32 => "machine32",
        }
    }

    pub fn from_slug(s: &str) -> Option<Scope> {
        match s.to_ascii_lowercase().as_str() {
            "user" | "hkcu" => Some(Scope::User),
            "machine" | "hklm" => Some(Scope::Machine),
            "machine32" | "hklm32" | "wow64" => Some(Scope::Machine32),
            _ => None,
        }
    }
}

/// Where in the shell hierarchy an entry applies.
///
/// The file-type variants are unused until milestone 7 but exist already so
/// that adding the file-type view does not force a change to this model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
// The file-type variants are constructed from milestone 7 onwards.
#[allow(dead_code)]
pub enum Category {
    /// `*` — every file.
    AllFiles,
    /// `AllFilesystemObjects` — files and folders.
    AllFilesystemObjects,
    /// `Directory` — right-click on a folder.
    Directory,
    /// `Directory\Background` — right-click on empty space inside a folder.
    DirectoryBackground,
    /// `Folder` — folders plus shell namespace objects such as ZIP archives.
    Folder,
    /// `DesktopBackground` — right-click on the desktop.
    DesktopBackground,
    /// `Drive` — right-click on a drive.
    Drive,
    /// `SystemFileAssociations\<perceived type>`
    PerceivedType(String),
    /// `SystemFileAssociations\.<ext>`
    ExtAssoc(String),
    /// A ProgID's own `shell` branch, with the extension it was reached from.
    ProgId { prog_id: String, from_ext: String },
    /// `.<ext>\shell` — rare, but it exists.
    ExtDirect(String),
}

impl Category {
    /// The seven base categories scanned without any file-type resolution.
    pub const BASE: [Category; 7] = [
        Category::AllFiles,
        Category::AllFilesystemObjects,
        Category::Directory,
        Category::DirectoryBackground,
        Category::Folder,
        Category::DesktopBackground,
        Category::Drive,
    ];

    /// Stable lowercase identifier used on the command line.
    pub fn slug(&self) -> String {
        match self {
            Category::AllFiles => "allfiles".into(),
            Category::AllFilesystemObjects => "allfilesystemobjects".into(),
            Category::Directory => "directory".into(),
            Category::DirectoryBackground => "directorybackground".into(),
            Category::Folder => "folder".into(),
            Category::DesktopBackground => "desktopbackground".into(),
            Category::Drive => "drive".into(),
            Category::PerceivedType(t) => format!("perceived:{t}"),
            Category::ExtAssoc(e) => format!("extassoc:{e}"),
            Category::ProgId { prog_id, .. } => format!("progid:{prog_id}"),
            Category::ExtDirect(e) => format!("ext:{e}"),
        }
    }

    pub fn from_slug(s: &str) -> Option<Category> {
        let s = s.to_ascii_lowercase();
        Category::BASE.iter().find(|c| c.slug() == s).cloned()
    }
}

/// What kind of context menu entry this is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum EntryKind {
    /// A static verb: display text and command line live in the registry.
    Verb {
        command: Option<String>,
        /// Children of a cascading menu, one level per nesting step.
        sub_commands: Vec<ContextEntry>,
    },
    /// A COM handler. Its menu text is produced at runtime by
    /// `IContextMenu::QueryContextMenu` and is therefore *not* in the registry.
    ShellEx {
        clsid: String,
        server_path: Option<String>,
        blocked: bool,
    },
}

impl EntryKind {
    pub fn type_label(&self) -> &'static str {
        match self {
            EntryKind::Verb { .. } => "verb",
            EntryKind::ShellEx { .. } => "shellex",
        }
    }
}

/// One context menu entry as found in the registry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextEntry {
    /// Stable across runs: FNV-1a over scope and lowercased registry path.
    pub id: String,
    /// Name of the registry subkey the entry lives in.
    pub key_name: String,
    /// Resolved text to show. Falls back to `key_name` when nothing else fits.
    pub display_name: String,
    /// Unresolved `MUIVerb` or default value, kept for diagnostics.
    pub raw_display: Option<String>,
    pub icon_ref: Option<String>,
    /// `Top` or `Bottom`; the only ordering hint Windows honours reliably.
    pub position: Option<String>,
    /// `Extended` — only visible while Shift is held.
    pub extended: bool,
    /// `LegacyDisable` or `ProgrammaticAccessOnly` — hidden from the menu.
    pub hidden: bool,
    pub applies_to: Option<String>,
    pub kind: EntryKind,
    pub scope: Scope,
    pub category: Category,
    /// Full path in `reg.exe` notation — the input for backup and delete.
    pub registry_path: String,
    pub read_only: bool,
    /// Grouping key for the program view; filled in milestone 8.
    pub program_key: Option<String>,
}

/// Diagnostics from one scan pass.
///
/// Reported rather than kept internal because the caching claims in this code
/// base are supposed to be checkable, not assumed.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct ScanStats {
    pub mui_cache_hits: usize,
    pub mui_cache_misses: usize,
    /// Size of the machine-wide blocked-CLSID list.
    pub blocked_clsids: usize,
}

/// One file type and the entries its resolution chain contributed.
///
/// Levels 1 and 2 of the chain apply to every file and are held once in
/// `ScanResult::entries` under their base categories; `entry_indices` covers
/// only levels 3 to 7, which are specific to this extension (ToDo 10.4).
#[derive(Debug, Clone, Serialize)]
pub struct FileTypeInfo {
    pub group: crate::registry::filetypes::TypeGroup,
    pub resolution: crate::registry::filetypes::Resolution,
    pub entry_indices: Vec<usize>,
}

impl FileTypeInfo {
    pub fn ext(&self) -> &str {
        &self.resolution.ext
    }

    pub fn own_entry_count(&self) -> usize {
        self.entry_indices.len()
    }
}

/// Result of one scan pass.
#[derive(Debug, Serialize)]
// The index maps are read by the GUI from milestone 4 onwards.
#[allow(dead_code)]
pub struct ScanResult {
    pub entries: Vec<ContextEntry>,
    /// Indices into `entries`. Skipped when serialising because JSON object
    /// keys must be strings and `Category` is an enum.
    #[serde(skip)]
    pub by_category: FxHashMap<Category, Vec<usize>>,
    #[serde(skip)]
    pub by_program: FxHashMap<String, Vec<usize>>,
    pub scanned_at: chrono::DateTime<chrono::Local>,
    pub stats: ScanStats,
    /// Empty unless the scan was asked to walk file types.
    pub file_types: Vec<FileTypeInfo>,
}

impl ScanResult {
    pub fn new(
        entries: Vec<ContextEntry>,
        file_types: Vec<FileTypeInfo>,
        stats: ScanStats,
    ) -> Self {
        let mut by_category: FxHashMap<Category, Vec<usize>> = FxHashMap::default();
        let mut by_program: FxHashMap<String, Vec<usize>> = FxHashMap::default();

        for (i, e) in entries.iter().enumerate() {
            by_category.entry(e.category.clone()).or_default().push(i);
            if let Some(key) = &e.program_key {
                by_program.entry(key.clone()).or_default().push(i);
            }
        }

        Self {
            entries,
            by_category,
            by_program,
            scanned_at: chrono::Local::now(),
            stats,
            file_types,
        }
    }
}

/// Progress report emitted while scanning.
///
/// The CLI prints these; milestone 4 sends them down an mpsc channel to the
/// GUI so the list fills visibly instead of appearing all at once.
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub done: usize,
    pub total: usize,
    pub label: String,
    pub found: usize,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Builds the entry ID.
///
/// FNV-1a rather than `FxHash`: the ID ends up in the selection state and in
/// backup manifests, so it has to survive restarts and toolchain updates, and
/// `FxHash` promises no stability across versions. Registry paths are
/// case-insensitive, hence the lowercasing.
pub fn stable_id(scope: Scope, registry_path: &str) -> String {
    let mut hash = FNV_OFFSET;
    hash = fnv1a(scope.label().as_bytes(), hash);
    hash = fnv1a(b"|", hash);
    hash = fnv1a(registry_path.to_lowercase().as_bytes(), hash);
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable_and_case_insensitive() {
        let a = stable_id(Scope::Machine, r"HKLM\SOFTWARE\Classes\Directory\shell\cmd");
        let b = stable_id(Scope::Machine, r"hklm\software\classes\directory\shell\CMD");
        assert_eq!(a, b, "registry paths are case-insensitive");
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn id_separates_scopes_and_paths() {
        let path = r"HKLM\SOFTWARE\Classes\Directory\shell\cmd";
        assert_ne!(
            stable_id(Scope::Machine, path),
            stable_id(Scope::Machine32, path)
        );
        assert_ne!(
            stable_id(Scope::Machine, path),
            stable_id(Scope::Machine, &format!("{path}2"))
        );
    }

    #[test]
    fn base_categories_round_trip_through_slugs() {
        for cat in Category::BASE {
            let slug = cat.slug();
            assert_eq!(Category::from_slug(&slug), Some(cat.clone()), "slug {slug}");
        }
        assert_eq!(Category::from_slug("nonsense"), None);
    }

    #[test]
    fn scopes_round_trip_through_slugs() {
        for scope in Scope::ALL {
            assert_eq!(Scope::from_slug(scope.slug()), Some(scope));
        }
        assert_eq!(Scope::from_slug("HKLM"), Some(Scope::Machine));
        assert_eq!(Scope::from_slug("nonsense"), None);
    }

    #[test]
    fn only_the_32_bit_scope_uses_the_wow_node() {
        assert_eq!(
            Scope::Machine32.classes_path(),
            r"SOFTWARE\WOW6432Node\Classes"
        );
        assert_eq!(Scope::Machine.classes_path(), r"SOFTWARE\Classes");
        // Both machine scopes live in the same hive as far as reg.exe cares.
        assert_eq!(Scope::Machine32.hive(), "HKLM");
    }
}
