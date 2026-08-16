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
//! path and no quotes to mark where the program name ends.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::registry::mui;

/// Executables that run something else. For these, argv[0] is not the answer.
///
/// The comparison is on the file stem, so `powershell` and `powershell.exe`
/// and a fully qualified path all match.
///
/// The list is grounded in a survey of 3.118 real command lines from this
/// machine, where 222 of them (7,1 %) went through one of these. Ranked by
/// how often they actually occurred: rundll32 89, explorer 46, cmd 28,
/// wscript 20, powershell 18, cscript 6, regsvr32 2, mshta 1, pwsh 1.
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

/// Extensions an argv[0] may be missing in the registry value.
///
/// A bare `…\tool` is meant as `…\tool.exe`; without completing the candidate,
/// the file system probe below cannot confirm the right prefix.
const EXECUTABLE_EXTENSIONS: [&str; 4] = [".exe", ".com", ".bat", ".cmd"];

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
    ///
    /// The extension is resolved here but deliberately *not* in `target`:
    /// the detail view should show what the registry actually says, while
    /// grouping has to collapse `…\tool` and `…\tool.exe` into one program.
    pub fn program_key(&self) -> String {
        normalize(&resolve_extension(&self.target))
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
/// yields `C:\Program`. The fix is to extend the candidate word by word and
/// ask the file system which prefix exists.
fn split_argv0(command: &str) -> (String, &str) {
    let trimmed = command.trim_start();

    if let Some(rest) = trimmed.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => (rest[..end].to_string(), &rest[end + 1..]),
            // Unbalanced quote: take everything, there is nothing better.
            None => (rest.to_string(), ""),
        };
    }

    // Longest prefix that resolves to an actual FILE wins.
    //
    // `exists()` would be wrong, and measurably so: on this machine
    // `C:\Program Files\Vectorworks 2025\Vectorworks 2025 Install Manager`
    // is an existing *directory* exactly one token short of the executable
    // with almost the same name. A directory is never a program.
    let mut best: Option<usize> = None;
    let mut cursor = 0;
    while let Some(offset) = trimmed[cursor..].find(' ') {
        let end = cursor + offset;
        if is_executable_file(trimmed[..end].trim()) {
            best = Some(end);
        }
        cursor = end + 1;
        if cursor >= trimmed.len() {
            break;
        }
    }
    // The whole string may itself be a path without arguments.
    if is_executable_file(trimmed.trim()) {
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

    // rundll32 takes `<dll>,<entrypoint>` as its FIRST argument, and the DLL
    // may be quoted.
    if stem == "rundll32" {
        let (first, _) = split_argv0(rest);
        let dll = first.split(',').next().unwrap_or(&first).trim();
        return (!dll.is_empty()).then(|| dll.trim_matches('"').to_string());
    }

    // regsvr32 is the other way round: the switches come first and the DLL is
    // the LAST argument. Observed on this machine as
    // `regsvr32.exe /n /i:"%1" scrobj.dll` under scriptletfile.
    if stem == "regsvr32" {
        return target_from_the_end(rest);
    }

    // Everything else: the first argument that looks like a file rather than
    // a switch. `-File script.ps1` and `/c program.exe` both land here.
    target_from_the_start(rest)
}

/// Searches from the front for the argument behind an interpreter that names
/// a real target — `cmd /c program.exe`, `wscript script.vbs`, and so on.
/// Skips switches and non-file words such as the `cd` in
/// `cmd /c cd /d "..." && "...\run.bat"` along the way.
///
/// An unquoted path with spaces was split into several tokens by [`tokens`],
/// with no quotes left to say where it ends. This recombines them the same
/// way `split_argv0` recombines argv[0]: extend one word at a time and
/// remember the longest prefix that is confirmed to exist as a FILE, never a
/// directory — `Path::exists()` cannot tell those apart. When nothing on disk
/// confirms anything (a registry entry can point at a program that was since
/// uninstalled), the longest prefix that merely *looks* like a path, by its
/// extension, is used instead of trusting a bare fragment.
fn target_from_the_start(rest: &str) -> Option<String> {
    let toks = tokens(rest);
    let mut i = 0;
    while i < toks.len() {
        let token = &toks[i];
        if token.starts_with('-') || token.starts_with('/') {
            i += 1;
            continue;
        }
        let trimmed = token.trim_matches('"');
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        if token.starts_with('"') {
            // Quoting already marks the boundary; nothing to recombine.
            if is_executable_file(trimmed) || has_program_extension(trimmed) {
                return Some(trimmed.to_string());
            }
            i += 1;
            continue;
        }

        let (best_confirmed, best_guess) = extend_forward(&toks, i);
        if let Some(end) = best_confirmed.or(best_guess) {
            return Some(toks[i..=end].join(" "));
        }
        i += 1;
    }
    None
}

