//! Parsing icon references out of the registry.
//!
//! Three shapes occur in the wild (ToDo 7.1):
//!
//! ```text
//! C:\pfad\datei.exe,3
//! C:\pfad\icon.ico
//! %SystemRoot%\system32\shell32.dll,-244
//! ```
//!
//! Everything here is pure string handling, deliberately: it is the part that
//! is easy to get subtly wrong and trivial to test, and keeping it away from
//! the Win32 extraction means the awkward cases can be covered without a
//! single GDI object.

use crate::registry::mui;

/// A resolved reference to one icon inside a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IconRef {
    /// Absolute path with environment variables expanded.
    pub path: String,
    /// Positive: index within the file. Negative: a resource ID.
    ///
    /// Both `ExtractIconExW` and `SHDefExtractIconW` want the negative value
    /// passed through unchanged, so this is deliberately not normalised.
    pub index: i32,
}

impl IconRef {
    /// Cache key. Registry paths are case-insensitive, so two references that
    /// differ only in capitalisation must share one extracted texture.
    pub fn cache_key(&self) -> String {
        format!("{}|{}", self.path.to_lowercase(), self.index)
    }

    /// Is this a resource ID rather than a positional index?
    pub fn is_resource_id(&self) -> bool {
        self.index < 0
    }
}

/// Parses a raw `Icon` value.
///
/// Returns `None` for anything without a usable path — an empty value or a
/// lone index is not worth a failed extraction attempt later.
pub fn parse(raw: &str) -> Option<IconRef> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (path_part, index) = split_index(trimmed);
    let path = unquote(path_part);
    if path.is_empty() {
        return None;
    }

    Some(IconRef {
        path: mui::expand_env(&path),
        index,
    })
}

/// Splits a trailing `,<number>` off, if there is one.
///
/// The number must parse as `i32` before the tail is treated as an index.
/// Without that check, `C:\dir\my,file.ico` would lose half its name — file
/// names containing commas are unusual but entirely legal.
fn split_index(value: &str) -> (&str, i32) {
    // A quoted path may itself contain commas; only look after the closing
    // quote. `"C:\Program Files\A,B\t.exe",2` is a real shape.
    let search_from = match value.strip_prefix('"') {
        Some(rest) => match rest.find('"') {
            // One byte for the opening quote, one for the closing one.
            Some(offset) => offset + 2,
            // Unbalanced quote: treat the whole thing as a path.
            None => return (value, 0),
        },
        None => 0,
    };

    let Some(comma) = value[search_from..].rfind(',') else {
        return (value, 0);
    };
    let comma = search_from + comma;

    let (head, tail) = value.split_at(comma);
    let tail = tail[1..].trim();

    match tail.parse::<i32>() {
        Ok(index) => (head, index),
        // Not a number, so the comma belongs to the path.
        Err(_) => (value, 0),
    }
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(raw: &str) -> (String, i32) {
        let r = parse(raw).expect("should parse");
        (r.path, r.index)
    }

    #[test]
    fn a_plain_path_has_index_zero() {
        assert_eq!(
            parsed(r"C:\pfad\icon.ico"),
            (r"C:\pfad\icon.ico".to_string(), 0)
        );
    }

    #[test]
    fn a_positive_index_is_read() {
        assert_eq!(
            parsed(r"C:\pfad\datei.exe,3"),
            (r"C:\pfad\datei.exe".to_string(), 3)
        );
    }

    #[test]
    fn a_negative_index_stays_negative() {
        // It is a resource ID, and the extraction APIs want it unchanged.
        let r = parse(r"C:\windows\system32\shell32.dll,-244").expect("parses");
        assert_eq!(r.index, -244);
        assert!(r.is_resource_id());
    }

    #[test]
    fn environment_variables_are_expanded() {
        let r = parse(r"%SystemRoot%\system32\shell32.dll,-244").expect("parses");
        assert!(!r.path.contains('%'), "unexpanded: {}", r.path);
        assert!(r.path.to_lowercase().ends_with(r"\system32\shell32.dll"));
        assert_eq!(r.index, -244);
    }

    #[test]
    fn quotes_are_stripped_even_with_an_index_behind_them() {
        assert_eq!(
            parsed(r#""C:\Program Files\Tool\t.exe",0"#),
            (r"C:\Program Files\Tool\t.exe".to_string(), 0)
        );
        assert_eq!(
            parsed(r#""C:\Program Files\Tool\t.exe""#),
            (r"C:\Program Files\Tool\t.exe".to_string(), 0)
        );
    }

    #[test]
    fn a_comma_inside_the_file_name_is_not_an_index() {
        assert_eq!(
            parsed(r"C:\dir\my,file.ico"),
            (r"C:\dir\my,file.ico".to_string(), 0)
        );
    }

    #[test]
    fn a_comma_inside_a_quoted_path_is_not_an_index() {
        assert_eq!(
            parsed(r#""C:\Program Files\A,B\t.exe",2"#),
            (r"C:\Program Files\A,B\t.exe".to_string(), 2)
        );
    }

    #[test]
    fn whitespace_around_the_index_is_tolerated() {
        assert_eq!(
            parsed(r"C:\pfad\datei.exe , 12 "),
            (r"C:\pfad\datei.exe".to_string(), 12)
        );
    }

    #[test]
    fn unusable_values_yield_nothing() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
        assert_eq!(parse(",5"), None);
        assert_eq!(parse(r#""""#), None);
    }

    #[test]
    fn the_cache_key_ignores_capitalisation_but_not_the_index() {
        let a = parse(r"C:\Windows\System32\SHELL32.dll,-244").expect("parses");
        let b = parse(r"c:\windows\system32\shell32.dll,-244").expect("parses");
        let c = parse(r"C:\Windows\System32\shell32.dll,-245").expect("parses");

        assert_eq!(a.cache_key(), b.cache_key());
        assert_ne!(a.cache_key(), c.cache_key());
    }
}
