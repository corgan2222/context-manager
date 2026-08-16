//! Resolving COM handler registrations.
//!
//! A `shellex` key holds nothing but a CLSID. Everything a user could
//! recognise — the handler's name and the DLL behind it — lives under
//! `CLSID\{…}`, in whichever hive registered it.

use rustc_hash::{FxHashMap, FxHashSet};
use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

use super::mui;
use super::paths::SHELL_EXTENSIONS_BLOCKED;
use crate::model::Scope;

/// What could be learned about a CLSID.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClsidInfo {
    /// Default value of the `CLSID\{…}` key, e.g. "7-Zip Shell Extension".
    pub friendly_name: Option<String>,
    /// Expanded path from `InprocServer32`.
    pub server_path: Option<String>,
    /// Normalised server path — the grouping key for the program view.
    pub program_key: Option<String>,
    /// Which hive the CLSID was found in.
    pub scope: Option<Scope>,
}

/// Looks up CLSIDs and knows which of them Windows has been told to block.
pub struct ClsidResolver {
    cache: FxHashMap<String, ClsidInfo>,
    blocked: FxHashSet<String>,
    /// Handler names use the same indirect form as verbs, so this resolver
    /// carries its own — sharing one cache across all CLSIDs beats building a
    /// throwaway resolver per lookup.
    mui: mui::MuiResolver,
}

impl ClsidResolver {
    /// Reads the blocked list once up front.
    ///
    /// One value there disables a handler everywhere at once, which is why the
    /// program view prefers blocking over deleting the same handler under
    /// twenty classes.
    pub fn new() -> Self {
        let mut blocked = FxHashSet::default();

        if let Ok(key) = LOCAL_MACHINE.open(SHELL_EXTENSIONS_BLOCKED)
            && let Ok(values) = key.values()
        {
            for (name, _) in values {
                blocked.insert(name.to_uppercase());
            }
        }

        Self {
            cache: FxHashMap::default(),
            blocked,
            mui: mui::MuiResolver::new(),
        }
    }

    pub fn is_blocked(&self, clsid: &str) -> bool {
        self.blocked.contains(&clsid.to_uppercase())
    }

    pub fn blocked_count(&self) -> usize {
        self.blocked.len()
    }

    /// Resolves a CLSID, caching both hits and misses.
    ///
    /// Search order mirrors how `HKCR` merges: the user's own registration
    /// wins, then the 64-bit machine view, then the 32-bit one. A 32-bit
    /// handler registered only under `WOW6432Node` is exactly the case
    /// competing tools tend to miss.
    pub fn resolve(&mut self, clsid: &str) -> ClsidInfo {
        if clsid.is_empty() {
            return ClsidInfo::default();
        }

        // Same normalisation as `is_blocked`: GUIDs turn up in both cases
        // across a scan, and a cache keyed on the raw string would miss the
        // second spelling and pay for the full three-hive lookup again.
        let key = clsid.to_uppercase();
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }

        let info = lookup(clsid, &mut self.mui);
        self.cache.insert(key, info.clone());
        info
    }
}

impl Default for ClsidResolver {
    fn default() -> Self {
        Self::new()
    }
}

fn lookup(clsid: &str, mui_resolver: &mut mui::MuiResolver) -> ClsidInfo {
    for scope in [Scope::User, Scope::Machine, Scope::Machine32] {
        let root = match scope {
            Scope::User => CURRENT_USER,
            _ => LOCAL_MACHINE,
        };
        let path = format!("{}\\CLSID\\{clsid}", scope.classes_path());

        let Ok(key) = root.open(&path) else { continue };

        let friendly_name = key
            .get_string("")
            .ok()
            .filter(|s| !s.trim().is_empty())
            // A handler name reaches the menu the same way a verb name does,
            // so it goes through the same accelerator rule.
            .map(|raw| mui::strip_accelerator(&mui_resolver.resolve(&raw)));

        let server_path = key
            .open("InprocServer32")
            .ok()
            .and_then(|server| server.get_string("").ok())
            .filter(|s| !s.trim().is_empty())
            .map(|raw| mui::expand_env(&raw));

        return ClsidInfo {
            program_key: server_path.as_deref().map(normalize_path),
            friendly_name,
            server_path,
            scope: Some(scope),
        };
    }

    ClsidInfo::default()
}