/// Searches from the back for the DLL argument of `regsvr32`, which always
/// comes last, after its switches. Applies the same word-by-word
/// recombination as [`target_from_the_start`], just walking towards the front
/// instead of away from it: the same reversed search over the same tokens
/// would otherwise lose the same spaces.
fn target_from_the_end(rest: &str) -> Option<String> {
    let toks = tokens(rest);
    let mut i = toks.len();
    while i > 0 {
        i -= 1;
        let token = &toks[i];
        if token.starts_with('-') || token.starts_with('/') {
            continue;
        }
        let trimmed = token.trim_matches('"');
        if trimmed.is_empty() {
            continue;
        }

        if token.starts_with('"') {
            return Some(trimmed.to_string());
        }

        let (best_confirmed, best_guess) = extend_backward(&toks, i);
        return match best_confirmed.or(best_guess) {
            Some(start) => Some(toks[start..=i].join(" ")),
            // Nothing confirms or even suggests a longer prefix — trust the
            // bare last word, as before: a bare DLL name such as
            // "scrobj.dll" has no directory in it to probe.
            None => Some(trimmed.to_string()),
        };
    }
    None
}

/// Extends `toks[start]` forward through the following tokens for as long as
/// they are neither quoted nor switches, tracking the longest prefix that is
/// confirmed to exist and, separately, the longest prefix that only looks
/// like a path by its extension. Both are indices into `toks`, inclusive.
fn extend_forward(toks: &[String], start: usize) -> (Option<usize>, Option<usize>) {
    let mut best_confirmed = None;
    let mut best_guess = None;
    let mut candidate = toks[start].clone();
    update_best(&candidate, start, &mut best_confirmed, &mut best_guess);

    let mut end = start;
    while end + 1 < toks.len() {
        let next = &toks[end + 1];
        if next.starts_with('"') || next.starts_with('-') || next.starts_with('/') {
            break;
        }
        end += 1;
        candidate.push(' ');
        candidate.push_str(&toks[end]);
        update_best(&candidate, end, &mut best_confirmed, &mut best_guess);
    }
    (best_confirmed, best_guess)
}

/// The mirror image of [`extend_forward`], prepending instead of appending —
/// what `regsvr32`'s trailing DLL argument needs.
fn extend_backward(toks: &[String], start: usize) -> (Option<usize>, Option<usize>) {
    let mut best_confirmed = None;
    let mut best_guess = None;
    let mut candidate = toks[start].clone();
    update_best(&candidate, start, &mut best_confirmed, &mut best_guess);

    let mut begin = start;
    while begin > 0 {
        let prev = &toks[begin - 1];
        if prev.starts_with('"') || prev.starts_with('-') || prev.starts_with('/') {
            break;
        }
        begin -= 1;
        let prefix = &toks[begin];
        candidate = format!("{prefix} {candidate}");
        update_best(&candidate, begin, &mut best_confirmed, &mut best_guess);
    }
    (best_confirmed, best_guess)
}

