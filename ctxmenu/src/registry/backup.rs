//! Backup and restore of registry subtrees via `reg.exe`.
//!
//! `reg.exe` rather than a hand-written exporter: it ships with every Windows,
//! and it already handles binary values, embedded newlines and the UTF-16
//! encoding of `.reg` files correctly. Getting that wrong silently is a class
//! of bug that only shows up when a restore is actually needed.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use super::paths::RegTarget;

/// Keeps `reg.exe` from flashing a console window (ToDo 13.1).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How often an operation that lost a race with a file handle is retried.
///
/// Both retrying places here fight the same opponent — a scanner holding a
/// file or directory that was created microseconds ago — so they get the same
/// budget. The export used to stop after three attempts and 240 ms while
/// directory creation persisted for two seconds, and there was never a reason
/// for the export to give up first.
const RETRY_ATTEMPTS: usize = 20;

/// Base step of the backoff. Attempt `n` waits `STEP * (n + 1)`, so twenty
/// attempts spread over about two seconds rather than hammering.
const RETRY_STEP: std::time::Duration = std::time::Duration::from_millis(10);

/// How long to wait before attempt `next`, or `None` when none follows.
///
/// Pure so the total budget is asserted in a test instead of estimated in a
/// comment — and so the last attempt provably does not sleep. Waiting after
/// the final try is time added to a failure the caller is already waiting for.
fn retry_delay(
    next: usize,
    limit: usize,
    step: std::time::Duration,
) -> Option<std::time::Duration> {
    (next < limit).then(|| step * next as u32)
}

/// Proof that specific keys were exported before anything was changed.
///
/// The only way to obtain one is [`export`]. [`super::write::delete_tree`]
/// demands one that covers the key being deleted, so "never delete without a
/// backup" is checked by the compiler and the type, not by discipline.
#[derive(Debug, Clone)]
pub struct BackupToken {
    directory: PathBuf,
    /// Lowercased full paths that were successfully exported.
    covered: FxHashSet<String>,
}

impl BackupToken {
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn covers(&self, target: &RegTarget) -> bool {
        self.covers_path(target.full_path())
    }

