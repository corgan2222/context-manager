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
    /// Lowercased full paths Windows answered "no such key" for.
    absent: FxHashSet<String>,
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

    /// Did the backup find nothing at this path?
    ///
    /// A key that does not exist yet has a state too — the empty one — and it
    /// is as restorable as any other, by removing the key again. Windows ships
    /// no blocked list, so on a machine where nothing was ever blocked this is
    /// the *only* answer a backup of it can give.
    ///
    /// Deliberately not folded into [`covers_path`]: a delete needs the
    /// contents of a key on disk, and "there was nothing here" is not that.
    /// The gate in [`super::write::delete_tree`] therefore stays exactly as
    /// strict as it was.
    pub fn records_absence(&self, path: impl AsRef<str>) -> bool {
        self.absent.contains(&path.as_ref().to_lowercase())
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
    /// The keys in `missing` that provably did not exist, and whose empty
    /// state a restore therefore puts back by *removing* them again.
    ///
    /// Kept apart from `missing` because the two are undone differently: an
    /// export that merely failed must be left alone, or a restore would delete
    /// the very key it could not save. Only [`export`] fills this in, never
    /// [`export_wide`] — see [`Absence`]. `serde(default)` so backups written
    /// before this field still load, and load as "remove nothing".
    #[serde(default)]
    pub absent: Vec<String>,
}

/// `%LOCALAPPDATA%\ctxmenu\backups`
pub fn root_dir() -> Result<PathBuf> {
    let base =
        dirs::data_local_dir().context("\x1ekein LOCALAPPDATA\x1fno local data directory\x1d")?;
    Ok(base.join("ctxmenu").join("backups"))
}

/// What a restore should do about a key that did not exist when the backup
/// was taken.
///
/// The distinction is about breadth, not about honesty: both kinds of backup
/// write the absence down. Only the narrow kind acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Absence {
    /// Put the empty state back by removing the key again.
    ///
    /// For a backup whose paths are exactly what is about to be changed. Those
    /// keys are this program's own doing — `Block` creates the blocked list,
    /// the editor creates an entry key — so undoing the change means taking
    /// them away again.
    Restorable,
    /// Write it down and leave it alone.
    ///
    /// For the whole-machine backup, whose paths are containers such as
    /// `Directory\shell`. Every program on the machine installs into those,
    /// and a restore months later must not carry off what somebody else put
    /// there in the meantime. The tooltip on the restore button promises
    /// exactly that, and it stays true.
    Noted,
}

impl Absence {
    /// What the manifest says about a key that was not there.
    ///
    /// Two wordings rather than one, because the note is the only place a
    /// reader of the backup can tell whether restoring it will remove the key.
    fn reason(self) -> &'static str {
        match self {
            Absence::Restorable => {
                "\x1eSchlüssel existiert nicht, der leere Ausgangszustand ist gesicher\
                 t\x1fkey does not exist, the empty starting state was recorded\x1d"
            }
            Absence::Noted => "\x1eSchlüssel existiert nicht\x1fkey does not exist\x1d",
        }
    }
}

/// Exports every target into a new timestamped directory.
///
/// Fails only if nothing at all could be captured — a partially existing
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
///
/// For the paths of one action, so a key that is not there yet is recorded as
/// an empty starting state and removed again on restore ([`Absence`]).
pub fn export(action: &str, paths: &[String]) -> Result<BackupToken> {
    export_with(action, paths, Absence::Restorable)
}

/// The same, for a net cast over whole containers rather than over one action.
///
/// Used by the "back up everything" button. What is missing here is missing
/// because this Windows never had it, not because this program is about to
/// create it, so the absence is written down and nothing acts on it.
pub fn export_wide(action: &str, paths: &[String]) -> Result<BackupToken> {
    export_with(action, paths, Absence::Noted)
}