/// Records `index` under whichever tier `candidate` qualifies for: confirmed
/// if it names a real file (never a directory, per [`is_executable_file`]),
/// guessed if it merely ends in a known program extension.
fn update_best(
    candidate: &str,
    index: usize,
    best_confirmed: &mut Option<usize>,
    best_guess: &mut Option<usize>,
) {
    if is_executable_file(candidate) {
        *best_confirmed = Some(index);
    } else if has_program_extension(candidate) {
        *best_guess = Some(index);
    }
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

/// Is this a real file, possibly after adding a missing executable extension?
///
/// Only ever used to *disambiguate* an unquoted head, never to reject an
/// already quoted path. That distinction matters: `C:\Program Files\
/// WindowsApps\…` is unreadable by ACL, so the probe answers "no" for files
/// that are demonstrably there. Treating that as "not a program" would drop
/// every Store app from the grouping.
fn is_executable_file(candidate: &str) -> bool {
    let candidate = candidate.trim().trim_matches('"');
    if candidate.is_empty() {
        return false;
    }
    if Path::new(candidate).is_file() {
        return true;
    }
    EXECUTABLE_EXTENSIONS
        .iter()
        .any(|ext| Path::new(&format!("{candidate}{ext}")).is_file())
}

/// Lowercased file name without extension.
fn file_stem(path: &str) -> String {
    Path::new(path.trim_matches('"'))
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Appends the executable extension the registry value left out, if any, and
/// resolves a bare program name against `PATH`.
///
/// Windows resolves `C:\tools\t` to `C:\tools\t.exe`; without doing the same,
/// two spellings of one program would become two groups. The `PATH` step is
/// the same argument one level up: the registry is allowed to say `attrib`
/// where it means `C:\Windows\System32\attrib.exe`, and a name without a
/// directory is a name the icon fallback cannot extract anything from
/// (measured on this machine: eight commands, among them `attrib` and
/// `compact`, had no icon for exactly this reason).
fn resolve_extension(path: &str) -> String {
    let trimmed = path.trim().trim_matches('"');
    if trimmed.is_empty() || Path::new(trimmed).is_file() {
        return trimmed.to_string();
    }
    for ext in EXECUTABLE_EXTENSIONS {
        let candidate = format!("{trimmed}{ext}");
        if Path::new(&candidate).is_file() {
            return candidate;
        }
    }
    if let Some(found) = resolve_on_path(trimmed) {
        return found;
    }
    trimmed.to_string()
}

/// Looks a bare program name up in the directories of `PATH`.
///
/// Deliberately restricted to names without a directory separator: anything
/// carrying a path is either right or wrong on its own terms, and searching
/// `PATH` for it would invent a second program with the same file name. A
/// relative path such as `.\tool.exe` is meaningless here anyway, because the
/// working directory of *this* process is not the one the verb will run in.
fn resolve_on_path(name: &str) -> Option<String> {
    if name.contains('\\') || name.contains('/') || name.contains(':') {
        return None;
    }

    let has_extension = Path::new(name).extension().is_some();
    for directory in path_directories() {
        if has_extension {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
            continue;
        }
        for ext in EXECUTABLE_EXTENSIONS {
            let candidate = directory.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// The directories of `PATH`, split and expanded once.
///
/// Once, because a full scan asks for this a few thousand times and `PATH` on
/// this machine has 40-odd entries — re-splitting it per command line turns a
/// lookup into a measurable part of the scan.
fn path_directories() -> &'static [PathBuf] {
    static DIRECTORIES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRECTORIES.get_or_init(|| {
        let Ok(value) = std::env::var("PATH") else {
            return Vec::new();
        };
        value
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| PathBuf::from(mui::expand_env(part.trim_matches('"'))))
            .collect()
    })
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

    // ------------------------------------------------------------------
    // Cases taken from a survey of 3.118 real command lines on this machine.
    // Invented examples had already led to two wrong implementations.
    // ------------------------------------------------------------------

    #[test]
    fn regsvr32_takes_the_dll_from_the_end_not_the_start() {
        // Real shape under HKCR\scriptletfile: the switches come first.
        // Treating it like rundll32 attributed every such entry to "/n".
        let parsed = parse(r#"regsvr32.exe /n /i:"%1" scrobj.dll"#).expect("parses");
        assert_eq!(parsed.target, "scrobj.dll");
        assert_eq!(parsed.via_interpreter.as_deref(), Some("regsvr32.exe"));
    }

    #[test]
    fn a_directory_one_token_short_does_not_win_over_the_executable() {
        // Measured on this machine: the directory
        // "C:\Program Files\Vectorworks 2025\Vectorworks 2025 Install Manager"
        // exists, and with Path::exists() it was a candidate for argv[0].
        // A directory is never a program.
        let dir = r"C:\Program Files";
        if !Path::new(dir).is_dir() {
            return;
        }
        let parsed = parse(&format!("{dir} /switch")).expect("parses");
        assert_ne!(
            parsed.target, dir,
            "an existing directory must not be accepted as the program"
        );
    }

    #[test]
    fn a_directory_behind_cmd_does_not_win_over_the_real_target() {
        // `cmd /c cd /d "install dir" && "install dir\run.exe"` is a common
        // shape, and the general branch used Path::exists() — which
        // answers "true" for the directory the `cd` changes into just as
        // readily as for a real program.
        let dir = r"C:\Program Files\Windows Defender";
        let exe = r"C:\Program Files\Windows Defender\MpCmdRun.exe";
        if !Path::new(exe).is_file() {
            return;
        }
        let command = format!(r#"cmd.exe /c cd /d "{dir}" && "{exe}" -Scan"#);
        let parsed = parse(&command).expect("parses");
        assert_eq!(parsed.target, exe);
        assert_eq!(parsed.via_interpreter.as_deref(), Some("cmd.exe"));
    }

    #[test]
    fn cmd_recombines_an_unquoted_target_with_spaces() {
        // `tokens()` splits an unquoted path at every space with no
        // quotes left to mark where it ends, so `"Files\Windows Defender\
        // MpCmdRun.exe"` — a fragment — used to win purely because it ends
        // in ".exe".
        let exe = r"C:\Program Files\Windows Defender\MpCmdRun.exe";
        if !Path::new(exe).is_file() {
            return;
        }
        let command = format!("cmd.exe /c {exe} %1");
        let parsed = parse(&command).expect("parses");
        assert_eq!(parsed.target, exe);
    }

    #[test]
    fn regsvr32_recombines_an_unquoted_dll_path_with_spaces() {
        // Same class of bug as the two tests above, hitting the reversed
        // search regsvr32 uses instead: the DLL is the LAST argument, and an
        // unquoted path with spaces still splits into fragments there.
        let dll = r"C:\Program Files\Windows Defender\MpClient.dll";
        if !Path::new(dll).is_file() {
            return;
        }
        let command = format!(r#"regsvr32.exe /n /i:"%1" {dll}"#);
        let parsed = parse(&command).expect("parses");
        assert_eq!(parsed.target, dll);
    }

    #[test]
    fn an_executable_without_its_extension_still_groups_with_the_full_name() {
        // Windows resolves `…\notepad` to `…\notepad.exe`; grouping has to
        // do the same or one program becomes two rows.
        let full = r"C:\Windows\System32\notepad.exe";
        if !Path::new(full).is_file() {
            return;
        }
        let without = r"C:\Windows\System32\notepad";
        assert_eq!(
            parse(without).expect("parses").program_key(),
            parse(full).expect("parses").program_key()
        );
    }

    #[test]
    fn quoted_paths_are_trusted_even_when_unreadable() {
        // C:\Program Files\WindowsApps is ACL-protected, so is_file() answers
        // "no" for files that are demonstrably there. A quoted argv[0] must
        // never be rejected on that basis, or every Store app disappears.
        let store = r#""C:\Program Files\WindowsApps\Some.App_1.0_x64__abc\app.exe" "%1""#;
        let parsed = parse(store).expect("parses");
        assert_eq!(
            parsed.target,
            r"C:\Program Files\WindowsApps\Some.App_1.0_x64__abc\app.exe"
        );
    }

    #[test]
    fn a_bare_command_name_is_resolved_against_path() {
        // The eight icon-less rows from the measurement: `attrib %1` names a
        // program that exists, just not where the string says. Without the
        // PATH step the icon fallback gets `attrib,0` and extracts nothing.
        let resolved = resolve_extension("attrib");
        assert!(
            resolved.to_lowercase().ends_with(r"\attrib.exe"),
            "expected a full path, got {resolved}"
        );
        assert!(Path::new(&resolved).is_file(), "{resolved} must exist");
    }

    #[test]
    fn a_bare_name_that_already_has_its_extension_is_resolved_too() {
        // The rundll32 case: the target is `shell32.dll`, a name with an
        // extension but no directory.
        let resolved = resolve_extension("shell32.dll");
        assert!(
            resolved.to_lowercase().ends_with(r"\shell32.dll"),
            "expected a full path, got {resolved}"
        );
    }

    #[test]
    fn a_name_carrying_a_directory_is_never_searched_on_path() {
        // `C:\Nirgends\attrib.exe` must stay wrong rather than silently become
        // the `attrib.exe` from System32 — that would attribute an entry to a
        // program it has nothing to do with.
        let wrong = r"C:\Nirgends\attrib.exe";
        assert_eq!(resolve_extension(wrong), wrong);
        assert_eq!(resolve_on_path(wrong), None);
        assert_eq!(resolve_on_path(r".\attrib"), None);
    }

    #[test]
    fn a_name_that_is_nowhere_on_path_stays_as_it_was() {
        assert_eq!(
            resolve_extension("gibtesnichtaufdiesermaschine"),
            "gibtesnichtaufdiesermaschine"
        );
    }

    #[test]
    fn path_resolution_collapses_the_bare_and_the_full_spelling() {
        // The grouping consequence: `attrib` and the full path are one program.
        let bare = parse("attrib +r %1").expect("parses");
        let full = parse(r"C:\Windows\System32\attrib.exe +r %1").expect("parses");
        if !Path::new(r"C:\Windows\System32\attrib.exe").is_file() {
            return;
        }
        assert_eq!(bare.program_key(), full.program_key());
    }

    #[test]
    fn quoted_runs_survive_tokenisation() {
        let t = tokens(r#"-File "C:\a b\c.ps1" -Extra"#);
        assert_eq!(t, vec!["-File", r#""C:\a b\c.ps1""#, "-Extra"]);
    }
}
