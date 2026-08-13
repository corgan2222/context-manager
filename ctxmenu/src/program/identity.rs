//! Turning a program path into a name a person recognises.
//!
//! "7-Zip Shell Extension" instead of `c:\program files\7-zip\7-zip.dll`
//! (ToDo 11.1). The name lives in the binary's version resource, which is a
//! nested block of counted structures reached through three separate calls.
//!
//! Reading it costs a synchronous disk hit — measured at roughly 0.33 ms per
//! file warm — so it happens once after a scan, in the worker, never in the
//! frame path.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use rustc_hash::FxHashMap;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::core::{PCWSTR, w};

/// One entry of `\VarFileInfo\Translation`: language id and code page.
///
/// The `windows` crate generates no type for this — the whole block format is
/// undeclared — so it is declared here.
#[repr(C)]
#[derive(Clone, Copy)]
struct LangAndCodepage {
    language: u16,
    codepage: u16,
}

/// Owns the raw version block.
///
/// `Vec<u32>` rather than `Vec<u8>` on purpose: the block is a DWORD-structured
/// resource and the structures read out of it must be aligned. A `Vec<u8>`
/// guarantees only byte alignment, which x86 tolerates and other targets do
/// not.
struct VersionBlock {
    data: Vec<u32>,
}

impl VersionBlock {
    fn load(path: &str) -> Option<Self> {
        // The pointer must outlive both calls below. Writing
        // `PCWSTR(wide(path).as_ptr())` inline would drop the buffer at the
        // end of the statement and hand version.dll freed memory — which
        // fails silently, as a missing version resource.
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let file = PCWSTR(wide.as_ptr());

        // Zero means "no version resource" *or* "file not found". Both are
        // ordinary: 74 of 4821 binaries under System32 and Program Files
        // carry no version resource at all.
        let size = unsafe { GetFileVersionInfoSizeW(file, None) };
        if size == 0 {
            return None;
        }

        let mut data = vec![0u32; (size as usize).div_ceil(4)];
        unsafe { GetFileVersionInfoW(file, None, size, data.as_mut_ptr().cast::<c_void>()) }
            .ok()?;

        Some(Self { data })
    }

    /// Reads a string value out of the block.
    ///
    /// The value is copied immediately: `VerQueryValueW` hands back a pointer
    /// *into* the block, which dangles as soon as the block is dropped.
    fn string(&self, sub_block: &str) -> Option<String> {
        let wide: Vec<u16> = sub_block.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buffer: *mut c_void = ptr::null_mut();
        let mut len: u32 = 0;

        // Returns a Win32 BOOL, not a Result, and leaves the out-parameters
        // untouched on failure — hence the explicit null check.
        let ok = unsafe {
            VerQueryValueW(
                self.data.as_ptr().cast::<c_void>(),
                PCWSTR(wide.as_ptr()),
                &mut buffer,
                &mut len,
            )
        };
        if !ok.as_bool() || buffer.is_null() || len == 0 {
            return None;
        }

        // For string values `len` counts UTF-16 characters and includes the
        // terminator. Keeping that terminator would carry a NUL into labels
        // and JSON, where it looks like a rendering fault.
        let chars = unsafe { std::slice::from_raw_parts(buffer.cast::<u16>(), len as usize) };
        let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
        let text = String::from_utf16_lossy(&chars[..end]).trim().to_string();

        (!text.is_empty()).then_some(text)
    }

    /// The translations the file declares, plus the classic fallbacks.
    ///
    /// Here `len` is a size in *bytes*, unlike the string case.
    fn translations(&self) -> Vec<LangAndCodepage> {
        let mut buffer: *mut c_void = ptr::null_mut();
        let mut len: u32 = 0;

        let ok = unsafe {
            VerQueryValueW(
                self.data.as_ptr().cast::<c_void>(),
                w!("\\VarFileInfo\\Translation"),
                &mut buffer,
                &mut len,
            )
        };

        let mut out = Vec::new();
        if ok.as_bool() && !buffer.is_null() {
            let count = len as usize / size_of::<LangAndCodepage>();
            out.extend_from_slice(unsafe {
                std::slice::from_raw_parts(buffer.cast::<LangAndCodepage>(), count)
            });
        }

        // Files without a VarFileInfo block usually still answer to these.
        for (language, codepage) in [(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0000, 0x04B0)] {
            if !out
                .iter()
                .any(|t| t.language == language && t.codepage == codepage)
            {
                out.push(LangAndCodepage { language, codepage });
            }
        }
        out
    }
}

