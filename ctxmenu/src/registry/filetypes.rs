//! File types and the chain Windows walks to build their context menu.
//!
//! Right-clicking a `.jpg` shows entries drawn from at least seven different
//! registry branches. Two of them — the perceived type and the
//! extension-specific `SystemFileAssociations` key — are where image viewers,
//! converters and photo tools register, and they are exactly the two most
//! tools overlook.

use serde::{Deserialize, Serialize};
use windows_registry::CURRENT_USER;

use super::paths::{self, Anchor, CategorySource, SourceKind};
use crate::model::{Category, Scope};

/// Grouping for the tree on the left of the file type tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TypeGroup {
    Documents,
    Images,
    Raw,
    Audio,
    Video,
    Archives,
    Code,
    System,
    /// Added by the user, or found by scanning every installed extension.
    Other,
}

impl TypeGroup {
    pub const ALL: [TypeGroup; 9] = [
        TypeGroup::Documents,
        TypeGroup::Images,
        TypeGroup::Raw,
        TypeGroup::Audio,
        TypeGroup::Video,
        TypeGroup::Archives,
        TypeGroup::Code,
        TypeGroup::System,
        TypeGroup::Other,
    ];
}

/// One curated file type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTypeDef {
    /// Always lowercase and always with the leading dot.
    pub ext: &'static str,
    pub group: TypeGroup,
}

/// The starting point, not the limit.
///
/// A machine typically carries 400 to 900 registered extensions; showing all
/// of them would bury the interesting ones. The user can add extensions by
/// hand, and milestone 7 offers a full scan on request.
pub const CURATED: &[FileTypeDef] = &[
    // Dokumente
    def(".pdf", TypeGroup::Documents),
    def(".doc", TypeGroup::Documents),
    def(".docx", TypeGroup::Documents),
    def(".xls", TypeGroup::Documents),
    def(".xlsx", TypeGroup::Documents),
    def(".ppt", TypeGroup::Documents),
    def(".pptx", TypeGroup::Documents),
    def(".txt", TypeGroup::Documents),
    def(".rtf", TypeGroup::Documents),
    def(".odt", TypeGroup::Documents),
    def(".ods", TypeGroup::Documents),
    def(".csv", TypeGroup::Documents),
    def(".md", TypeGroup::Documents),
    def(".epub", TypeGroup::Documents),
    // Bilder
    def(".jpg", TypeGroup::Images),
    def(".jpeg", TypeGroup::Images),
    def(".png", TypeGroup::Images),
    def(".gif", TypeGroup::Images),
    def(".bmp", TypeGroup::Images),
    def(".tif", TypeGroup::Images),
    def(".tiff", TypeGroup::Images),
    def(".webp", TypeGroup::Images),
    def(".heic", TypeGroup::Images),
    def(".svg", TypeGroup::Images),
    def(".ico", TypeGroup::Images),
    def(".psd", TypeGroup::Images),
    def(".xcf", TypeGroup::Images),
    // RAW
    def(".cr2", TypeGroup::Raw),
    def(".cr3", TypeGroup::Raw),
    def(".nef", TypeGroup::Raw),
    def(".arw", TypeGroup::Raw),
    def(".dng", TypeGroup::Raw),
    def(".raf", TypeGroup::Raw),
    def(".orf", TypeGroup::Raw),
    def(".rw2", TypeGroup::Raw),
    // Audio
    def(".mp3", TypeGroup::Audio),
    def(".flac", TypeGroup::Audio),
    def(".wav", TypeGroup::Audio),
    def(".m4a", TypeGroup::Audio),
    def(".ogg", TypeGroup::Audio),
    def(".opus", TypeGroup::Audio),
    def(".aac", TypeGroup::Audio),
    def(".wma", TypeGroup::Audio),
    // Video
    def(".mp4", TypeGroup::Video),
    def(".mkv", TypeGroup::Video),
    def(".avi", TypeGroup::Video),
    def(".mov", TypeGroup::Video),
    def(".webm", TypeGroup::Video),
    def(".wmv", TypeGroup::Video),
    def(".flv", TypeGroup::Video),
    def(".m2ts", TypeGroup::Video),
    def(".mpg", TypeGroup::Video),
    // Archive
    def(".zip", TypeGroup::Archives),
    def(".rar", TypeGroup::Archives),
    def(".7z", TypeGroup::Archives),
    def(".tar", TypeGroup::Archives),
    def(".gz", TypeGroup::Archives),
    def(".bz2", TypeGroup::Archives),
    def(".xz", TypeGroup::Archives),
    def(".iso", TypeGroup::Archives),
    def(".cab", TypeGroup::Archives),
    // Code und Konfiguration
    def(".py", TypeGroup::Code),
    def(".rs", TypeGroup::Code),
    def(".go", TypeGroup::Code),
    def(".js", TypeGroup::Code),
    def(".ts", TypeGroup::Code),
    def(".jsx", TypeGroup::Code),
    def(".tsx", TypeGroup::Code),
    def(".c", TypeGroup::Code),
    def(".cpp", TypeGroup::Code),
    def(".h", TypeGroup::Code),
    def(".cs", TypeGroup::Code),
    def(".java", TypeGroup::Code),
    def(".rb", TypeGroup::Code),
    def(".php", TypeGroup::Code),
    def(".sql", TypeGroup::Code),
    def(".json", TypeGroup::Code),
    def(".yaml", TypeGroup::Code),
    def(".yml", TypeGroup::Code),
    def(".toml", TypeGroup::Code),
    def(".xml", TypeGroup::Code),
    def(".html", TypeGroup::Code),
    def(".css", TypeGroup::Code),
    def(".sh", TypeGroup::Code),
    def(".ps1", TypeGroup::Code),
    def(".bat", TypeGroup::Code),
    def(".cmd", TypeGroup::Code),
    def(".ini", TypeGroup::Code),
    def(".conf", TypeGroup::Code),
    def(".log", TypeGroup::Code),
    // System
    def(".exe", TypeGroup::System),
    def(".dll", TypeGroup::System),
    def(".msi", TypeGroup::System),
    def(".lnk", TypeGroup::System),
    def(".reg", TypeGroup::System),
    def(".sys", TypeGroup::System),
    def(".vhd", TypeGroup::System),
    def(".vhdx", TypeGroup::System),
];