    /// For locations outside the classes tree — the blocked list lives under
    /// `HKLM\SOFTWARE\Microsoft\…` and deliberately cannot be a `RegTarget`.
    pub fn covers_path(&self, path: impl AsRef<str>) -> bool {
        self.covered.contains(&path.as_ref().to_lowercase())
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct BackupEntry {
    pub registry_path: String,
    pub scope: String,
    pub file: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub created_at: chrono::DateTime<chrono::Local>,
    pub action: String,
    pub entries: Vec<BackupEntry>,
    /// Keys that were requested but did not exist. Recorded rather than
    /// dropped: on restore this is the difference between "nothing to bring
    /// back" and "the export silently failed".
    pub missing: Vec<String>,
    /// What `reg.exe` said about each entry in `missing`, one line each.
    ///
    /// A gap in a backup used to be visible but not explicable, which turned a
    /// rare failure into guesswork. `serde(default)` so backups written before
    /// this field still load.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// `%LOCALAPPDATA%\ctxmenu\backups`
pub fn root_dir() -> Result<PathBuf> {
    let base =
        dirs::data_local_dir().context("\x1ekein LOCALAPPDATA\x1fno local data directory\x1d")?;
    Ok(base.join("ctxmenu").join("backups"))
}

/// Exports every target into a new timestamped directory.
///
/// Fails only if nothing at all could be exported — a partially existing
/// selection still yields a usable backup, with the gaps written into the
/// manifest.
pub fn export_targets(action: &str, targets: &[RegTarget]) -> Result<BackupToken> {
    let paths: Vec<String> = targets.iter().map(RegTarget::full_path).collect();
    export(action, &paths)
}

/// Exports arbitrary registry paths in `reg.exe` notation.
///
/// Takes plain paths rather than [`RegTarget`]s because the blocked list sits
/// outside the classes tree. The safety property is unchanged: the token
/// still records exactly which paths were captured, and the write functions
/// still refuse anything the token does not name.
pub fn export(action: &str, paths: &[String]) -> Result<BackupToken> {
    if paths.is_empty() {
        bail!("\x1eBackup ohne Ziele\x1fbackup with no targets\x1d");
    }

    // Colons are legal in ISO 8601 and illegal in Windows file names, hence
    // the compact form rather than the one the ToDo sketches. Milliseconds
    // because two actions a second apart is entirely normal.
    let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S%3f");
    let directory = unique_directory(&root_dir()?, &format!("{stamp}_{}", sanitize(action)))?;

    let mut entries = Vec::new();
    let mut missing = Vec::new();
    let mut notes = Vec::new();
    let mut covered = FxHashSet::default();

    for (index, full) in paths.iter().enumerate() {
        let file_name = format!("{:02}_{}.reg", index + 1, sanitize(full));
        let file = directory.join(&file_name);

        match export_one(full, &file)? {
            None => {
                covered.insert(full.to_lowercase());
                entries.push(BackupEntry {
                    registry_path: full.clone(),
                    scope: hive_of(full).to_string(),
                    file: file_name,
                });
            }
            Some(reason) => {
                missing.push(full.clone());
                // Cut here, not on display: a manifest outlives the run that
                // wrote it and is meant to be readable on its own, with an
                // editor or with `Get-Content`. Markers in a stored file are
                // invisible control characters in somebody else's tool.
                let reason = crate::bilingual::pick(&reason, crate::bilingual::language());
                notes.push(format!("{full}: {reason}"));
            }
        }
    }

    let manifest = BackupManifest {
        created_at: chrono::Local::now(),
        action: action.to_string(),
        entries,
        missing,
        notes,
    };
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )
    .context("manifest.json")?;

    if manifest.entries.is_empty() {
        // Leave no empty directory behind: a backup listing full of husks
        // makes the one backup that matters harder to find.
        let _ = std::fs::remove_dir_all(&directory);
        bail!(
            "\x1eKein einziger Schlüssel konnte exportiert werden\x1fnothing could be exported\x1d"
        );
    }

    Ok(BackupToken { directory, covered })
}

/// Claims a directory nobody else holds.
///
/// Uses `create_dir` rather than `create_dir_all` on purpose: the former fails
/// when the directory already exists, which is exactly the collision that has
/// to be detected. Two group actions in the same millisecond would otherwise
/// share a directory and quietly overwrite each other's `.reg` files —
/// destroying the backup that the second action is about to rely on.
fn unique_directory(root: &Path, base: &str) -> Result<PathBuf> {
    create_root(root)?;

    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            root.join(base)
        } else {
            root.join(format!("{base}_{suffix}"))
        };
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            // Taken, or taken and being deleted right now: on Windows a
            // directory whose last handle is still open keeps its name and
            // answers ACCESS_DENIED. Either way the next name is the answer.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(anyhow::Error::from(error).context(format!(
                    "\x1eBackup-Verzeichnis\x1fbackup directory\x1d {candidate:?}"
                )));
            }
        }
    }

    bail!("\x1eKein freier Backup-Verzeichnisname\x1fno free backup directory name\x1d")
}

/// Exports one key, trying again when the *file* could not be written.
///
/// Measured, not assumed. In a long test series `reg.exe` failed for one key
/// out of five while the other four in the same directory went through, with
/// "Die Datei kann nicht geschrieben werden. Es ist möglicherweise ein
/// Datenträger- bzw. Dateisystemfehler aufgetreten." — the registry key was
/// fine, the freshly created file was not. A virus scanner reading the
/// directory that was created microseconds earlier is the obvious candidate.
///
/// Why this matters beyond a flaky test: the missing key gets no backup, and
/// a step without a backup is refused. A group action would silently do
/// nine of ten things. That is precisely what the backup exists to prevent.
///
/// A key that genuinely does not exist is answered before the first attempt,
/// so the retries do not delay the case they cannot help.
fn export_one(full: &str, file: &Path) -> Result<Option<String>> {
    if key_is_absent(full) {
        return Ok(Some(
            "\x1eSchlüssel existiert nicht\x1fkey does not exist\x1d".to_string(),
        ));
    }

    let mut last = None;
    for attempt in 0..RETRY_ATTEMPTS {
        match run_reg(&["export", full, &file.to_string_lossy(), "/y"])? {
            None => {
                // Reported, not silently swallowed: how often this happens and
                // on which attempt it clears is the only way the number above
                // stops being a guess. Silence here means it never retried.
                if attempt > 0 {
                    crate::errln!(
                        "backup_export_retry: succeeded on attempt {} for {full}",
                        attempt + 1
                    );
                }
                return Ok(None);
            }
            Some(reason) => {
                last = Some(reason);
                match retry_delay(attempt + 1, RETRY_ATTEMPTS, RETRY_STEP) {
                    Some(delay) => std::thread::sleep(delay),
                    None => break,
                }
            }
        }
    }

    Ok(last)
}