/// Reads the display name of a program.
///
/// Tries `FileDescription` across every declared translation, then
/// `ProductName`. Returns `None` rather than an error for anything without a
/// usable resource — a nameless program is a display problem, never a reason
/// to fail a scan.
pub fn file_description(path: &str) -> Option<String> {
    let block = VersionBlock::load(path)?;
    let translations = block.translations();

    // FileDescription first across all languages, then ProductName across
    // all languages. An empty FileDescription counts as absent: 57 of 4747
    // files with a version resource have one that is present but blank, and
    // taking it would put empty labels in the list.
    for name in ["FileDescription", "ProductName"] {
        for translation in &translations {
            let sub_block = format!(
                "\\StringFileInfo\\{:04x}{:04x}\\{name}",
                translation.language, translation.codepage
            );
            if let Some(value) = block.string(&sub_block) {
                return Some(value);
            }
        }
    }

    None
}

/// Resolves a bare executable name against the system directory.
///
/// Registry commands frequently say just `powershell.exe`, which is not a path
/// and carries no version resource until it is one.
fn absolute_path(path: &str) -> String {
    let candidate = Path::new(path);
    if candidate.is_absolute() || path.contains('\\') || path.contains('/') {
        return path.to_string();
    }

    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let in_system32 = format!(r"{system_root}\System32\{path}");
    if Path::new(&in_system32).is_file() {
        return in_system32;
    }
    path.to_string()
}

/// Names programs, remembering what it already looked up.
///
/// The same DLL backs a dozen entries — `shell32.dll` alone accounts for
/// twelve on this machine — so without the cache the disk would be hit twelve
/// times for one answer.
#[derive(Default)]
pub struct NameResolver {
    cache: FxHashMap<String, String>,
    hits: usize,
    lookups: usize,
}

impl NameResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// The best available name for a program path.
    ///
    /// Falls back to the file name without extension, and finally to the path
    /// itself, so this never returns something unprintable.
    pub fn display_name(&mut self, program_key: &str) -> String {
        if let Some(hit) = self.cache.get(program_key) {
            self.hits += 1;
            return hit.clone();
        }

        self.lookups += 1;
        let absolute = absolute_path(program_key);
        let name = file_description(&absolute).unwrap_or_else(|| file_name(program_key));

        self.cache.insert(program_key.to_string(), name.clone());
        name
    }

    /// Cache hits and actual lookups.
    pub fn stats(&self) -> (usize, usize) {
        (self.hits, self.lookups)
    }
}

/// File name without extension, capitalised as written.
fn file_name(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string())
}

/// Does this program live inside the Windows directory?
///
/// The program view marks those as system components and warns before acting
/// on them (ToDo 11.1).
pub fn is_system_component(path: &str) -> bool {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    absolute_path(path)
        .to_lowercase()
        .starts_with(&system_root.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_system_dll_has_a_description() {
        let name = file_description(r"C:\Windows\System32\shell32.dll")
            .expect("shell32 always carries a version resource");
        assert!(!name.trim().is_empty());
        assert!(!name.contains('\0'), "terminator leaked into the name");
    }

    #[test]
    fn files_without_a_version_resource_yield_nothing() {
        // A text file that exists on every Windows installation.
        assert_eq!(
            file_description(r"C:\Windows\System32\drivers\etc\hosts"),
            None
        );
        assert_eq!(file_description(r"C:\gibt\es\nicht.dll"), None);
        assert_eq!(file_description(""), None);
    }

    #[test]
    fn a_command_line_is_not_a_path() {
        // Guards the boundary: quotes and arguments must be stripped by the
        // caller. If this ever starts returning a name, someone has taught
        // this function to parse command lines, which is not its job.
        assert_eq!(
            file_description(r#""C:\Windows\System32\shell32.dll" "%1""#),
            None
        );
    }

    #[test]
    fn bare_executable_names_resolve_against_system32() {
        let resolved = absolute_path("notepad.exe");
        assert!(
            resolved.to_lowercase().contains(r"\system32\notepad.exe"),
            "got {resolved}"
        );
        // A path stays untouched.
        assert_eq!(absolute_path(r"C:\a\b.exe"), r"C:\a\b.exe");
    }

    #[test]
    fn the_resolver_falls_back_to_the_file_name() {
        let mut resolver = NameResolver::new();
        assert_eq!(resolver.display_name(r"C:\gibt\es\nicht.dll"), "nicht");
    }

    #[test]
    fn repeated_lookups_come_from_the_cache() {
        let mut resolver = NameResolver::new();
        let path = r"C:\Windows\System32\shell32.dll";
        let first = resolver.display_name(path);
        let second = resolver.display_name(path);

        assert_eq!(first, second);
        assert_eq!(resolver.stats(), (1, 1), "one hit after one lookup");
    }

    #[test]
    fn windows_binaries_are_recognised_as_system_components() {
        assert!(is_system_component(r"C:\Windows\System32\shell32.dll"));
        assert!(is_system_component("notepad.exe"));
        assert!(!is_system_component(r"C:\Program Files\7-Zip\7-zip.dll"));
    }
}
