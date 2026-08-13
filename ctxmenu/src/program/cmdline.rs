//! Working out which program a context menu entry actually belongs to.
//!
//! The registry stores a command line, not a program. Grouping "7-Zip" into
//! one row means turning
//!
//! ```text
//! "C:\Program Files\7-Zip\7zFM.exe" "%1"
//! rundll32.exe shell32.dll,Control_RunDLL foo.cpl
//! C:\Program Files\Tool\t.exe %1
//! ```
//!
//! into one identity each — including the last one, which has a space in its
//! path and no quotes to mark where the program name ends (ToDo 11.1).

use std::path::Path;

use crate::registry::mui;

/// Executables that run something else. For these, argv[0] is not the answer.
///
/// The comparison is on the file stem, so `powershell` and `powershell.exe`
/// and a fully qualified path all match.
const INTERPRETERS: [&str; 9] = [
    "rundll32",
    "regsvr32",
    "mshta",
    "cmd",
    "powershell",
    "pwsh",
    "wscript",
    "cscript",
    "explorer",
];

/// What a command line resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    /// First token, environment variables expanded.
    pub argv0: String,
    /// The program the entry belongs to. Equal to `argv0` unless an
    /// interpreter was seen and a real target could be found behind it.
    pub target: String,
    /// Set when `argv0` was an interpreter, so the interface can say
    /// "via powershell.exe" instead of pretending the entry is PowerShell.
    pub via_interpreter: Option<String>,
}

impl ParsedCommand {
    /// The grouping key for the program view: normalised target path.
    pub fn program_key(&self) -> String {
        normalize(&self.target)
    }
}

/// Parses a registry command line.
///
/// Returns `None` only for an empty command; anything else yields a best
/// effort, because an entry that cannot be attributed still has to appear
/// somewhere rather than vanish from the list.
pub fn parse(command: &str) -> Option<ParsedCommand> {
    let expanded = mui::expand_env(command.trim());
    if expanded.trim().is_empty() {
        return None;
    }

    let (argv0, rest) = split_argv0(&expanded);
    if argv0.is_empty() {
        return None;
    }

    let stem = file_stem(&argv0);
    if INTERPRETERS.contains(&stem.as_str()) {
        if let Some(target) = target_behind_interpreter(&stem, rest) {
            return Some(ParsedCommand {
                argv0: argv0.clone(),
                target,
                via_interpreter: Some(argv0),
            });
        }
        // An interpreter with no file argument — `powershell -command ...` —
        // really is its own program as far as grouping goes.
        return Some(ParsedCommand {
            argv0: argv0.clone(),
            target: argv0.clone(),
            via_interpreter: Some(argv0),
        });
    }

    Some(ParsedCommand {
        argv0: argv0.clone(),
        target: argv0,
        via_interpreter: None,
    })
}

/// Convenience for the scanner: the grouping key, or `None`.
pub fn program_key(command: &str) -> Option<String> {
    parse(command).map(|p| p.program_key())
}

/// Splits the first token off, following Windows quoting.
///
/// The unquoted case is the awkward one: `C:\Program Files\Tool\t.exe %1` has
/// no marker for where the program name ends, and splitting at the first space
/// yields `C:\Program`. The fix from ToDo 16 is to extend the candidate word
/// by word and ask the file system which prefix exists.
fn split_argv0(command: &str) -> (String, &str) {
    let trimmed = command.trim_start();

    if let Some(rest) = trimmed.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => (rest[..end].to_string(), &rest[end + 1..]),
            // Unbalanced quote: take everything, there is nothing better.
            None => (rest.to_string(), ""),
        };
    }

    // Longest prefix that actually exists on disk wins.
    let mut best: Option<usize> = None;
    let mut cursor = 0;
    while let Some(offset) = trimmed[cursor..].find(' ') {
        let end = cursor + offset;
        if Path::new(trimmed[..end].trim()).exists() {
            best = Some(end);
        }
        cursor = end + 1;
        if cursor >= trimmed.len() {
            break;
        }
    }
    // The whole string may itself be a path without arguments.
    if Path::new(trimmed.trim()).exists() {
        best = Some(trimmed.len());
    }

    match best {
        Some(end) => (
            trimmed[..end].trim().to_string(),
            &trimmed[end.min(trimmed.len())..],
        ),
        None => match trimmed.find(' ') {
            Some(end) => (trimmed[..end].to_string(), &trimmed[end..]),
            None => (trimmed.to_string(), ""),
        },
    }
}

/// Digs the real target out of the arguments of an interpreter.
fn target_behind_interpreter(stem: &str, rest: &str) -> Option<String> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }

    // rundll32 takes `<dll>,<entrypoint>`, and the DLL may be quoted.
    if stem == "rundll32" || stem == "regsvr32" {
        let (first, _) = split_argv0(rest);
        let dll = first.split(',').next().unwrap_or(&first).trim();
        return (!dll.is_empty()).then(|| dll.trim_matches('"').to_string());
    }

    // Everything else: the first argument that looks like a file rather than
    // a switch. `-File script.ps1` and `/c program.exe` both land here.
    for token in tokens(rest) {
        if token.starts_with('-') || token.starts_with('/') {
            continue;
        }
        let candidate = token.trim_matches('"');
        if candidate.is_empty() {
            continue;
        }
        if Path::new(candidate).exists() || has_program_extension(candidate) {
            return Some(candidate.to_string());
        }
    }

    None
}

