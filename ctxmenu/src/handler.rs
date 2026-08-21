//! Putting ctxmenu's own entries into the new Windows 11 menu.
//!
//! The pieces travel inside this executable: the handler DLL (embedded by
//! `build.rs`), the sparse package that registers it, and its logo. Turning
//! the feature on writes them to `%LOCALAPPDATA%\ctxmenu\win11\` and
//! registers the package with the shell; from then on the DLL serves
//! `entries.json` on every menu open, and creating or removing entries
//! needs no further package work (`decisions/0036`).
//!
//! Registration is `Add-AppxPackage -AllowUnsigned` through PowerShell — a
//! child process, but the one tool that is on every Windows 11 and speaks
//! the deployment stack. Adding needs an elevated context: the package
//! declares an executable activation, and Windows refuses that for an
//! unsigned package with `0x80073D2B` (measured on a Windows 11 VM,
//! 2026-08-21). Removing needs no rights at all — a package registered for
//! this user is this user's to remove — so each operation tries plainly
//! first and elevates only where the try failed, because what a token may do
//! is measured, not assumed (`decisions/0022`).
//!
//! *Asking* whether the package is registered does not go through PowerShell.
//! It goes to [`PackageManager`], and it used to go to a registry key. That
//! key, `Software\Classes\PackagedCom\Package`, outlives the registration it
//! describes: on the 2026-08-21 VM it still named
//! `ctxmenu.Menu_1.0.0.0_x64__0fw22dw1vr2nw` while `Get-AppxPackage` found
//! nothing, hours after Windows had removed the package on its own
//! (`RemoveForUserProfileDeletion`, 07:04:46, in
//! `Microsoft-Windows-AppXDeploymentServer/Operational`). A leftover key
//! made the checkbox claim a handler that was gone and made removing it
//! impossible, because the removal kept finding the key it had just tried to
//! get rid of. [`is_installed`] now asks the deployment stack and clears the
//! key when the two disagree.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use windows::Management::Deployment::PackageManager;
use windows::core::HSTRING;

/// The DLL, fresh from this build. `build.rs` builds it and names the path.
static DLL: &[u8] = include_bytes!(env!("CTXMENU_HANDLER_DLL"));
/// The sparse package. Checked in rather than packed at build time: it
/// holds nothing but `AppxManifest.xml`, and `makeappx` lives in the SDK,
/// not on every machine that builds. Repack when the manifest changes:
/// `makeappx pack /d <dir-with-manifest> /p handler.msix /nv /o`.
static MSIX: &[u8] = include_bytes!("../../ctxmenu-handler/handler.msix");
static LOGO: &[u8] = include_bytes!("../../ctxmenu-handler/logo.png");

/// The package identity's name — the prefix its full name starts with.
const PACKAGE_NAME: &str = "ctxmenu.Menu";

/// Keeps the PowerShell child from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where the external content lives.
pub fn directory() -> Result<PathBuf> {
    let base =
        dirs::data_local_dir().context("\x1ekein LOCALAPPDATA\x1fno local data directory\x1d")?;
    Ok(base.join("ctxmenu").join("win11"))
}

/// Is the handler package registered for this user?
///
/// Asked of the deployment stack. Where it says no while `PackagedCom`
/// still names the package, the stale key is cleared as a side effect —
/// that is not tidying, it is what keeps every later answer true, and it is
/// the one thing that lets a user who lost the registration switch the
/// checkbox off again (see the module note).
///
/// Where the stack cannot be asked at all, the old reading of `PackagedCom`
/// stands in and nothing is cleared. A WinRT failure is not an answer, and
/// reporting it as "not installed" would be the same lie in the other
/// direction.
pub fn is_installed() -> bool {
    match registered_for_this_user() {
        Some(true) => true,
        Some(false) => {
            clear_stale_com_key();
            false
        }
        None => named_in_packaged_com(),
    }
}

