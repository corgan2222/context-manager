//! Resolving indirect string and path references.
//!
//! Display names in the registry are frequently not text but a pointer into a
//! binary's string table, e.g. `@%SystemRoot%\system32\shell32.dll,-8506`.
//! Showing that raw is what makes competing tools look unfinished.

use rustc_hash::FxHashMap;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::UI::Shell::SHLoadIndirectString;
use windows::core::HSTRING;

/// Menu strings are short; this is well past the longest seen in the wild.
const MUI_BUFFER: usize = 1024;

/// Resolves `@file,-id` references, caching the results.
///
/// The cache matters more than it looks: a full scan hits the same
/// `shell32.dll` and `imageres.dll` references dozens of times, and every miss
/// is a file load plus a resource lookup.
#[derive(Default)]
pub struct MuiResolver {
    cache: FxHashMap<String, String>,
    hits: usize,
    misses: usize,
}

impl MuiResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the display text for a raw registry value.
    ///
    /// Anything not starting with `@` is already literal text and is returned
    /// unchanged. A failed lookup yields the raw string rather than an error:
    /// a broken reference is a display problem, never a reason to abort a scan.
    pub fn resolve(&mut self, raw: &str) -> String {
        if !raw.starts_with('@') {
            return raw.to_string();
        }

        if let Some(hit) = self.cache.get(raw) {
            self.hits += 1;
            return hit.clone();
        }

        self.misses += 1;
        let resolved = load_indirect_string(raw).unwrap_or_else(|| raw.to_string());
        self.cache.insert(raw.to_string(), resolved.clone());
        resolved
    }

    /// Cache effectiveness, for the measurements the handover asks for.
    pub fn stats(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }
}

fn load_indirect_string(raw: &str) -> Option<String> {
    let source = HSTRING::from(raw);
    let mut buffer = vec![0u16; MUI_BUFFER];

    // windows 0.62 returns `Result<()>` here. The ToDo still shows the older
    // shape that needed a trailing `.ok()?` on an HRESULT.
    unsafe { SHLoadIndirectString(&source, &mut buffer, None) }.ok()?;

    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..len]);

    // A resolver that "succeeds" with nothing is worse than one that fails:
    // the caller would show an empty menu entry instead of the raw reference.
    (!text.trim().is_empty()).then_some(text)
}

/// Expands `%SystemRoot%`-style references in a path.
///
/// Returns the input unchanged when expansion fails, for the same reason as
/// above: a path we cannot expand is still worth showing.
pub fn expand_env(raw: &str) -> String {
    let source = HSTRING::from(raw);

    unsafe {
        let needed = ExpandEnvironmentStringsW(&source, None);
        if needed == 0 {
            return raw.to_string();
        }

        let mut buffer = vec![0u16; needed as usize];
        let written = ExpandEnvironmentStringsW(&source, Some(&mut buffer));
        if written == 0 || written as usize > buffer.len() {
            return raw.to_string();
        }

        // The count includes the terminating null.
        String::from_utf16_lossy(&buffer[..written as usize - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_text_passes_through_untouched() {
        let mut resolver = MuiResolver::new();
        assert_eq!(resolver.resolve("Open with VLC"), "Open with VLC");
        assert_eq!(resolver.resolve(""), "");
        assert_eq!(resolver.stats(), (0, 0), "no lookup for literal text");
    }

    #[test]
    fn a_broken_reference_falls_back_to_the_raw_string() {
        let mut resolver = MuiResolver::new();
        let raw = r"@C:\does\not\exist.dll,-9999";
        assert_eq!(resolver.resolve(raw), raw);
    }

    /// The reference behind the `cmd` verb on every Windows installation.
    #[test]
    fn a_shell32_reference_resolves_to_real_text() {
        let mut resolver = MuiResolver::new();
        let raw = r"@%SystemRoot%\system32\shell32.dll,-8506";
        let resolved = resolver.resolve(raw);

        assert_ne!(resolved, raw, "shell32 string table should be readable");
        assert!(!resolved.trim().is_empty());
        assert!(!resolved.starts_with('@'));
    }

    #[test]
    fn repeated_lookups_come_from_the_cache() {
        let mut resolver = MuiResolver::new();
        let raw = r"@%SystemRoot%\system32\shell32.dll,-8506";

        let first = resolver.resolve(raw);
        let second = resolver.resolve(raw);

        assert_eq!(first, second);
        assert_eq!(resolver.stats(), (1, 1), "one miss then one hit");
    }

    #[test]
    fn environment_variables_are_expanded() {
        let expanded = expand_env(r"%SystemRoot%\system32\shell32.dll");
        assert!(
            expanded.to_lowercase().contains(r"\system32\shell32.dll"),
            "unexpected expansion: {expanded}"
        );
        assert!(!expanded.contains('%'));
    }

    #[test]
    fn a_path_without_variables_survives_expansion() {
        let path = r"C:\Program Files\7-Zip\7-zip.dll";
        assert_eq!(expand_env(path), path);
    }
}