fn export_with(action: &str, paths: &[String], absence: Absence) -> Result<BackupToken> {
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
    let mut empty = Vec::new();
    let mut covered = FxHashSet::default();
    let mut recorded_absent = FxHashSet::default();

    // Cut on the way in, not on display: a manifest outlives the run that
    // wrote it and is meant to be readable on its own, with an editor or with
    // `Get-Content`. Markers in a stored file are invisible control characters
    // in somebody else's tool.
    let readable =
        |reason: &str| crate::bilingual::pick(reason, crate::bilingual::language()).into_owned();

    for (index, full) in paths.iter().enumerate() {
        let file_name = format!("{:02}_{}.reg", index + 1, sanitize(full));
        let file = directory.join(&file_name);

        match export_one(full, &file)? {
            Captured::Exported => {
                covered.insert(full.to_lowercase());
                entries.push(BackupEntry {
                    registry_path: full.clone(),
                    scope: hive_of(full).to_string(),
                    file: file_name,
                });
            }
            Captured::Absent => {
                recorded_absent.insert(full.to_lowercase());
                missing.push(full.clone());
                notes.push(format!("{full}: {}", readable(absence.reason())));
                if absence == Absence::Restorable {
                    empty.push(full.clone());
                }
            }
            Captured::Failed(reason) => {
                missing.push(full.clone());
                notes.push(format!("{full}: {}", readable(&reason)));
            }
        }
    }

    let manifest = BackupManifest {
        created_at: chrono::Local::now(),
        action: action.to_string(),
        entries,
        missing,
        notes,
        absent: empty,
    };
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )
    .context("manifest.json")?;

    if manifest.entries.is_empty() && recorded_absent.is_empty() {
        // Leave no empty directory behind: a backup listing full of husks
        // makes the one backup that matters harder to find.
        let _ = std::fs::remove_dir_all(&directory);
        bail!(
            "\x1eKein einziger Schlüssel konnte gesichert werden\x1fnothing could be backed up\x1d"
        );
    }

    Ok(BackupToken {
        directory,
        covered,
        absent: recorded_absent,
    })
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
fn export_one(full: &str, file: &Path) -> Result<Captured> {
    if presence(full) == Presence::Absent {
        return Ok(Captured::Absent);
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
                return Ok(Captured::Exported);
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

    // A key that vanished between the check above and here ends up as a
    // failure rather than as an absence, which is the safe direction: a
    // failure is left alone on restore, an absence is removed.
    Ok(Captured::Failed(last.expect("at least one attempt")))
}

/// What a backup found at one path.
///
/// Three answers where there used to be two. "Nothing is here" and "I could
/// not read what is here" used to be the same `Some(reason)`, and treating
/// them alike is what kept a `Block` from ever running on a machine where
/// nothing had been blocked before: Windows ships no blocked list, so the
/// backup of it could only ever fail.
enum Captured {
    /// The contents are in the file the caller named.
    Exported,
    /// Windows says there is no such key.
    Absent,
    /// `reg.exe` refused, with this reason.
    Failed(String),
}

/// What is known about a path before `reg.exe` is asked.
#[derive(Debug, PartialEq, Eq)]
enum Presence {
    Present,
    /// Windows answered "no such key" — the one answer that lets a backup
    /// record an empty starting state.
    Absent,
    /// An unknown hive prefix, or any other complaint. Deliberately not
    /// "absent": a key whose ACL refuses this process is very much there, and
    /// calling it missing would make a restore delete it.
    Unknown,
}

/// `HRESULT` forms of the two Win32 codes that mean "it is not there".
///
/// `windows_registry` hands every `RegOpenKeyEx` failure back as
/// `HRESULT_FROM_WIN32`, so these are the raw codes 2 and 3 with the facility
/// bits in front.
const HRESULT_FILE_NOT_FOUND: u32 = 0x8007_0002;
const HRESULT_PATH_NOT_FOUND: u32 = 0x8007_0003;

fn presence(full: &str) -> Presence {
    let Some((hive, rest)) = full.split_once('\\') else {
        return Presence::Unknown;
    };

    let root = match hive.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => windows_registry::CURRENT_USER,
        "HKLM" | "HKEY_LOCAL_MACHINE" => windows_registry::LOCAL_MACHINE,
        "HKCR" | "HKEY_CLASSES_ROOT" => windows_registry::CLASSES_ROOT,
        _ => return Presence::Unknown,
    };

    match root.open(rest) {
        Ok(_) => Presence::Present,
        Err(error) => match error.code().0 as u32 {
            HRESULT_FILE_NOT_FOUND | HRESULT_PATH_NOT_FOUND => Presence::Absent,
            _ => Presence::Unknown,
        },
    }
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

/// What a restore managed to do.
///
/// A tally and a list of reasons rather than the first error. A backup of 43
/// keys whose 21st file was missing used to stop right there: twenty keys
/// back, twenty-two not, one file name in the caller's hand, and a second
/// click that failed in the same place. Every entry is attempted now.
#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    /// Keys brought back from a `.reg` file.
    pub restored: usize,
    /// Keys removed again because the backup records them as not existing.
    pub removed: usize,
    /// One line per key that did not come back, with the reason. Bilingual,
    /// so the caller cuts it for whoever is reading.
    pub failures: Vec<String>,
}

impl RestoreReport {
    pub fn failed(&self) -> usize {
        self.failures.len()
    }