/// What the deployment stack says, or `None` where it could not be asked.
///
/// Filtered on the identity *name*, not on the package family name. The
/// family name would turn the walk into a single lookup, but it ends in a
/// hash of the publisher string that this program cannot compute — and a
/// hash gone stale reads exactly like "not installed", which is the failure
/// this function exists to stop making. The walk is over the packages of
/// one user and runs at startup and after a click, never in the frame path.
fn registered_for_this_user() -> Option<bool> {
    // An empty security id means "the user this process belongs to".
    let manager = PackageManager::new().ok()?;
    let packages = manager.FindPackagesByUserSecurityId(&HSTRING::new()).ok()?;

    for package in packages {
        let Ok(name) = package.Id().and_then(|id| id.Name()) else {
            // One unreadable package is not an answer about ours.
            continue;
        };
        if name.to_string_lossy().eq_ignore_ascii_case(PACKAGE_NAME) {
            return Some(true);
        }
    }
    Some(false)
}

/// Does `PackagedCom` still name the package?
///
/// The reading [`is_installed`] used to be, kept for the case where WinRT
/// cannot be reached and for deciding whether there is a stale key at all.
fn named_in_packaged_com() -> bool {
    let Ok(key) =
        windows_registry::CURRENT_USER.open(crate::registry::packaged::PACKAGED_COM_PACKAGE)
    else {
        return false;
    };
    let prefix = format!("{}_", PACKAGE_NAME.to_lowercase());
    crate::registry::scan::subkey_names(&key)
        .iter()
        .any(|name| name.to_lowercase().starts_with(&prefix))
}

/// Deletes what a gone registration left behind under `PackagedCom`.
///
/// Under `HKEY_CURRENT_USER`, so no elevation is involved. Quiet when there
/// is nothing to delete, which is the normal case; loud in the log when
/// there was, because a key outliving its package is worth knowing about
/// and nothing else in the program would ever mention it.
fn clear_stale_com_key() {
    let Ok(key) =
        windows_registry::CURRENT_USER.open(crate::registry::packaged::PACKAGED_COM_PACKAGE)
    else {
        return;
    };
    let prefix = format!("{}_", PACKAGE_NAME.to_lowercase());

    for name in crate::registry::scan::subkey_names(&key) {
        if !name.to_lowercase().starts_with(&prefix) {
            continue;
        }
        match key.remove_tree(&name) {
            Ok(()) => crate::errln!("handler_stale_com_key_cleared: {name}"),
            Err(reason) => crate::errln!("handler_stale_com_key_stays: {name}: {reason}"),
        }
    }
}

/// Writes DLL, package and logo, skipping bytes that are already there.
///
/// The skip is not an optimisation: while the shell has the DLL loaded in
/// a `dllhost`, overwriting it fails — but an unchanged DLL needs no
/// overwrite, so a repeated install stays idempotent instead of erroring.
fn deploy() -> Result<PathBuf> {
    let dir = directory()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("\x1eOrdner anlegen\x1fcreating\x1d: {}", dir.display()))?;

    for (name, bytes) in [
        ("ctxmenu_handler.dll", DLL),
        ("handler.msix", MSIX),
        ("logo.png", LOGO),
    ] {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|current| current == bytes) {
            continue;
        }
        std::fs::write(&path, bytes)
            .with_context(|| format!("\x1eSchreiben\x1fwriting\x1d: {}", path.display()))?;
    }
    Ok(dir)
}

