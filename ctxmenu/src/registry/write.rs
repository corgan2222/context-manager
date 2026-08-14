//! Changing the registry.
//!
//! Every function here is deliberately hard to misuse. The failure mode this
//! guards against is not a wrong error message — it is an unusable Explorer
//! after a wrong key was removed, which is only noticed at the next
//! right-click and cannot be undone from memory.

use anyhow::{Context as _, Result, bail};
use windows_registry::LOCAL_MACHINE;

use super::backup::BackupToken;
use super::paths::{self, RegTarget};

/// Deletes a key and everything below it.
///
/// Requires a [`BackupToken`] that covers exactly this target. The token can
/// only come from [`super::backup::export`], so a delete without a preceding,
/// successful backup of this very key cannot be written down — which is what
/// ToDo 13.1 asks for, enforced rather than documented.
pub fn delete_tree(target: &RegTarget, token: &BackupToken) -> Result<()> {
    if !token.covers(target) {
        bail!(
            "Backup deckt diesen Schlüssel nicht ab / backup does not cover this key: {}",
            target.full_path()
        );
    }

    paths::root_key(target.scope())
        .remove_tree(target.key_path())
        .with_context(|| {
            format!(
                "Löschen fehlgeschlagen / delete failed: {}",
                target.full_path()
            )
        })?;

    Ok(())
}

/// Does this key currently exist?
pub fn exists(target: &RegTarget) -> bool {
    paths::root_key(target.scope())
        .open(target.key_path())
        .is_ok()
}

/// Can this key be opened for writing right now?
///
/// Asked rather than derived from the elevation state. Measured in the test
/// VM: even an elevated session cannot write `Directory\shell\cmd`, which
/// belongs to TrustedInstaller, while `Directory\shell\find` in the same hive
/// and category is writable.
pub fn is_writable(target: &RegTarget) -> bool {
    paths::root_key(target.scope())
        .options()
        .read()
        .write()
        .open(target.key_path())
        .is_ok()
}

/// Writes a presence flag: an empty value whose mere existence is the signal.
///
/// `LegacyDisable` hides an entry, `Extended` makes it appear only while Shift
/// is held. Both are reversible with a single value deletion, which is why
/// ToDo 11.3 offers them before ever suggesting a delete.
pub fn set_flag(target: &RegTarget, name: &str, token: &BackupToken) -> Result<()> {
    require_backup(target, token)?;

    paths::root_key(target.scope())
        .options()
        .read()
        .write()
        .open(target.key_path())
        .and_then(|key| key.set_string(name, ""))
        .with_context(|| {
            format!(
                "{name} setzen fehlgeschlagen / could not set: {}",
                target.full_path()
            )
        })
}

/// Removes a presence flag. A missing value is success, not an error.
pub fn clear_flag(target: &RegTarget, name: &str, token: &BackupToken) -> Result<()> {
    require_backup(target, token)?;

    let key = paths::root_key(target.scope())
        .options()
        .read()
        .write()
        .open(target.key_path())
        .with_context(|| {
            format!(
                "Öffnen fehlgeschlagen / could not open: {}",
                target.full_path()
            )
        })?;

    match key.remove_value(name) {
        Ok(()) => Ok(()),
        // Already absent — the caller wanted it gone and it is gone.
        Err(_) if key.get_type(name).is_err() => Ok(()),
        Err(error) => Err(anyhow::Error::from(error).context(format!(
            "{name} entfernen fehlgeschlagen / could not remove: {}",
            target.full_path()
        ))),
    }
}

/// Sets or clears `Position`.
///
/// Takes the value as an opaque string on purpose. `Top` and `Bottom` are what
/// this tool offers and both are confirmed to work — verified by writing probe
/// verbs in the test VM and photographing a real right-click — but they are
/// not the only values Windows uses. `Windows.newfolder` in the CommandStore
/// carries `Position=Last`, and `Windows.playmusic` carries `Position=After`
/// together with a `PositionCompare` GUID naming another verb. Validating
/// against an enum of two would reject or silently mangle real, shipping
/// Microsoft keys.
pub fn set_position(target: &RegTarget, value: Option<&str>, token: &BackupToken) -> Result<()> {
    match value {
        Some(value) => {
            require_backup(target, token)?;
            paths::root_key(target.scope())
                .options()
                .read()
                .write()
                .open(target.key_path())
                .and_then(|key| key.set_string("Position", value))
                .with_context(|| {
                    format!(
                        "Position setzen fehlgeschlagen / could not set: {}",
                        target.full_path()
                    )
                })
        }
        None => clear_flag(target, "Position", token),
    }
}