const fn def(ext: &'static str, group: TypeGroup) -> FileTypeDef {
    FileTypeDef { ext, group }
}

/// Normalises user input into the form the registry uses.
///
/// Accepts `jpg`, `.JPG`, `*.jpg` and `  .jpg  ` alike, because all four are
/// what people type into an "add extension" field.
pub fn normalize_ext(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('*');
    let without_dot = trimmed.trim_start_matches('.').trim();

    if without_dot.is_empty() {
        return None;
    }
    // A registry key name cannot contain a backslash, and an extension with a
    // space is not one.
    if without_dot.contains(['\\', '/', ' ']) {
        return None;
    }

    Some(format!(".{}", without_dot.to_lowercase()))
}

/// Every file extension registered anywhere on this machine.
///
/// The subkeys with a leading dot below the three classes roots, normalised
/// and deduplicated — the full scan, the counterpart to the curated list.
///
/// Asked for rather than done at startup, and the numbers say why: measured on
/// this machine, 1304 such keys sit under `HKLM\SOFTWARE\Classes` and 624
/// under HKCU. That is more than thirteen times the curated list, and each one
/// costs a full resolution chain, so the window offers it as a button instead
/// of paying for it on every start.
pub fn installed() -> Vec<String> {
    let mut all = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for scope in Scope::ALL {
        // The classes root itself, which is what `key_path` builds with an
        // empty relative path.
        let Ok(root) = paths::root_key(scope).open(scope.classes_path()) else {
            continue;
        };

        for name in super::scan::subkey_names(&root) {
            if !name.starts_with('.') {
                continue;
            }
            // `normalize_ext` also refuses the nonsense that turns up here:
            // a bare dot, names with spaces, `.` inside the name.
            let Some(ext) = normalize_ext(&name) else {
                continue;
            };
            if seen.insert(ext.clone()) {
                all.push(ext);
            }
        }
    }

    all.sort();
    all
}

