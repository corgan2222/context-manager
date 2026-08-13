//! Synthetic scan results for performance work.
//!
//! Milestone 4 has to hold 60 fps at 2.000 rows, but this machine's registry
//! only yields around 165 entries. Rather than measure something smaller than
//! the target and hope, the table can be pointed at a generated result of any
//! size — the rendering path does not care where the entries came from.
//!
//! Deterministic on purpose: two runs must be comparable, so nothing here is
//! random.

use crate::model::{Category, ContextEntry, EntryKind, ScanResult, ScanStats, Scope, stable_id};

const PROGRAMS: [(&str, &str); 8] = [
    ("7-Zip", r"C:\Program Files\7-Zip\7zFM.exe"),
    ("VLC media player", r"C:\Program Files\VideoLAN\VLC\vlc.exe"),
    ("Notepad++", r"C:\Program Files\Notepad++\notepad++.exe"),
    ("IrfanView", r"C:\Program Files\IrfanView\i_view64.exe"),
    ("Git for Windows", r"C:\Program Files\Git\git-bash.exe"),
    ("TeraCopy", r"C:\Program Files\TeraCopy\TeraCopy.exe"),
    (
        "Visual Studio Code",
        r"C:\Program Files\Microsoft VS Code\Code.exe",
    ),
    ("Ümläut & Co", r"C:\Program Files\Ümläut & Co\tool.exe"),
];

const ICONS: [&str; 4] = [
    r"%SystemRoot%\system32\shell32.dll,-244",
    r"%SystemRoot%\system32\imageres.dll,-109",
    r"C:\Program Files\7-Zip\7zFM.exe,0",
    r"%SystemRoot%\system32\shell32.dll,-16826",
];

/// Builds `count` entries spread evenly over categories, scopes and programs.
pub fn scan_result(count: usize) -> ScanResult {
    let categories = Category::BASE;
    let mut entries = Vec::with_capacity(count);

    for i in 0..count {
        let category = categories[i % categories.len()].clone();
        let scope = Scope::ALL[(i / 3) % Scope::ALL.len()];
        let (program, executable) = PROGRAMS[i % PROGRAMS.len()];

        let key_name = format!("Synthetic{i:05}");
        let relative = format!("{}\\shell\\{key_name}", category.slug());
        let registry_path = format!("{}\\{}\\{relative}", scope.hive(), scope.classes_path());

        // Every fourth entry is a COM handler, matching the roughly 45 % share
        // measured on this machine.
        let kind = if i % 4 == 3 {
            EntryKind::ShellEx {
                clsid: format!("{{{:08X}-0000-0000-0000-000000000000}}", i),
                server_path: Some(executable.replace(".exe", ".dll")),
                blocked: i % 40 == 3,
            }
        } else {
            EntryKind::Verb {
                command: Some(format!("\"{executable}\" \"%V\" --entry {i}")),
                sub_commands: Vec::new(),
            }
        };

        entries.push(ContextEntry {
            id: stable_id(scope, &registry_path),
            key_name,
            display_name: format!("{program} — Eintrag {i}"),
            raw_display: None,
            icon_ref: Some(ICONS[i % ICONS.len()].to_string()),
            position: match i % 10 {
                0 => Some("Top".into()),
                5 => Some("Bottom".into()),
                _ => None,
            },
            extended: i % 7 == 0,
            hidden: i % 11 == 0,
            applies_to: None,
            kind,
            scope,
            category,
            registry_path,
            read_only: scope != Scope::User,
            program_key: Some(executable.to_lowercase()),
        });
    }

    ScanResult::new(entries, Vec::new(), ScanStats::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requested_number_of_entries_comes_back() {
        assert_eq!(scan_result(0).entries.len(), 0);
        assert_eq!(scan_result(2000).entries.len(), 2000);
    }

    #[test]
    fn ids_are_unique_so_selection_cannot_collide() {
        let result = scan_result(2000);
        let unique: std::collections::HashSet<_> =
            result.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(unique.len(), 2000, "generated ids must not collide");
    }

    #[test]
    fn every_category_and_scope_is_represented() {
        let result = scan_result(500);
        for category in Category::BASE {
            assert!(
                result.by_category.contains_key(&category),
                "{category:?} missing"
            );
        }
        for scope in Scope::ALL {
            assert!(
                result.entries.iter().any(|e| e.scope == scope),
                "{scope:?} missing"
            );
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = scan_result(200);
        let b = scan_result(200);
        assert_eq!(a.entries, b.entries, "two runs must be comparable");
    }

    #[test]
    fn both_entry_kinds_occur() {
        let result = scan_result(100);
        let shellex = result
            .entries
            .iter()
            .filter(|e| matches!(e.kind, EntryKind::ShellEx { .. }))
            .count();
        assert!(shellex > 0 && shellex < 100, "got {shellex} COM handlers");
    }
}