/// One PowerShell command, window-less, with its stderr in the error.
///
/// `$ErrorActionPreference = 'Stop'` goes in front of every command, and it
/// is not decoration. `Add-AppxPackage` and `Remove-AppxPackage` report a
/// deployment failure as a *non-terminating* error under the default
/// preference: the text lands on stderr, the process still exits 0, and
/// `output.status.success()` calls that success. That is how a refused
/// registration used to come back as `Ok(())`. With the preference set, the
/// same failure ends the command and the exit code says so.
///
/// The exit code is still not the last word anywhere it matters — every
/// caller checks [`is_installed`] afterwards. An empty pipeline
/// (`Get-AppxPackage` finding nothing to pipe into `Remove-AppxPackage`) is
/// no error at all under any preference, so "it exited 0" and "the package
/// is gone" remain two different statements.
fn powershell(command: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("$ErrorActionPreference = 'Stop'; {command}"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("\x1ePowerShell nicht startbar\x1fcould not start PowerShell\x1d")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{}", stderr.trim());
}

/// The error for an operation that reported success and changed nothing.
///
/// Its own function because both [`install`] and [`remove`] can end there,
/// and because the sentence is the whole point of 1.5.1: the program used
/// to say "done" here.
fn bail_unchanged(installing: bool) -> anyhow::Error {
    match installing {
        true => anyhow::anyhow!(
            "\x1eDie Registrierung meldet Erfolg, das Paket ist trotzdem nicht eingerichtet\
             \x1fthe registration reported success, yet the package is not installed\x1d"
        ),
        false => anyhow::anyhow!(
            "\x1eDas Entfernen meldet Erfolg, das Paket ist trotzdem noch eingerichtet\
             \x1fthe removal reported success, yet the package is still installed\x1d"
        ),
    }
}

/// Registers the package, elevating only when the direct try is refused.
///
/// Returns whether the user now has the handler. `Ok(false)` is one case
/// only: the UAC prompt was declined — a decision, not a fault.
pub fn install() -> Result<bool> {
    let dir = deploy()?;

    let register = format!(
        "Add-AppxPackage -Path '{}' -ExternalLocation '{}' -AllowUnsigned",
        dir.join("handler.msix").display(),
        dir.display()
    );

    // The plain try. On the 2026-08-21 VM it always failed with 0x80073D2B,
    // "an unsigned package cannot include Executable activations" — the
    // manifest declares one, so a plain token is refused. Kept anyway,
    // because that is a rule about this Windows and this manifest, and
    // 0022 says such things are measured per machine rather than assumed.
    if let Err(reason) = powershell(&register) {
        crate::errln!("handler_install_plain: {reason:#}");
    } else if is_installed() {
        return Ok(true);
    } else {
        crate::errln!("handler_install_plain: exited clean, package not registered");
    }

    if crate::elevation::is_elevated() {
        // Already elevated and still refused: the error is real. Run once
        // more so it reaches the caller with PowerShell's own words.
        powershell(&register)?;
        if !is_installed() {
            return Err(bail_unchanged(true));
        }
        return Ok(true);
    }

    match crate::elevation::run_self_elevated("handler install") {
        // The exit code says the child finished, not that it achieved
        // anything. Only this call knows the difference.
        crate::elevation::Outcome::Finished(0) if is_installed() => Ok(true),
        crate::elevation::Outcome::Finished(0) => Err(bail_unchanged(true)),
        crate::elevation::Outcome::Cancelled => Ok(false),
        crate::elevation::Outcome::Finished(code) => bail!(
            "\x1eDie erhöhte Instanz meldet Fehler {code}\
             \x1fthe elevated instance reported error {code}\x1d"
        ),
        crate::elevation::Outcome::Failed(reason) => bail!("{reason}"),
    }
}

/// Freshens the deployed files after a self-update, quietly.
///
/// The update chain replaces the exe and restarts it, and the new exe may
/// carry a newer DLL than the one deployed beside the package — which the
/// shell would keep serving. Where the package is not registered there is
/// nothing to freshen. A failure stays on stderr: the shell may still hold
/// the old DLL in a `dllhost`, and the next start simply tries again. A
/// *manifest* change is not covered — that takes a re-registration, which
/// is [`install`]'s job and may ask for elevation.
pub fn refresh_deployed() {
    if !is_installed() {
        if registration_lost() {
            crate::errln!(
                "handler_registration_lost: the package files are deployed, \
                 the package is not registered"
            );
        }
        return;
    }
    if let Err(reason) = deploy() {
        crate::errln!("handler_refresh: {reason:#}");
    }
}

/// Were the package files put in place by a registration that is now gone?
///
/// The one question that separates "the user never switched this on" from
/// "the user switched this on and no longer has it". Windows removes an
/// unsigned sparse package on its own — measured 2026-08-21, six hours
/// after a registration nobody touched — and until 1.5.1 the program had no
/// way to notice, because the checkbox was reading a key that stayed behind.
///
/// [`deploy`] leaves all three files, so any of them would do; the package
/// is the one the registration actually names.
pub fn registration_lost() -> bool {
    let Ok(dir) = directory() else {
        return false;
    };
    dir.join("handler.msix").is_file() && !is_installed()
}

/// Removes the package. The files stay — they are inert without the
/// registration, and the DLL may still be mapped into a `dllhost` that has
/// not gone away yet; the next install overwrites only what changed.
pub fn remove() -> Result<bool> {
    // Where the registration is already gone, there is nothing to run. This
    // is the ordinary case after Windows removed the package by itself, and
    // the call also clears the `PackagedCom` key that made the checkbox
    // claim otherwise. Before 1.5.1 this path did not exist: the removal
    // went looking for elevation it did not need, to undo something that
    // was not there, and reported success while the checkbox sprang back.
    if !is_installed() {
        return Ok(true);
    }

    let unregister = format!("Get-AppxPackage '{PACKAGE_NAME}' | Remove-AppxPackage");

    // Removing needs no administrator rights: a package registered for this
    // user is this user's to remove. So the plain try is the expected way
    // through, not a hopeful first shot.
    let plain = powershell(&unregister);
    if !is_installed() {
        return Ok(true);
    }
    if let Err(reason) = &plain {
        crate::errln!("handler_remove_plain: {reason:#}");
    }

    if crate::elevation::is_elevated() {
        powershell(&unregister)?;
        if !is_installed() {
            return Ok(true);
        }
        return Err(bail_unchanged(false));
    }

    match crate::elevation::run_self_elevated("handler remove") {
        crate::elevation::Outcome::Finished(0) if !is_installed() => Ok(true),
        crate::elevation::Outcome::Finished(0) => Err(bail_unchanged(false)),
        crate::elevation::Outcome::Cancelled => Ok(false),
        crate::elevation::Outcome::Finished(code) => bail!(
            "\x1eDie erhöhte Instanz meldet Fehler {code}\
             \x1fthe elevated instance reported error {code}\x1d"
        ),
        crate::elevation::Outcome::Failed(reason) => bail!("{reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trap 1.5.0 fell into, in one assertion.
    ///
    /// `powershell.exe -Command` exits 0 when a cmdlet writes a
    /// *non-terminating* error, which is what `Add-AppxPackage` and
    /// `Remove-AppxPackage` do on a deployment failure. `Get-Item` on a
    /// path that is not there fails the same way, so it stands in for them
    /// without deploying anything. Take `$ErrorActionPreference = 'Stop'`
    /// back out of [`powershell`] and this test goes green in the wrong
    /// direction: `expect_err` starts failing because the call returns
    /// `Ok(())`.
    #[test]
    fn a_failing_cmdlet_is_an_error_and_not_an_exit_code_of_zero() {
        let error = powershell(r"Get-Item 'C:\this\path\does\not\exist\at\all.txt'")
            .expect_err("a cmdlet that failed must not come back as success");
        let said = format!("{error:#}");
        assert!(
            !said.trim().is_empty(),
            "the error must carry PowerShell's own words, got: {said:?}"
        );
    }

    /// Regression for the checkbox that would not switch off.
    ///
    /// With no registration there is nothing to remove, and saying so is the
    /// whole answer — no PowerShell, no UAC prompt, and above all no
    /// `Ok(true)` handed back while `is_installed` still says yes. In 1.5.0
    /// this path did not exist and the stale `PackagedCom` key sent every
    /// click through elevation to undo something that was not there.
    #[test]
    fn removing_what_is_not_registered_succeeds_without_touching_anything() {
        if is_installed() {
            // Cannot exercise this on a machine that really has the package,
            // and unregistering the tester's own handler to find out is not
            // something a test gets to do.
            return;
        }
        assert!(
            remove().expect("removing nothing is not a failure"),
            "with no registration, removal is already done"
        );
    }

    /// Both halves of the "reported success, changed nothing" error are
    /// bilingual, and they do not say the same thing.
    ///
    /// Asked through `bilingual::pick` rather than by looking for the marker
    /// characters: a test that spells the markers out as literals is itself
    /// a malformed bilingual group, and
    /// `every_marked_message_in_the_source_is_a_complete_group` scans this
    /// file too.
    #[test]
    fn the_unchanged_error_speaks_both_languages_and_distinguishes_the_two_cases() {
        use crate::bilingual::{is_marker, pick};
        use crate::settings::Language;

        let installing = format!("{}", bail_unchanged(true));
        let removing = format!("{}", bail_unchanged(false));

        for message in [&installing, &removing] {
            let german = pick(message, Language::German);
            let english = pick(message, Language::English);

            assert_ne!(
                german, english,
                "one text for both languages means the markers are missing: {message:?}"
            );
            for picked in [&german, &english] {
                assert!(!picked.trim().is_empty(), "empty half in {message:?}");
                assert!(
                    !picked.chars().any(is_marker),
                    "a marker survived into the shown text: {picked:?}"
                );
            }
        }
        assert_ne!(
            installing, removing,
            "an install that did nothing and a removal that did nothing are              not the same news"
        );
    }

    /// Asking twice in a row must not change the answer. Weak on its own,
    /// but [`is_installed`] now has a side effect — it clears a stale key —
    /// and a side effect that alters the next answer would be a loop the
    /// checkbox sits inside.
    #[test]
    fn the_registration_question_answers_the_same_way_twice() {
        assert_eq!(is_installed(), is_installed());
    }

    /// Where nothing was ever deployed, nothing was lost. The distinction
    /// the tooltip rests on.
    #[test]
    fn nothing_is_reported_lost_while_the_package_file_is_absent() {
        let deployed = directory().is_ok_and(|dir| dir.join("handler.msix").is_file());
        if deployed {
            // This machine has the files; the question this test asks
            // cannot be put to it.
            return;
        }
        assert!(!registration_lost());
    }

    /// The whole of 1.5.1 rests on this call working.
    ///
    /// Where `PackageManager` cannot be reached, [`is_installed`] falls back
    /// to reading `PackagedCom` — the very reading that caused the bug. The
    /// fallback is deliberate, because a WinRT failure is not an answer, but
    /// it must stay the exception. This test says so out loud rather than
    /// letting a machine quietly return to 1.5.0 behaviour.
    #[test]
    fn the_deployment_stack_can_be_asked() {
        assert!(
            registered_for_this_user().is_some(),
            "PackageManager did not answer, so is_installed is back to              reading PackagedCom"
        );
    }

    #[test]
    fn the_embedded_pieces_are_not_empty() {
        assert!(DLL.len() > 100_000, "a real DLL, not a stub");
        assert!(MSIX.len() > 500, "a real package");
        assert!(LOGO.len() > 50, "a real image");
    }

    /// The manifest template, the packed msix and the DLL must agree on the
    /// CLSID — three copies of one value, and only the template is text.
    #[test]
    fn the_manifest_template_names_the_handler_clsid() {
        let manifest = include_str!("../../ctxmenu-handler/AppxManifest.xml");
        assert!(
            manifest.contains("C898E0C0-879E-4A3E-AF7E-631D99C7DE44"),
            "the template must name the CLSID the DLL implements"
        );
        assert!(manifest.contains(PACKAGE_NAME));
    }
}
