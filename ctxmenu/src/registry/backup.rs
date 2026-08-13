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
        self.covered.contains(&target.full_path().to_lowercase())
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
}

/// `%LOCALAPPDATA%\ctxmenu\backups`
pub fn root_dir() -> Result<PathBuf> {
    let base = dirs::data_local_dir().context("kein LOCALAPPDATA / no local data directory")?;
    Ok(base.join("ctxmenu").join("backups"))
}

/// Exports every target into a new timestamped directory.
///
/// Fails only if nothing at all could be exported — a partially existing
/// selection still yields a usable backup, with the gaps written into the
/// manifest.
pub fn export(action: &str, targets: &[RegTarget]) -> Result<BackupToken> {
    if targets.is_empty() {
        bail!("Backup ohne Ziele / backup with no targets");
    }

    let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S");
    // Colons are legal in ISO 8601 and illegal in Windows file names, hence
    // the compact form rather than the one the ToDo sketches.
    let directory = root_dir()?.join(format!("{stamp}_{}", sanitize(action)));
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("Backup-Verzeichnis / backup directory {directory:?}"))?;

    let mut entries = Vec::new();
    let mut missing = Vec::new();
    let mut covered = FxHashSet::default();

    for (index, target) in targets.iter().enumerate() {
        let full = target.full_path();
        let file_name = format!("{:02}_{}.reg", index + 1, sanitize(&full));
        let file = directory.join(&file_name);

        if run_reg(&["export", &full, &file.to_string_lossy(), "/y"])? {
            covered.insert(full.to_lowercase());
            entries.push(BackupEntry {
                registry_path: full,
                scope: target.scope.label().to_string(),
                file: file_name,
            });
        } else {
            missing.push(full);
        }
    }

    let manifest = BackupManifest {
        created_at: chrono::Local::now(),
        action: action.to_string(),
        entries,
        missing,
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
        bail!("Kein einziger Schlüssel konnte exportiert werden / nothing could be exported");
    }

    Ok(BackupToken { directory, covered })
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
            bail!("Backup unvollständig / backup incomplete: {file:?}");
        }
        if run_reg(&["import", &file.to_string_lossy()])? {
            restored += 1;
        } else {
            bail!("reg import fehlgeschlagen / failed for {file:?}");
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
fn run_reg(args: &[&str]) -> Result<bool> {
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

    let output = command
        .output()
        .with_context(|| format!("{exe:?} konnte nicht gestartet werden / could not be started"))?;

    Ok(output.status.success())
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