/// Adds a CLSID to the machine-wide blocked list.
///
/// One value here disables a handler everywhere at once. That beats deleting
/// the same handler under twenty classes, survives the program updating
/// itself, and comes back with a single deletion (ToDo 5.4).
pub fn block_clsid(clsid: &str, token: &BackupToken) -> Result<()> {
    require_blocked_backup(token)?;

    LOCAL_MACHINE
        .create(paths::SHELL_EXTENSIONS_BLOCKED)
        .and_then(|key| key.set_string(clsid, ""))
        .with_context(|| format!("CLSID blockieren fehlgeschlagen / could not block: {clsid}"))
}

/// Takes a CLSID off the blocked list. Absent means already unblocked.
pub fn unblock_clsid(clsid: &str, token: &BackupToken) -> Result<()> {
    require_blocked_backup(token)?;

    let Ok(key) = LOCAL_MACHINE
        .options()
        .read()
        .write()
        .open(paths::SHELL_EXTENSIONS_BLOCKED)
    else {
        // No list at all means nothing is blocked.
        return Ok(());
    };

    match key.remove_value(clsid) {
        Ok(()) => Ok(()),
        Err(_) if key.get_type(clsid).is_err() => Ok(()),
        Err(error) => Err(anyhow::Error::from(error).context(format!(
            "CLSID freigeben fehlgeschlagen / could not unblock: {clsid}"
        ))),
    }
}

/// Can the blocked list be written right now?
///
/// It lives in HKLM, so this is normally false without elevation — but asked
/// rather than assumed, because an elevated instance runs the same code.
pub fn is_blocked_list_writable() -> bool {
    // Deliberately without `.create()`: a probe must not bring the key into
    // existence as a side effect of asking whether it could.
    if LOCAL_MACHINE
        .options()
        .read()
        .write()
        .open(paths::SHELL_EXTENSIONS_BLOCKED)
        .is_ok()
    {
        return true;
    }

    // The list does not exist yet on a machine where nothing was ever
    // blocked, so fall back to asking whether it could be created.
    let parent = paths::SHELL_EXTENSIONS_BLOCKED
        .rsplit_once('\\')
        .map(|(head, _)| head)
        .unwrap_or(paths::SHELL_EXTENSIONS_BLOCKED);

    LOCAL_MACHINE.options().read().write().open(parent).is_ok()
}

/// Is this CLSID currently on the blocked list?
pub fn is_blocked(clsid: &str) -> bool {
    LOCAL_MACHINE
        .open(paths::SHELL_EXTENSIONS_BLOCKED)
        .and_then(|key| key.get_type(clsid))
        .is_ok()
}

fn require_backup(target: &RegTarget, token: &BackupToken) -> Result<()> {
    if token.covers(target) {
        return Ok(());
    }
    bail!(
        "Backup deckt diesen Schlüssel nicht ab / backup does not cover this key: {}",
        target.full_path()
    )
}

fn require_blocked_backup(token: &BackupToken) -> Result<()> {
    if token.covers_path(paths::blocked_list_display_path()) {
        return Ok(());
    }
    bail!("Backup deckt die Blocked-Liste nicht ab / backup does not cover the blocked list")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Scope;
    use crate::registry::backup;

    #[test]
    fn a_token_for_another_key_does_not_authorise_a_delete() {
        // Backing up one key must not become a licence to delete a different
        // one — the token records which keys it covers, not merely that some
        // backup happened.
        // Its own throwaway class rather than `Directory\shell`: the scanner
        // tests enumerate that key in parallel, and a test should not make
        // another test's enumeration race.
        let target = |name: &str| {
            RegTarget::below_classes(
                Scope::User,
                &format!(r"ctxmenu_selftest_write\shell\{name}"),
            )
            .expect("a test path names an entry")
        };
        let covered = target("token_a");
        let other = target("token_b");

        // Create only the first key so the export succeeds.
        paths::root_key(Scope::User)
            .create(covered.key_path())
            .expect("HKCU is writable");

        let token = backup::export_targets("selftest_token", std::slice::from_ref(&covered))
            .expect("export of an existing key");

        assert!(token.covers(&covered));
        assert!(!token.covers(&other));
        assert!(
            delete_tree(&other, &token).is_err(),
            "a delete outside the backup must be refused"
        );

        delete_tree(&covered, &token).expect("covered delete");
        assert!(!exists(&covered));

        let _ = std::fs::remove_dir_all(token.directory());
        let _ =
            paths::root_key(Scope::User).remove_tree("SOFTWARE\\Classes\\ctxmenu_selftest_write");
    }
}
