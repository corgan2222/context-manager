//! Changing the registry.
//!
//! Every function here is deliberately hard to misuse. The failure mode this
//! guards against is not a wrong error message — it is an unusable Explorer
//! after a wrong key was removed, which is only noticed at the next
//! right-click and cannot be undone from memory.

use anyhow::{Context as _, Result, bail};

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

    paths::root_key(target.scope)
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
    paths::root_key(target.scope)
        .open(target.key_path())
        .is_ok()
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
        let covered = RegTarget {
            scope: Scope::User,
            relative: r"ctxmenu_selftest_write\shell\token_a".into(),
        };
        let other = RegTarget {
            scope: Scope::User,
            relative: r"ctxmenu_selftest_write\shell\token_b".into(),
        };

        // Create only the first key so the export succeeds.
        paths::root_key(Scope::User)
            .create(covered.key_path())
            .expect("HKCU is writable");

        let token = backup::export("selftest_token", std::slice::from_ref(&covered))
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