    /// Folds in a second backup's outcome — a split action leaves two.
    pub fn merge(&mut self, other: RestoreReport) {
        self.restored += other.restored;
        self.removed += other.removed;
        self.failures.extend(other.failures);
    }
}

/// Puts a backup back, key by key, and reports what became of each one.
///
/// Known limitation, and the reason this is enough for the delete case but not
/// in general: `reg import` adds and overwrites, it never removes. Restoring
/// after a delete recreates exactly the removed keys; restoring over a key
/// that has since gained values leaves those extra values in place. The one
/// exception is `manifest.absent` — keys the backup found empty, which are put
/// back by removing them again.
pub fn restore(directory: &Path) -> Result<RestoreReport> {
    let manifest = read_manifest(directory)?;

    // Every file is looked for before the first import, not during. A gap
    // found halfway through is a half-restored registry; found here it has
    // cost nothing, and it is reported alongside everything that did work.
    let (present, gaps): (Vec<&BackupEntry>, Vec<&BackupEntry>) = manifest
        .entries
        .iter()
        .partition(|entry| directory.join(&entry.file).exists());

    // A backup that names keys and has not one of their files left is broken
    // rather than partly usable, and saying so here costs nothing: not a
    // single import has run yet.
    if !manifest.entries.is_empty() && present.is_empty() {
        bail!(
            "\x1eBackup unvollständig, keine einzige Datei ist noch d\
             a\x1fbackup incomplete, not one file is left\x1d: {directory:?}"
        );
    }

    let mut report = RestoreReport::default();

    for entry in gaps {
        report.failures.push(format!(
            "{}: \x1eDatei fehlt\x1ffile missing\x1d {}",
            entry.registry_path, entry.file
        ));
    }

    for entry in present {
        let file = directory.join(&entry.file);
        match run_reg(&["import", &file.to_string_lossy()])? {
            None => report.restored += 1,
            Some(reason) => report.failures.push(format!(
                "{}: \x1ereg import fehlgeschlagen\x1freg import failed\x1d: {reason}",
                entry.registry_path
            )),
        }
    }

    for path in &manifest.absent {
        // Still not there: the state the backup recorded is the state the
        // machine is in, and a restore that changes nothing is still a
        // restore. Only a key that appeared since has to go.
        if presence(path) == Presence::Absent {
            continue;
        }
        match run_reg(&["delete", path, "/f"])? {
            None => report.removed += 1,
            Some(reason) => report.failures.push(format!(
                "{path}: \x1eEntfernen fehlgeschlagen\x1fcould not remove\x1d: {reason}"
            )),
        }
    }

    Ok(report)
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
        assert_eq!(presence(r"HKCU\SOFTWARE"), Presence::Present);
        assert_eq!(
            presence(r"HKCU\SOFTWARE\ctxmenu_gibt_es_ganz_sicher_nicht"),
            Presence::Absent
        );

        // Unknown prefixes must fall through to the export attempt. Answering
        // "absent" for something merely unrecognised used to drop a key from a
        // backup; now it would also make a restore delete that key, so the
        // conservative answer matters twice over.
        assert_eq!(presence(r"HKXX\irgendwas"), Presence::Unknown);
        assert_eq!(presence("ohne_backslash"), Presence::Unknown);
    }

    #[test]
    fn exporting_nothing_is_an_error_rather_than_an_empty_backup() {
        assert!(export("noop", &[]).is_err());
    }

    /// A throwaway key of this tool's own, in the hive that needs no
    /// elevation. Same shape as the fixtures in the integration tests, and for
    /// the same reason: nothing a user would recognise may be touched.
    fn selftest_path(name: &str) -> (String, String) {
        let relative = format!(r"SOFTWARE\Classes\ctxmenu_selftest_backup_{name}");
        let full = format!(r"HKCU\{relative}");
        let _ = windows_registry::CURRENT_USER.remove_tree(&relative);
        (relative, full)
    }

    #[test]
    fn a_key_that_does_not_exist_yet_is_a_state_and_not_a_failed_export() {
        // The bug: `export` counted a missing key as a failure, so a backup of
        // nothing but missing keys produced no token at all -- and the blocked
        // list, which Windows does not ship, is exactly such a key.
        let (_, full) = selftest_path("absent");

        let token = export("selftest_absent", std::slice::from_ref(&full))
            .expect("an empty starting state is a state");

        assert!(!token.covers_path(&full), "there was nothing to export");
        assert!(token.records_absence(&full));

        let manifest = read_manifest(token.directory()).expect("manifest.json");
        assert!(manifest.entries.is_empty());
        assert_eq!(manifest.absent, vec![full.clone()]);
        assert_eq!(manifest.missing, vec![full]);

        // Nothing came back and nothing had to go: the machine is already in
        // the state the backup recorded.
        let report = restore(token.directory()).expect("a recorded absence is restorable");
        assert_eq!((report.restored, report.removed), (0, 0));
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        let _ = std::fs::remove_dir_all(token.directory());
    }

    #[test]
    fn restoring_an_empty_starting_state_removes_the_key_that_appeared_since() {
        // The other half of the promise. `reg import` never removes, so a
        // `Block` on a machine with no blocked list could be applied but not
        // undone -- the restore button would report success and change
        // nothing.
        let (relative, full) = selftest_path("removal");

        let token = export("selftest_removal", std::slice::from_ref(&full))
            .expect("an empty starting state is a state");

        windows_registry::CURRENT_USER
            .create(&relative)
            .expect("HKCU is writable")
            .set_string("Blocked", "")
            .expect("a value to find afterwards");

        let report = restore(token.directory()).expect("restore runs");
        assert_eq!(report.removed, 1, "{:?}", report.failures);
        assert_eq!(
            presence(&full),
            Presence::Absent,
            "the key must be gone again"
        );

        let _ = std::fs::remove_dir_all(token.directory());
        let _ = windows_registry::CURRENT_USER.remove_tree(&relative);
    }

    #[test]
    fn a_wide_backup_writes_an_absence_down_without_acting_on_it() {
        // `Directory\shell` in a hive that has none is missing because this
        // Windows never had it, not because this program is about to create
        // it. Removing such a container on restore would carry off whatever
        // every other program installed into it since.
        let (relative, full) = selftest_path("wide");

        let token = export_wide("selftest_wide", std::slice::from_ref(&full))
            .expect("a wide backup records the gap");
        assert!(token.records_absence(&full), "the token still knows");

        let manifest = read_manifest(token.directory()).expect("manifest.json");
        assert!(manifest.absent.is_empty(), "nothing to remove on restore");
        assert_eq!(manifest.missing, vec![full.clone()]);

        windows_registry::CURRENT_USER
            .create(&relative)
            .expect("HKCU is writable");
        let report = restore(token.directory()).expect("restore runs");
        assert_eq!((report.restored, report.removed), (0, 0));
        assert_eq!(presence(&full), Presence::Present, "left alone");

        let _ = std::fs::remove_dir_all(token.directory());
        let _ = windows_registry::CURRENT_USER.remove_tree(&relative);
    }

    #[test]
    fn a_restore_reports_every_gap_instead_of_stopping_at_the_first() {
        // Measured shape of the bug: the gap was found in the middle of the
        // loop, so the keys before it were back, the keys after it were not,
        // and the next attempt stopped in the same place. Here the gap is
        // first, which under the old code meant nothing was restored at all.
        let (relative, full) = selftest_path("gap");
        let second_relative = format!(r"{relative}\zweiter");
        let second_full = format!(r"{full}\zweiter");

        windows_registry::CURRENT_USER
            .create(&second_relative)
            .expect("HKCU is writable")
            .set_string("", "Selbsttest")
            .expect("default value");

        let token =
            export("selftest_gap", &[full.clone(), second_full.clone()]).expect("both keys exist");
        let manifest = read_manifest(token.directory()).expect("manifest.json");
        assert_eq!(manifest.entries.len(), 2);

        // Somebody removed a file from the backup directory -- a scanner, a
        // cleaner, a half-finished copy.
        std::fs::remove_file(token.directory().join(&manifest.entries[0].file))
            .expect("the first .reg file");
        let _ = windows_registry::CURRENT_USER.remove_tree(&relative);

        let report = restore(token.directory()).expect("a gap must not stop the rest");
        assert_eq!(report.restored, 1, "the intact half must come back");
        assert_eq!(report.failed(), 1);
        assert!(
            report.failures[0].contains(&manifest.entries[0].registry_path),
            "the report must name the key, got {:?}",
            report.failures
        );
        assert_eq!(
            presence(&second_full),
            Presence::Present,
            "the key behind the gap must be back"
        );

        let _ = std::fs::remove_dir_all(token.directory());
        let _ = windows_registry::CURRENT_USER.remove_tree(&relative);
    }

    #[test]
    fn the_backup_root_sits_under_local_appdata() {
        let root = root_dir().expect("LOCALAPPDATA exists on Windows");
        assert!(root.ends_with(r"ctxmenu\backups"), "got {root:?}");
    }
}