/// The extensions the window walks: the curated list plus the user's own.
///
/// Own extensions come last but win nothing — duplicates fall out — and they
/// are normalised on the way in, so `PNG`, `.png` and ` .PNG ` are one type
/// rather than three tree entries pointing at the same registry keys.
pub fn wanted(custom: &[String]) -> Vec<String> {
    let mut all: Vec<String> = CURATED.iter().map(|d| d.ext.to_string()).collect();
    let mut seen: std::collections::HashSet<String> = all.iter().cloned().collect();

    for raw in custom {
        let Some(ext) = normalize_ext(raw) else {
            continue;
        };
        if seen.insert(ext.clone()) {
            all.push(ext);
        }
    }

    all
}

/// Looks up the curated group of an extension.
pub fn group_of(ext: &str) -> TypeGroup {
    let normalized = normalize_ext(ext);
    let needle = normalized.as_deref().unwrap_or(ext);
    CURATED
        .iter()
        .find(|d| d.ext == needle)
        .map_or(TypeGroup::Other, |d| d.group)
}

/// What Windows knows about one extension.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Resolution {
    pub ext: String,
    /// `…\FileExts\<ext>\UserChoice`. Beats the class default.
    pub user_choice: Option<String>,
    /// Default value of `HKCR\<ext>`.
    pub default_progid: Option<String>,
    /// `text`, `image`, `audio`, `video`, `compressed`, … Absent for many
    /// extensions, and then level 3 of the chain does not exist at all.
    pub perceived_type: Option<String>,
    /// Value names under `HKCR\<ext>\OpenWithProgids`.
    pub open_with_progids: Vec<String>,
    /// Does the extension key exist anywhere?
    pub registered: bool,
}

impl Resolution {
    /// The ProgID that actually wins.
    ///
    /// The user's own choice takes precedence over the system default, and
    /// the two really do disagree: measured on this machine, `.jpg` defaults
    /// to `ImageGlass.AssocFile.JPG` while the user choice is a Store app.
    pub fn effective_progid(&self) -> Option<&str> {
        self.user_choice
            .as_deref()
            .or(self.default_progid.as_deref())
    }

    /// Every ProgID that contributes entries: the effective one first, then
    /// the alternatives from `OpenWithProgids`, without repeats.
    pub fn all_progids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for candidate in self
            .effective_progid()
            .into_iter()
            .map(str::to_string)
            .chain(self.default_progid.clone())
            .chain(self.open_with_progids.iter().cloned())
        {
            if !candidate.trim().is_empty() && !out.contains(&candidate) {
                out.push(candidate);
            }
        }
        out
    }
}

/// Reads everything needed to walk the chain for one extension.
pub fn resolve(ext: &str) -> Resolution {
    let Some(ext) = normalize_ext(ext) else {
        return Resolution::default();
    };

    let mut resolution = Resolution {
        ext: ext.clone(),
        // The user's choice lives outside the classes tree entirely.
        user_choice: CURRENT_USER
            .open(format!(
                r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoice"
            ))
            .ok()
            .and_then(|key| key.get_string("ProgId").ok())
            .filter(|value| !value.trim().is_empty()),
        ..Resolution::default()
    };

    // HKCU wins over HKLM, and the 64-bit view over the 32-bit one.
    for scope in Scope::ALL {
        let Ok(key) = paths::root_key(scope).open(paths::key_path(scope, &ext)) else {
            continue;
        };
        resolution.registered = true;

        if resolution.default_progid.is_none() {
            resolution.default_progid = key
                .get_string("")
                .ok()
                .filter(|value| !value.trim().is_empty());
        }
        if resolution.perceived_type.is_none() {
            // Lowercased on purpose. Different extensions spell the same
            // perceived type differently — measured on this machine: some
            // declare `image`, others `Image` — and without folding the case
            // the tree grows two branches for one type and the shared level-3
            // location gets scanned twice. Registry paths do not care about
            // case, so nothing is lost.
            resolution.perceived_type = key
                .get_string("PerceivedType")
                .ok()
                .map(|value| value.trim().to_lowercase())
                .filter(|value| !value.is_empty());
        }

        if let Ok(open_with) = key.open("OpenWithProgids")
            && let Ok(values) = open_with.values()
        {
            for (name, _) in values {
                if !name.trim().is_empty() && !resolution.open_with_progids.contains(&name) {
                    resolution.open_with_progids.push(name);
                }
            }
        }
    }

    resolution
}

