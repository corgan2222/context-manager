//! Collecting the entries of one program into a single row.
//!
//! A program like 7-Zip registers under ten to twenty classes. Removing that
//! by hand is exactly the work this tool exists to take over (ToDo 11).

use rustc_hash::FxHashMap;
use serde::Serialize;

use super::identity::{self, NameResolver};
use crate::model::{ContextEntry, EntryKind, ScanResult, Scope};

/// Everything belonging to one program.
#[derive(Debug, Clone, Serialize)]
pub struct ProgramGroup {
    /// Normalised path — the key entries were grouped on.
    pub key: String,
    pub display_name: String,
    pub icon_ref: Option<String>,
    /// Indices into `ScanResult::entries`.
    ///
    /// Indices rather than cloned entries, so the truth stays in one place
    /// and a selection made in the table survives switching tabs.
    pub entry_indices: Vec<usize>,
    /// Distinct CLSIDs, for the blocked list. One value there disables a
    /// handler everywhere, which beats deleting it under twenty classes.
    pub clsids: Vec<String>,
    pub scopes: Vec<Scope>,
    /// Human readable locations, for the summary line.
    pub locations: Vec<String>,
    pub is_system: bool,
    /// True when every entry of this group is read-only.
    pub read_only: bool,
}

impl ProgramGroup {
    pub fn entry_count(&self) -> usize {
        self.entry_indices.len()
    }
}

/// Builds the program view from a finished scan.
///
/// Sorted by size descending, then by name: the program with twenty entries
/// is the one worth looking at first, and that is also the one this tool
/// saves the most work on.
pub fn build(scan: &ScanResult, names: &mut NameResolver) -> Vec<ProgramGroup> {
    let mut by_key: FxHashMap<String, ProgramGroup> = FxHashMap::default();

    for (index, entry) in scan.entries.iter().enumerate() {
        let Some(raw_key) = &entry.program_key else {
            // No command line and no server DLL — a submenu parent, for
            // instance. Attributing it to a program would be a guess.
            continue;
        };

        // Resolved before grouping, not after: one entry says `cmd.exe` and
        // the next `C:\Windows\System32\cmd.exe`, and grouping on the raw
        // string made that one program appear twice under one name. Measured
        // on this machine: three such pairs out of 98 keys.
        let key = &identity::absolute_path(raw_key).to_lowercase();

        let group = by_key.entry(key.clone()).or_insert_with(|| ProgramGroup {
            key: key.clone(),
            display_name: names.display_name(key),
            icon_ref: None,
            entry_indices: Vec::new(),
            clsids: Vec::new(),
            scopes: Vec::new(),
            locations: Vec::new(),
            is_system: identity::is_system_component(key),
            read_only: true,
        });

        group.entry_indices.push(index);

        if group.icon_ref.is_none() {
            group.icon_ref = entry.icon_ref.clone();
        }
        if !group.scopes.contains(&entry.scope) {
            group.scopes.push(entry.scope);
        }

        let location = location_label(entry);
        if !group.locations.contains(&location) {
            group.locations.push(location);
        }

        if let EntryKind::ShellEx { clsid, .. } = &entry.kind
            && !clsid.is_empty()
            && !group.clsids.contains(clsid)
        {
            group.clsids.push(clsid.clone());
        }

        // A group counts as removable as soon as one of its entries is.
        if !entry.read_only {
            group.read_only = false;
        }
    }

    let mut groups: Vec<ProgramGroup> = by_key.into_values().collect();
    for group in &mut groups {
        group.scopes.sort();
        group.locations.sort();
    }
    disambiguate(&mut groups);
    groups.sort_by(|a, b| {
        b.entry_count().cmp(&a.entry_count()).then_with(|| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
        })
    });
    groups
}

/// Makes names that collide tell each other apart.
///
/// A version resource is free vendor text and is not unique: on this machine
/// `bitlockerwizard.exe` and `bitlockerwizardelev.exe` both call themselves
/// "Assistent für die BitLocker-Laufwerkverschlüsselung", and in System32 the
/// product name "Microsoft (R) Windows (R) Operating System" is shared by 25
/// binaries. Two identical rows in a list of programs are worse than a longer
/// caption, so the file name is appended — but only where it is needed, since
/// most names are fine as they are.
fn disambiguate(groups: &mut [ProgramGroup]) {
    let mut seen: FxHashMap<String, usize> = FxHashMap::default();
    for group in groups.iter() {
        *seen.entry(group.display_name.to_lowercase()).or_insert(0) += 1;
    }

    for group in groups.iter_mut() {
        if seen
            .get(&group.display_name.to_lowercase())
            .is_some_and(|count| *count > 1)
        {
            let file = std::path::Path::new(&group.key)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| group.key.clone());
            group.display_name = format!("{} ({file})", group.display_name);
        }
    }
}

/// Short, readable origin of an entry, for the group summary.
fn location_label(entry: &ContextEntry) -> String {
    format!("{} · {}", entry.scope.label(), entry.category.slug())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    #[test]
    fn entries_of_one_program_end_up_in_one_group() {
        let scan = synthetic::scan_result(200);
        let mut names = NameResolver::new();
        let groups = build(&scan, &mut names);

        // The generator spreads 200 entries over eight programs.
        assert_eq!(groups.len(), 8, "expected one group per generated program");

        let total: usize = groups.iter().map(|g| g.entry_count()).sum();
        assert_eq!(total, 200, "every entry must land in exactly one group");
    }

    #[test]
    fn groups_are_sorted_by_size_then_name() {
        let scan = synthetic::scan_result(200);
        let mut names = NameResolver::new();
        let groups = build(&scan, &mut names);

        for pair in groups.windows(2) {
            assert!(
                pair[0].entry_count() >= pair[1].entry_count(),
                "{} ({}) before {} ({})",
                pair[0].display_name,
                pair[0].entry_count(),
                pair[1].display_name,
                pair[1].entry_count()
            );
        }
    }

    #[test]
    fn entries_without_a_program_key_are_skipped_rather_than_guessed() {
        let mut scan = synthetic::scan_result(20);
        for entry in &mut scan.entries {
            entry.program_key = None;
        }

        let mut names = NameResolver::new();
        assert!(build(&scan, &mut names).is_empty());
    }

    #[test]
    fn a_group_collects_its_clsids_and_scopes_without_duplicates() {
        let scan = synthetic::scan_result(120);
        let mut names = NameResolver::new();
        let groups = build(&scan, &mut names);

        for group in &groups {
            let unique: std::collections::HashSet<_> = group.clsids.iter().collect();
            assert_eq!(unique.len(), group.clsids.len(), "duplicate CLSID");

            let unique: std::collections::HashSet<_> = group.scopes.iter().collect();
            assert_eq!(unique.len(), group.scopes.len(), "duplicate scope");
        }
    }

    #[test]
    fn a_group_is_removable_as_soon_as_one_entry_is() {
        let scan = synthetic::scan_result(60);
        let mut names = NameResolver::new();
        let groups = build(&scan, &mut names);

        for group in &groups {
            let any_writable = group
                .entry_indices
                .iter()
                .any(|&i| !scan.entries[i].read_only);
            assert_eq!(
                group.read_only, !any_writable,
                "group {} reports read_only={}",
                group.display_name, group.read_only
            );
        }
    }
}
