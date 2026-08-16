//! Asking Windows for a file, so a path does not have to be typed.
//!
//! `GetOpenFileNameW` from `comdlg32`, not the newer `IFileOpenDialog`: this
//! needs one call and no COM apartment of its own, and what it returns — a
//! path to an existing file — is the whole requirement. The newer interface
//! would buy places to put custom buttons that this form does not have.

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::core::{PCWSTR, PWSTR};

/// What the picker should offer, as pairs of (label, pattern).
///
/// Kept as a type rather than a string so a caller cannot forget the double
/// NUL terminator the API insists on — a filter that is off by one byte shows
/// an empty file type list and nothing else says why.
pub struct Filter<'a>(pub &'a [(&'a str, &'a str)]);

impl Filter<'_> {
    /// The `label\0pattern\0…\0\0` block the dialog wants.
    fn encode(&self) -> Vec<u16> {
        let mut out = Vec::new();
        for (label, pattern) in self.0 {
            // The labels below are written in both languages; the dialog shows
            // the one the window is showing.
            out.extend(crate::bilingual::shown(label).encode_utf16());
            out.push(0);
            out.extend(pattern.encode_utf16());
            out.push(0);
        }
        // The list itself ends with an extra NUL.
        out.push(0);
        out
    }
}

/// Programs, plus everything, for the command line field.
pub const PROGRAMS: Filter<'static> = Filter(&[
    ("\x1eProgramme\x1fPrograms\x1d", "*.exe;*.com;*.bat;*.cmd"),
    ("\x1eAlle Dateien\x1fAll files\x1d", "*.*"),
]);

/// Where icons live: resource carriers as well as icon files.
pub const ICONS: Filter<'static> = Filter(&[
    ("\x1eSymbole\x1fIcons\x1d", "*.ico;*.exe;*.dll"),
    ("\x1eAlle Dateien\x1fAll files\x1d", "*.*"),
]);

/// Opens the file picker. `None` means the user cancelled, which is an answer
/// and not a failure.
///
/// `start` seeds the dialog with whatever is already in the field, so editing
/// an existing path opens where that path is rather than in Documents.
pub fn pick_file(owner: Option<HWND>, filter: &Filter<'_>, start: &str) -> Option<PathBuf> {
    // MAX_PATH is not the limit here — the buffer size is what the API reads,
    // and a long path simply needs a long buffer.
    let mut buffer = vec![0u16; 4096];

    // A quoted command line is the normal content of the field this is called
    // from; the dialog wants a bare path.
    let seed = start.trim().trim_matches('"');
    if !seed.is_empty() && seed.len() < buffer.len() - 1 {
        for (slot, unit) in buffer.iter_mut().zip(seed.encode_utf16()) {
            *slot = unit;
        }
    }

    // Read-only for the dialog, hence `PCWSTR` — but it still has to outlive
    // the call, which is why it is a binding and not a temporary.
    let filters = filter.encode();

    let mut options = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner.unwrap_or_default(),
        lpstrFilter: PCWSTR(filters.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        // `NOCHANGEDIR` matters more than it looks: without it the dialog
        // moves the *process* working directory to wherever the user browsed,
        // and every later relative path in this program would follow along.
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    // Returns false both when the user cancelled and when something went
    // wrong. The difference is `CommDlgExtendedError`, and there is nothing
    // this program would do differently for either.
    let picked = unsafe { GetOpenFileNameW(&mut options) };
    if !picked.as_bool() {
        return None;
    }

    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let path = String::from_utf16_lossy(&buffer[..end]);
    (!path.trim().is_empty()).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_ends_in_the_double_nul_the_api_insists_on() {
        // Off by one byte here and the dialog shows an empty file type list
        // with nothing to say why.
        let encoded = Filter(&[("Programme", "*.exe")]).encode();
        let expected: Vec<u16> = "Programme\0*.exe\0\0".encode_utf16().collect();
        assert_eq!(encoded, expected);

        // Every shipped filter has to survive the same rule.
        for filter in [&PROGRAMS, &ICONS] {
            let encoded = filter.encode();
            assert_eq!(
                encoded[encoded.len() - 2..],
                [0, 0],
                "a filter list ends with two NULs"
            );
            assert!(encoded.len() > 4);
        }
    }
}
