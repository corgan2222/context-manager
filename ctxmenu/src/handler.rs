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
//! the deployment stack without pulling WinRT into this program. It needs
//! an elevated context (measured 2026-08-20); a plain run tries directly
//! first and only then asks for elevation, because whether the current
//! token may deploy is measured, not assumed (`decisions/0022`).

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

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
/// Read from `PackagedCom` — the same place the scan reads every package
/// from — rather than asked via a child process: a status that costs a
/// process start would end up being guessed instead of checked.
pub fn is_installed() -> bool {
    let Ok(key) = windows_registry::CURRENT_USER.open(r"Software\Classes\PackagedCom\Package")
    else {
        return false;
    };
    let prefix = format!("{}_", PACKAGE_NAME.to_lowercase());
    crate::registry::scan::subkey_names(&key)
        .iter()
        .any(|name| name.to_lowercase().starts_with(&prefix))
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
fn powershell(command: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("\x1ePowerShell nicht startbar\x1fcould not start PowerShell\x1d")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{}", stderr.trim());
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
    if powershell(&register).is_ok() {
        return Ok(true);
    }
    if crate::elevation::is_elevated() {
        // Already elevated and still refused: the error is real. Run once
        // more so it reaches the caller with PowerShell's own words.
        powershell(&register)?;
        return Ok(true);
    }

    match crate::elevation::run_self_elevated("handler install") {
        crate::elevation::Outcome::Finished(0) => Ok(true),
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
        return;
    }
    if let Err(reason) = deploy() {
        crate::errln!("handler_refresh: {reason:#}");
    }
}

/// Removes the package. The files stay — they are inert without the
/// registration, and the DLL may still be mapped into a `dllhost` that has
/// not gone away yet; the next install overwrites only what changed.
pub fn remove() -> Result<bool> {
    let unregister = format!("Get-AppxPackage '{PACKAGE_NAME}' | Remove-AppxPackage");
    if powershell(&unregister).is_ok() && !is_installed() {
        return Ok(true);
    }
    if crate::elevation::is_elevated() {
        powershell(&unregister)?;
        return Ok(true);
    }
    match crate::elevation::run_self_elevated("handler remove") {
        crate::elevation::Outcome::Finished(0) => Ok(true),
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