/// Whether this path is *known* to name no key.
///
/// Deliberately conservative: an unknown hive prefix answers `false`, so the
/// export is attempted rather than skipped. Getting this wrong in that
/// direction costs one `reg.exe` call; the other direction would quietly drop
/// a key from a backup.
fn key_is_absent(full: &str) -> bool {
    let Some((hive, rest)) = full.split_once('\\') else {
        return false;
    };

    let root = match hive.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => windows_registry::CURRENT_USER,
        "HKLM" | "HKEY_LOCAL_MACHINE" => windows_registry::LOCAL_MACHINE,
        "HKCR" | "HKEY_CLASSES_ROOT" => windows_registry::CLASSES_ROOT,
        _ => return false,
    };

    root.open(rest).is_err()
}

/// Creates the backup root, tolerating a deletion Windows has not finished.
///
/// Measured, not guessed: `create_dir_all` immediately after `remove_dir_all`
/// of the same path fails with ACCESS_DENIED often enough to break a test run
/// on this machine. Windows keeps a deleted directory's name until the last
/// handle to anything inside it closes, and a virus scanner reading the freshly
/// written `.reg` files is exactly such a handle. Waiting a few milliseconds
/// costs nothing and turns a lost backup into a slightly later one.
fn create_root(root: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..RETRY_ATTEMPTS {
        match std::fs::create_dir_all(root) {
            Ok(()) => {
                if attempt > 0 {
                    crate::errln!("backup_root_retry: succeeded on attempt {}", attempt + 1);
                }
                return Ok(());
            }
            Err(error) => {
                last = Some(error);
                match retry_delay(attempt + 1, RETRY_ATTEMPTS, RETRY_STEP) {
                    Some(delay) => std::thread::sleep(delay),
                    None => break,
                }
            }
        }
    }

    Err(anyhow::Error::from(last.expect("at least one attempt"))
        .context(format!("\x1eBackup-Wurzel\x1fbackup root\x1d {root:?}")))
}

/// Hive prefix of a full path, for the manifest.
fn hive_of(path: &str) -> &str {
    path.split('\\').next().unwrap_or("?")
}

/// Reads the manifest of a backup directory.
pub fn read_manifest(directory: &Path) -> Result<BackupManifest> {
    let raw = std::fs::read_to_string(directory.join("manifest.json"))
        .with_context(|| format!("manifest.json in {directory:?}"))?;
    Ok(serde_json::from_str(&raw)?)
}

/// Re-imports every `.reg` file of a backup.
///
/// Known limitation, and the reason this is enough for the delete case but not
/// in general: `reg import` adds and overwrites, it never removes. Restoring
/// after a delete recreates exactly the removed keys; restoring over a key
/// that has since gained values leaves those extra values in place.
pub fn restore(directory: &Path) -> Result<usize> {
    let manifest = read_manifest(directory)?;
    let mut restored = 0;

    for entry in &manifest.entries {
        let file = directory.join(&entry.file);
        if !file.exists() {
            bail!("\x1eBackup unvollständig\x1fbackup incomplete\x1d: {file:?}");
        }
        match run_reg(&["import", &file.to_string_lossy()])? {
            None => restored += 1,
            Some(reason) => {
                bail!("\x1ereg import fehlgeschlagen\x1ffailed for\x1d {file:?}: {reason}")
            }
        }
    }

    Ok(restored)
}

/// Lists backup directories, newest first.
pub fn list() -> Result<Vec<(PathBuf, BackupManifest)>> {
    let root = root_dir()?;
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut found: Vec<(PathBuf, BackupManifest)> = std::fs::read_dir(&root)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| read_manifest(&path).ok().map(|m| (path, m)))
        .collect();

    found.sort_by_key(|(_, manifest)| std::cmp::Reverse(manifest.created_at));
    Ok(found)
}

/// Runs `reg.exe`, returning whether it reported success.
///
/// Called by absolute path: relying on `PATH` would let a stray `reg.exe` in
/// the working directory take over an operation that edits the registry.
fn run_reg(args: &[&str]) -> Result<Option<String>> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt as _;

    let exe = std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("reg.exe");

    let mut command = Command::new(&exe);
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command.output().with_context(|| {
        format!("{exe:?} \x1ekonnte nicht gestartet werden\x1fcould not be started\x1d")
    })?;

    if output.status.success() {
        return Ok(None);
    }

    // reg.exe writes its complaint to stdout on some Windows versions and to
    // stderr on others, so both are consulted before falling back to the code.
    let said = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().replace('\n', " ");
    let message = match (said(&output.stdout), said(&output.stderr)) {
        (out, err) if !err.is_empty() => {
            if out.is_empty() {
                err
            } else {
                format!("{err} {out}")
            }
        }
        (out, _) if !out.is_empty() => out,
        _ => format!("Exit-Code {:?}", output.status.code()),
    };
    Ok(Some(message))
}