/// Normalises a file path into a grouping key.
///
/// Windows paths are case-insensitive and accept either slash, so the same
/// DLL can appear a dozen different ways across a dozen registrations. The
/// program view groups on this, so it has to collapse all of them.
pub fn normalize_path(path: &str) -> String {
    let lowered = path.trim().trim_matches('"').to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut previous_was_separator = false;

    for ch in lowered.chars() {
        let ch = if ch == '/' { '\\' } else { ch };
        let is_separator = ch == '\\';

        // Collapse doubled separators, but keep a leading UNC "\\".
        if is_separator && previous_was_separator && !out.is_empty() && out.len() > 1 {
            continue;
        }

        out.push(ch);
        previous_was_separator = is_separator;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_that_differ_only_cosmetically_group_together() {
        let expected = r"c:\program files\7-zip\7-zip.dll";
        assert_eq!(
            normalize_path(r"C:\Program Files\7-Zip\7-zip.dll"),
            expected
        );
        assert_eq!(
            normalize_path(r"C:/Program Files/7-Zip/7-zip.dll"),
            expected
        );
        assert_eq!(
            normalize_path(r"C:\Program Files\\7-Zip\\7-zip.dll"),
            expected
        );
        assert_eq!(
            normalize_path("  \"C:\\Program Files\\7-Zip\\7-zip.dll\"  "),
            expected
        );
    }

    #[test]
    fn unc_prefixes_survive_normalisation() {
        assert_eq!(
            normalize_path(r"\\server\share\tool.dll"),
            r"\\server\share\tool.dll"
        );
    }

    #[test]
    fn an_unknown_clsid_resolves_to_nothing_rather_than_failing() {
        let mut resolver = ClsidResolver::new();
        let info = resolver.resolve("{00000000-0000-0000-0000-000000000000}");
        assert_eq!(info, ClsidInfo::default());
        assert_eq!(resolver.resolve(""), ClsidInfo::default());
    }

    /// The shell's own "Send to" handler, present on every Windows install.
    #[test]
    fn a_known_windows_clsid_resolves_to_a_server_path() {
        let mut resolver = ClsidResolver::new();
        let info = resolver.resolve("{7BA4C740-9E81-11CF-99D3-00AA004AE837}");

        let server = info
            .server_path
            .expect("SendTo handler must have an InprocServer32");
        assert!(
            server.to_lowercase().contains("shell32.dll"),
            "unexpected server: {server}"
        );
        assert_eq!(
            info.program_key.as_deref(),
            Some(normalize_path(&server).as_str())
        );
    }

    #[test]
    fn repeated_lookups_are_cached() {
        let mut resolver = ClsidResolver::new();
        let clsid = "{7BA4C740-9E81-11CF-99D3-00AA004AE837}";
        assert_eq!(resolver.resolve(clsid), resolver.resolve(clsid));
    }

    /// `is_blocked` folds case because GUIDs turn up spelled both ways
    /// across a scan; the cache has to fold it the same way or the second
    /// spelling misses and repeats the full three-hive lookup for nothing.
    #[test]
    fn a_cache_hit_survives_a_different_letter_case() {
        let mut resolver = ClsidResolver::new();
        let lower = "{7ba4c740-9e81-11cf-99d3-00aa004ae837}";
        let upper = "{7BA4C740-9E81-11CF-99D3-00AA004AE837}";

        let first = resolver.resolve(lower);
        assert_eq!(resolver.cache.len(), 1);

        let second = resolver.resolve(upper);
        assert_eq!(second, first, "the two spellings name the same CLSID");
        assert_eq!(
            resolver.cache.len(),
            1,
            "a different case of the same CLSID must reuse the cached entry"
        );
    }
}