/// The registry locations that contribute entries for one extension.
///
/// Levels 1 and 2 of the chain — `*` and `AllFilesystemObjects` — are the same
/// for every file type and are deliberately **not** included: they are scanned
/// once as base categories and reused. What comes back here is
/// levels 3 to 7.
pub fn sources_for(resolution: &Resolution) -> Vec<CategorySource> {
    let mut sources = Vec::new();
    let ext = &resolution.ext;
    if ext.is_empty() {
        return sources;
    }

    let mut push = |category: Category, relative: String| {
        sources.push(CategorySource {
            category: category.clone(),
            relative: format!(r"{relative}\shell"),
            kind: SourceKind::Shell,
            anchor: Anchor::Classes,
        });
        sources.push(CategorySource {
            category,
            relative: format!(r"{relative}\shellex\ContextMenuHandlers"),
            kind: SourceKind::ShellEx,
            anchor: Anchor::Classes,
        });
    };

    // Level 3: the perceived type. Skipped entirely when the extension has
    // none, which is the common case — `.pdf` has no PerceivedType at all.
    if let Some(perceived) = &resolution.perceived_type {
        push(
            Category::PerceivedType(perceived.clone()),
            format!(r"SystemFileAssociations\{perceived}"),
        );
    }

    // Level 4: the extension under SystemFileAssociations. Together with
    // level 3 this is where image viewers and converters register, and it is
    // what most competing tools miss.
    push(
        Category::ExtAssoc(ext.clone()),
        format!(r"SystemFileAssociations\{ext}"),
    );

    // Levels 5 and 7: the winning ProgID and every alternative.
    for prog_id in resolution.all_progids() {
        push(
            Category::ProgId {
                prog_id: prog_id.clone(),
                from_ext: ext.clone(),
            },
            prog_id,
        );
    }

    // Level 6: the extension key itself. Rare — it exists for none of the
    // eight extensions surveyed on this machine — but it does occur.
    push(Category::ExtDirect(ext.clone()), ext.clone());

    sources
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_users_own_extension_joins_the_curated_list_exactly_once() {
        // The setting was written to disk from milestone 5 on and read by
        // nobody until 2026-08-15 — a promise the program did not keep.
        let list = wanted(&[
            ".PNG".into(), // curated already, in the wrong case
            "png".into(),  // and again, without the dot
            "*.ctxmenu_probe".into(),
            "   ".into(), // nonsense from a half-typed field
            r"a\b".into(),
        ]);

        assert_eq!(
            list.iter().filter(|e| *e == ".png").count(),
            1,
            "an extension that is already curated must not appear twice"
        );
        assert!(list.contains(&".ctxmenu_probe".to_string()));
        assert_eq!(
            list.len(),
            CURATED.len() + 1,
            "exactly one usable extension was added; the rest is not one"
        );
    }

    #[test]
    fn every_installed_extension_is_found_and_normalised() {
        // Against the real registry: this is the "scan everything" path, and
        // what makes it worth having is that it finds far more than the
        // curated selection.
        let all = installed();

        assert!(
            all.len() > CURATED.len(),
            "a machine has more registered extensions than the curated {}: got {}",
            CURATED.len(),
            all.len()
        );
        assert!(
            all.iter().any(|ext| ext == ".txt"),
            "every Windows registers .txt"
        );

        for ext in &all {
            assert!(ext.starts_with('.'), "{ext} lacks the dot");
            assert_eq!(ext, &ext.to_lowercase(), "{ext} is not folded");
            assert!(ext.len() > 1, "a bare dot is not an extension");
        }

        // Sorted and without repeats — the three scopes overlap heavily, and
        // the tree would show the same type several times.
        let mut tidy = all.clone();
        tidy.sort();
        tidy.dedup();
        assert_eq!(tidy, all, "the list must be sorted and free of duplicates");
    }

    #[test]
    fn the_curated_list_has_no_duplicates() {
        let mut seen = HashSet::new();
        for def in CURATED {
            assert!(seen.insert(def.ext), "{} appears twice", def.ext);
        }
    }

    #[test]
    fn every_entry_is_lowercase_and_dotted() {
        for def in CURATED {
            assert!(def.ext.starts_with('.'), "{} lacks the dot", def.ext);
            assert_eq!(
                def.ext,
                def.ext.to_lowercase(),
                "{} is not lowercase",
                def.ext
            );
            assert!(def.ext.len() > 1, "{} is just a dot", def.ext);
        }
    }

    #[test]
    fn every_group_except_other_is_populated() {
        for group in TypeGroup::ALL {
            if group == TypeGroup::Other {
                continue;
            }
            assert!(
                CURATED.iter().any(|d| d.group == group),
                "{group:?} has no file types"
            );
        }
    }

    #[test]
    fn user_input_is_normalised_the_way_people_type_it() {
        for raw in ["jpg", ".jpg", ".JPG", "*.jpg", "  .Jpg  ", "*.JPG"] {
            assert_eq!(normalize_ext(raw).as_deref(), Some(".jpg"), "input {raw:?}");
        }
    }

    #[test]
    fn nonsense_input_is_refused() {
        for raw in ["", "   ", ".", "*", "*.", r"c:\pfad", "zwei worte", "a/b"] {
            assert_eq!(normalize_ext(raw), None, "input {raw:?} should be refused");
        }
    }

    #[test]
    fn groups_are_found_for_curated_types_and_default_otherwise() {
        assert_eq!(group_of(".jpg"), TypeGroup::Images);
        assert_eq!(group_of("JPG"), TypeGroup::Images);
        assert_eq!(group_of(".cr3"), TypeGroup::Raw);
        assert_eq!(group_of(".rs"), TypeGroup::Code);
        assert_eq!(group_of(".vhdx"), TypeGroup::System);
        assert_eq!(group_of(".gibtsnicht"), TypeGroup::Other);
    }

    // ------------------------------------------------------------------
    // The chain, checked against what this machine actually holds.
    // ------------------------------------------------------------------

    #[test]
    fn a_common_image_type_resolves_completely() {
        let jpg = resolve(".jpg");
        assert!(jpg.registered, ".jpg must be registered on any Windows");
        assert_eq!(
            jpg.perceived_type.as_deref(),
            Some("image"),
            "the perceived type drives level 3 of the chain"
        );
        assert!(
            jpg.effective_progid().is_some(),
            "some ProgID must win for .jpg"
        );
    }

    #[test]
    fn the_user_choice_beats_the_class_default() {
        let mut resolution = Resolution {
            default_progid: Some("System.Default".into()),
            user_choice: Some("Users.Pick".into()),
            ..Resolution::default()
        };
        assert_eq!(resolution.effective_progid(), Some("Users.Pick"));

        resolution.user_choice = None;
        assert_eq!(resolution.effective_progid(), Some("System.Default"));
    }

    #[test]
    fn the_default_progid_survives_even_when_the_user_chose_otherwise() {
        // Both branches carry entries, so losing the default would hide them.
        let resolution = Resolution {
            default_progid: Some("System.Default".into()),
            user_choice: Some("Users.Pick".into()),
            open_with_progids: vec!["Third.One".into(), "Users.Pick".into()],
            ..Resolution::default()
        };
        let all = resolution.all_progids();
        assert_eq!(all, vec!["Users.Pick", "System.Default", "Third.One"]);
    }

    #[test]
    fn a_type_without_a_perceived_type_skips_level_three() {
        // Measured: .pdf carries no PerceivedType on this machine.
        let without = Resolution {
            ext: ".pdf".into(),
            default_progid: Some("Some.Reader".into()),
            registered: true,
            ..Resolution::default()
        };
        let sources = sources_for(&without);
        assert!(
            !sources
                .iter()
                .any(|s| matches!(s.category, Category::PerceivedType(_))),
            "level 3 must be absent without a perceived type"
        );

        let with = Resolution {
            perceived_type: Some("image".into()),
            ..without.clone()
        };
        assert!(
            sources_for(&with)
                .iter()
                .any(|s| matches!(s.category, Category::PerceivedType(_)))
        );
    }

    #[test]
    fn the_chain_covers_levels_three_to_seven_and_not_one_or_two() {
        let resolution = Resolution {
            ext: ".jpg".into(),
            perceived_type: Some("image".into()),
            default_progid: Some("jpegfile".into()),
            registered: true,
            ..Resolution::default()
        };
        let sources = sources_for(&resolution);

        let has = |needle: &str| sources.iter().any(|s| s.relative.contains(needle));
        assert!(has(r"SystemFileAssociations\image"), "level 3");
        assert!(has(r"SystemFileAssociations\.jpg"), "level 4");
        assert!(has("jpegfile"), "level 5");
        assert!(
            sources
                .iter()
                .any(|s| matches!(&s.category, Category::ExtDirect(e) if e == ".jpg")),
            "level 6"
        );

        // Levels 1 and 2 are base categories, scanned once and reused.
        assert!(!has(r"*\shell"), "level 1 must not be repeated per type");
        assert!(!has("AllFilesystemObjects"), "level 2 must not be repeated");

        // Every location is looked at for both verbs and COM handlers.
        let shell = sources
            .iter()
            .filter(|s| s.kind == SourceKind::Shell)
            .count();
        let shellex = sources
            .iter()
            .filter(|s| s.kind == SourceKind::ShellEx)
            .count();
        assert_eq!(shell, shellex);
    }

    /// Guards the deduplication the scanner relies on.
    ///
    /// Every image extension names the same level-3 location. If that is
    /// scanned once per extension, each of them reports the shared entries
    /// several times over — measured before the fix: `.jpg` claimed 79
    /// entries where it has 19.
    #[test]
    fn image_extensions_name_the_same_level_three_location() {
        let jpg = sources_for(&resolve(".jpg"));
        let png = sources_for(&resolve(".png"));

        let level3 = |sources: &[CategorySource]| -> Vec<String> {
            sources
                .iter()
                .filter(|s| matches!(s.category, Category::PerceivedType(_)))
                .map(|s| s.relative.to_lowercase())
                .collect()
        };

        let a = level3(&jpg);
        let b = level3(&png);
        assert!(!a.is_empty(), "images must have a perceived type");
        assert_eq!(a, b, "the shared location must be spelled identically");
    }

    #[test]
    fn perceived_types_are_case_folded() {
        // `.tif` and `.jpg` both mean the image perceived type, but the two
        // keys do not agree on capitalisation. Two branches for one type
        // would double both the tree and the scanning work.
        let types: HashSet<String> = [".jpg", ".jpeg", ".png", ".tif", ".tiff", ".bmp", ".gif"]
            .iter()
            .filter_map(|ext| resolve(ext).perceived_type)
            .collect();

        for value in &types {
            assert_eq!(
                value,
                &value.to_lowercase(),
                "perceived type {value:?} was not folded"
            );
        }
        assert!(
            types.len() <= 1,
            "image extensions should share one perceived type, got {types:?}"
        );
    }

    #[test]
    fn an_unregistered_extension_yields_nothing_but_does_not_fail() {
        let resolution = resolve(".gibtesganzsichernicht");
        assert!(!resolution.registered);
        assert_eq!(resolution.effective_progid(), None);
        assert!(resolution.all_progids().is_empty());
    }

    /// The list is a starting point, but a shrunken one would silently drop
    /// coverage. This pins the breadth the list is meant to have.
    #[test]
    fn the_list_covers_the_documented_breadth() {
        assert!(
            CURATED.len() >= 95,
            "only {} curated types, the list should cover about 100",
            CURATED.len()
        );
    }
}
