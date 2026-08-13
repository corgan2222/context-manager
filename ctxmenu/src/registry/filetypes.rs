//! File types and the chain Windows walks to build their context menu.
//!
//! Right-clicking a `.jpg` shows entries drawn from at least seven different
//! registry branches (ToDo 10.1). Two of them — the perceived type and the
//! extension-specific `SystemFileAssociations` key — are where image viewers,
//! converters and photo tools register, and they are exactly the two most
//! tools overlook.

use serde::{Deserialize, Serialize};

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

/// The starting point, not the limit (ToDo 10.3).
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

/// Looks up the curated group of an extension.
pub fn group_of(ext: &str) -> TypeGroup {
    let normalized = normalize_ext(ext);
    let needle = normalized.as_deref().unwrap_or(ext);
    CURATED
        .iter()
        .find(|d| d.ext == needle)
        .map_or(TypeGroup::Other, |d| d.group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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

    /// The list is a starting point, but a shrunken one would silently drop
    /// coverage. This pins the size the ToDo asks for.
    #[test]
    fn the_list_covers_the_documented_breadth() {
        assert!(
            CURATED.len() >= 95,
            "only {} curated types, the plan lists about 100",
            CURATED.len()
        );
    }
}