/// Splits on spaces but keeps quoted runs together.
fn tokens(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in value.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn has_program_extension(value: &str) -> bool {
    let lower = value.to_lowercase();
    [
        ".exe", ".dll", ".cpl", ".ps1", ".vbs", ".js", ".bat", ".cmd", ".msc",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// Lowercased file name without extension.
fn file_stem(path: &str) -> String {
    Path::new(path.trim_matches('"'))
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Collapses the many spellings of one path into a single grouping key.
fn normalize(path: &str) -> String {
    crate::registry::clsid::normalize_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(command: &str) -> String {
        parse(command).expect("should parse").target
    }

    #[test]
    fn a_quoted_path_is_taken_whole() {
        assert_eq!(
            target(r#""C:\Program Files\7-Zip\7zFM.exe" "%1""#),
            r"C:\Program Files\7-Zip\7zFM.exe"
        );
    }

    #[test]
    fn an_unquoted_path_without_spaces_is_easy() {
        assert_eq!(target(r"C:\tools\t.exe %1"), r"C:\tools\t.exe");
    }

    #[test]
    fn an_unquoted_path_with_spaces_is_recovered_from_the_file_system() {
        // The documented hard case. Uses a file that exists on every Windows
        // installation and whose path contains a space.
        let real = r"C:\Program Files\Windows Defender\MpCmdRun.exe";
        if !Path::new(real).exists() {
            // Not present on every build; the pure-logic fallback is covered
            // by the test below.
            return;
        }
        assert_eq!(target(&format!("{real} -Scan")), real);
    }

    #[test]
    fn an_unquoted_path_with_spaces_that_does_not_exist_falls_back_to_the_first_word() {
        // Nothing on disk to confirm the guess, so the parser must not invent
        // one. Splitting at the first space is the honest fallback.
        let parsed = parse(r"C:\Nirgends Vorhanden\t.exe %1").expect("parses");
        assert_eq!(parsed.target, r"C:\Nirgends");
    }

    #[test]
    fn rundll32_points_at_the_dll_not_at_rundll32() {
        let parsed = parse("rundll32.exe shell32.dll,Control_RunDLL foo.cpl").expect("parses");
        assert_eq!(parsed.target, "shell32.dll");
        assert_eq!(parsed.via_interpreter.as_deref(), Some("rundll32.exe"));
    }

    #[test]
    fn rundll32_handles_a_quoted_dll_path() {
        let parsed =
            parse(r#"rundll32.exe "C:\Program Files\Tool\t.dll",Entry %1"#).expect("parses");
        assert_eq!(parsed.target, r"C:\Program Files\Tool\t.dll");
    }

    #[test]
    fn a_script_behind_an_interpreter_is_the_target() {
        let parsed = parse(r"wscript.exe C:\tools\convert.vbs %1").expect("parses");
        assert_eq!(parsed.target, r"C:\tools\convert.vbs");
        assert_eq!(parsed.via_interpreter.as_deref(), Some("wscript.exe"));

        let parsed = parse(r"powershell.exe -File C:\tools\do.ps1").expect("parses");
        assert_eq!(parsed.target, r"C:\tools\do.ps1");
    }

    #[test]
    fn an_interpreter_without_a_file_stays_itself() {
        // `powershell -command Set-Location ...` has no target file; calling
        // it "PowerShell" is then correct rather than a failure.
        let parsed = parse(r#"powershell.exe -noexit -command Set-Location -LiteralPath '%V'"#)
            .expect("parses");
        assert_eq!(parsed.target, "powershell.exe");
        assert_eq!(parsed.via_interpreter.as_deref(), Some("powershell.exe"));
    }

    #[test]
    fn environment_variables_are_expanded_before_anything_else() {
        let parsed = parse(r"%SystemRoot%\Explorer.exe").expect("parses");
        assert!(!parsed.target.contains('%'), "got {}", parsed.target);
        assert!(parsed.target.to_lowercase().ends_with(r"\explorer.exe"));
    }

    #[test]
    fn the_grouping_key_collapses_spelling_differences() {
        let a = parse(r#""C:\Program Files\7-Zip\7zFM.exe" "%1""#).expect("parses");
        let b = parse(r#""C:/Program Files/7-Zip/7zFM.EXE" %1"#).expect("parses");
        assert_eq!(a.program_key(), b.program_key());
        assert_eq!(a.program_key(), r"c:\program files\7-zip\7zfm.exe");
    }

    #[test]
    fn empty_commands_yield_nothing() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
    }

    #[test]
    fn quoted_runs_survive_tokenisation() {
        let t = tokens(r#"-File "C:\a b\c.ps1" -Extra"#);
        assert_eq!(t, vec!["-File", r#""C:\a b\c.ps1""#, "-Extra"]);
    }
}
