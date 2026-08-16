//! The Windows 11 context menu, and the switch that turns it off.
//!
//! Windows 11 replaced the context menu with a shorter one and moved everything
//! it does not know about behind *Show more options*. Entries written to the
//! registry — every entry this program can make — live down there. That is not
//! a bug in them: the new menu only shows built-in commands and handlers
//! registered as `IExplorerCommand` in a signed MSIX package.
//!
//! There is one documented way back, and it is a single registry key:
//!
//! ```text
//! HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}
//!     \InprocServer32   (default) = ""     <- empty, on purpose
//! ```
//!
//! That CLSID is the shell extension that *draws* the new menu. Pointing its
//! in-process server at nothing makes the loading fail, and Explorer falls back
//! to the classic menu — the whole one, with every entry in it. Removing the key
//! puts the new menu back.
//!
//! Nothing is faked here: the value really is an empty string, and the empty
//! string really is what does the work. It is the mechanism every "bring back
//! the old right-click menu" recipe uses, and it survives because it is not a
//! hack against Explorer but an ordinary COM registration that happens to be
//! unusable.
//!
//! Two things follow, and both are the user's to decide:
//!
//! * It is **per user**, in `HKCU`, so no elevation and no effect on anyone
//!   else's account.
//! * It needs **Explorer to restart** before anything changes. The menu handler
//!   is loaded once, when the shell starts.

use anyhow::{Context as _, Result};
use windows_registry::CURRENT_USER;

/// The shell extension that draws the Windows 11 context menu.
const MENU_CLSID: &str = "{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}";

/// Where the switch lives, relative to `HKCU`.
fn switch_path() -> String {
    format!("Software\\Classes\\CLSID\\{MENU_CLSID}")
}

/// Whether this Windows even has the new menu.
///
/// Build 22000 is the first Windows 11. On anything older the switch would be
/// a key that does nothing, and a control that does nothing is worse than a
/// control that is not there.
pub fn has_new_menu() -> bool {
    build_number() >= 22000
}

/// The build number this is running on, or 0 when it cannot be read.
///
/// Read from the registry rather than from `GetVersionEx`: that function lies
/// to processes without the right manifest entry, and has done since Windows 8.
/// `CurrentBuildNumber` is what `winver` shows.
pub fn build_number() -> u32 {
    CURRENT_USER
        .open("Software")
        .ok()
        .and_then(|_| {
            windows_registry::LOCAL_MACHINE
                .open("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
                .ok()
        })
        .and_then(|key| key.get_string("CurrentBuildNumber").ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

/// Whether the classic menu is switched on.
pub fn classic_menu() -> bool {
    CURRENT_USER
        .open(format!("{}\\InprocServer32", switch_path()))
        .is_ok()
}

/// Switches between the Windows 11 menu and the classic one.
///
/// `true` means the classic menu. Explorer has to restart before it takes
/// effect; the caller offers that, because doing it unasked closes every
/// Explorer window the user has open.
pub fn set_classic_menu(classic: bool) -> Result<()> {
    match classic {
        true => {
            // The key exists and its default value is an empty string. Both
            // parts matter: a missing key means the new menu, and a key with a
            // path in it would send Explorer looking for a DLL.
            let key = CURRENT_USER
                .create(format!("{}\\InprocServer32", switch_path()))
                .with_context(|| {
                    format!(
                        "\x1eSchlüssel anlegen\x1fcreating key\x1d: HKCU\\{}",
                        switch_path()
                    )
                })?;
            key.set_string("", "")
                .with_context(|| "\x1eLeeren Standardwert setzen\x1fsetting the empty default\x1d")
        }
        false => {
            // Remove the whole CLSID key, not just the value: an empty
            // InprocServer32 left behind would keep the new menu off.
            let classes = CURRENT_USER
                .create("Software\\Classes\\CLSID")
                .context("\x1eHKCU\\Software\\Classes\\CLSID\x1f\x1d")?;
            // Asked first rather than matching on the error afterwards: the
            // "not found" code is spelled differently by every layer this
            // travels through, and "it is already gone" is the state that was
            // wanted anyway.
            if !classic_menu() && classes.open(MENU_CLSID).is_err() {
                return Ok(());
            }
            classes.remove_tree(MENU_CLSID).with_context(|| {
                format!(
                    "\x1eSchlüssel entfernen\x1fremoving key\x1d: HKCU\\{}",
                    switch_path()
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_sits_where_every_recipe_says_it_does() {
        assert_eq!(
            switch_path(),
            "Software\\Classes\\CLSID\\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}"
        );
    }

    #[test]
    fn the_build_number_is_the_one_winver_shows() {
        let build = build_number();
        // This machine is Windows 10 19045 or newer; anything below 10240 means
        // the read failed rather than that the machine is ancient.
        assert!(build >= 10240, "read {build} as the build number");
    }

    #[test]
    fn only_windows_eleven_is_offered_the_switch() {
        // The control is drawn from `has_new_menu`, so this pins the boundary
        // rather than whatever this particular machine happens to be.
        assert_eq!(has_new_menu(), build_number() >= 22000);
    }

    #[test]
    fn reading_the_switch_never_fails_however_the_registry_looks() {
        // Whatever the answer, it comes back without a panic and without a
        // Result the caller has to unwrap in the frame path.
        let _: bool = classic_menu();
    }
}