/// Turns a registry path into something a file system accepts.
fn sanitize(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect();

    // Long class paths would otherwise blow past the file name limit; the
    // numeric prefix added by the caller keeps names unique regardless.
    let trimmed = cleaned.trim_matches('_');
    trimmed
        .chars()
        .rev()
        .take(120)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_attempt_does_not_sleep() {
        // The bug this replaced: three attempts, three sleeps. The third one
        // was 120 ms of pure delay in front of a failure the caller was
        // already waiting for.
        assert_eq!(
            retry_delay(RETRY_ATTEMPTS, RETRY_ATTEMPTS, RETRY_STEP),
            None
        );
        assert!(retry_delay(RETRY_ATTEMPTS - 1, RETRY_ATTEMPTS, RETRY_STEP).is_some());
    }

    #[test]
    fn the_retry_budget_is_about_two_seconds() {
        // Long enough to outlast a scanner holding a freshly written file,
        // short enough that a genuinely broken export still answers. Asserted
        // rather than described, because the two constants are easy to change
        // without noticing what they add up to.
        let total: std::time::Duration = (0..RETRY_ATTEMPTS)
            .filter_map(|attempt| retry_delay(attempt + 1, RETRY_ATTEMPTS, RETRY_STEP))
            .sum();

        assert!(
            (1900..=2200).contains(&total.as_millis()),
            "retry budget is {} ms",
            total.as_millis()
        );
    }

    #[test]
    fn the_backoff_grows_instead_of_hammering() {
        let first = retry_delay(1, RETRY_ATTEMPTS, RETRY_STEP).expect("a second attempt follows");
        let later = retry_delay(10, RETRY_ATTEMPTS, RETRY_STEP).expect("an eleventh follows");
        assert!(later > first, "{later:?} should be longer than {first:?}");
    }

    #[test]
    fn sanitising_keeps_names_recognisable_and_legal() {
        assert_eq!(
            sanitize(r"HKCU\SOFTWARE\Classes\Directory\shell\cmd"),
            "HKCU_SOFTWARE_Classes_Directory_shell_cmd"
        );
        assert_eq!(sanitize("Ümläut & Leerzeichen"), "ml_ut___Leerzeichen");
        for c in sanitize(r"a<b>c:d\e/f|g?h*i").chars() {
            assert!(c.is_ascii_alphanumeric() || c == '_' || c == '-');
        }
    }

    #[test]
    fn very_long_paths_stay_within_the_file_name_limit() {
        let long = format!(r"HKCU\SOFTWARE\Classes\{}", "a".repeat(500));
        let name = sanitize(&long);
        assert!(name.len() <= 120, "got {} characters", name.len());
        assert!(name.ends_with('a'), "the tail is the distinguishing part");
    }

    /// Regression guard: the directory name used to be second-resolution plus
    /// the action label, so two group actions in the same second shared a
    /// directory and the second overwrote the first one's backup.
    #[test]
    fn two_backups_of_the_same_action_never_share_a_directory() {
        let root = std::env::temp_dir().join("ctxmenu_backup_name_test");
        let _ = std::fs::remove_dir_all(&root);

        let a = unique_directory(&root, "stamp_aktion").expect("first");
        let b = unique_directory(&root, "stamp_aktion").expect("second");
        let c = unique_directory(&root, "stamp_aktion").expect("third");

        assert_ne!(a, b);
        assert_ne!(b, c);
        assert!(a.is_dir() && b.is_dir() && c.is_dir());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_absent_key_is_recognised_without_asking_reg_exe() {
        assert!(!key_is_absent(r"HKCU\SOFTWARE"), "this one exists");
        assert!(key_is_absent(
            r"HKCU\SOFTWARE\ctxmenu_gibt_es_ganz_sicher_nicht"
        ));

        // Unknown prefixes must fall through to the export attempt: answering
        // "absent" for something merely unrecognised would drop a key from a
        // backup without anyone noticing.
        assert!(!key_is_absent("HKXX\\irgendwas"));
        assert!(!key_is_absent("ohne_backslash"));
    }

    #[test]
    fn exporting_nothing_is_an_error_rather_than_an_empty_backup() {
        assert!(export("noop", &[]).is_err());
    }

    #[test]
    fn the_backup_root_sits_under_local_appdata() {
        let root = root_dir().expect("LOCALAPPDATA exists on Windows");
        assert!(root.ends_with(r"ctxmenu\backups"), "got {root:?}");
    }
}
