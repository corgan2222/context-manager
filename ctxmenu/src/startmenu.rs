//! The one file this program leaves lying around, and what it buys.
//!
//! A shortcut in the user's Start menu, `ctxmenu.lnk`, carrying nothing but
//! the path of the running `.exe` and one property: `PKEY_AppUserModel_ID`,
//! set to the same identifier [`crate::notify`] files its toasts under.
//!
//! # Why a program that installs nothing writes a file after all
//!
//! [`crate::notify`] has the measurement that forced this. A toast fired under
//! an invented AppUserModelID *is* accepted, *is* stored, and *is* there in
//! the Action Center afterwards -- the history under
//! `ctxmenu.ContextMenuManager` grew by exactly one entry per file, 8 to 13
//! for five files. What it never did was appear on screen. Measured on
//! 2026-08-19, with everything else ruled out one at a time:
//!
//! ```text
//! Focus Assist                      Unrestricted -- off
//! ToastEnabled                      1
//! per-app switches, written by hand Enabled/ShowBanner/ShowInActionCenter -- no change
//! the same toast, PowerShell's registered AUMID   appeared, visibly
//! ```
//!
//! The only difference left was the registration. Windows draws a banner for a
//! desktop program's toast when a Start menu shortcut names the same
//! identifier in `System.AppUserModel.ID`. The registry value
//! [`crate::notify`] writes gives the sender a *name*; this file gives it the
//! standing to be shown at all.
//!
//! With the file in place the banner arrived -- but not at once. It took
//! several minutes after the first write: a run one minute later still drew
//! nothing, one four minutes later drew the banner, and from then on every
//! run did. The shell reads that folder on its own schedule, and no amount of
//! `SHChangeNotify` moved it along. Nothing to fix here, but worth knowing
//! before somebody measures a fresh machine once and calls this broken.
//!
//! What Windows learns this way, it keeps. Measured afterwards on 2026-08-20:
//! with the shortcut *deleted* the banner still appeared, and the platform's
//! own `Notifications\Settings\ctxmenu.ContextMenuManager` key -- absent
//! before any of this -- was there. So deleting the file is safe rather than
//! punishing, and the honest claim is the narrow one: this is what gets the
//! sender known in the first place.
//!
//! The price was named and paid deliberately: one shortcut, in the folder the
//! Start menu reads anyway, for a program that otherwise leaves nothing
//! behind. The message reaches the Action Center with or without it, which is
//! where it had to survive in the first place.
//!
//! # What is deliberately not here
//!
//! No desktop icon, no autostart entry, no uninstall registration, and no
//! `IconUri` -- that one would need a `.png` on disk, and a second file is a
//! second thing to explain and a second thing to delete.
//!
//! # Ten processes at once
//!
//! The registry command of a web tool favourite ends in `"%1"`, so ten
//! selected files start ten of these processes within the same few
//! milliseconds. Two consequences shape everything below:
//!
//! * **The common case must cost almost nothing.** After the first run the
//!   shortcut is already right, so [`ensure`] reads it and returns. Nothing is
//!   written unless something actually differs.
//! * **A write must not be observable half-done.** The shortcut is built under
//!   a name of this process's own and renamed into place, and a rename within
//!   one directory is atomic. Ten processes writing at once produce ten
//!   identical files and one winner; nobody ever sees a torn `.lnk`.
//!
//! # Failure is never fatal
//!
//! Same rule as the display name in [`crate::notify`], for the same reason:
//! this is decoration on the delivery. A missing shortcut costs the banner,
//! not the message. Nothing in here may be the reason [`crate::notify::show`]
//! returns an `Err`, because that summons the very message box the toast was
//! introduced to get rid of -- ten of them, one per file.

use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink};
use windows::core::{BSTR, GUID, Interface as _, PCWSTR, PWSTR};

/// `PKEY_AppUserModel_ID`, written out rather than imported.
///
/// The same trade [`crate::webtool::shell`] makes with `CF_HDROP`: the
/// constant lives in `Win32_Storage_EnhancedStorage`, a module of several
/// hundred property keys pulled in for one of them. The value has been in
/// `propkey.h` unchanged since Windows 7, and the test below checks the typed
/// digits against the same GUID written the other way round.
const PKEY_APPUSERMODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

/// Below the roaming application data directory: the user's own Start menu.
///
/// The per-user folder, not the one under `%ProgramData%`: nothing here is
/// machine-wide and nothing here needs elevation.
const FOLDER: &str = r"Microsoft\Windows\Start Menu\Programs";

/// What the shortcut is called, and therefore what the Start menu lists.
const FILE_NAME: &str = "ctxmenu.lnk";

/// Makes sure the Start menu shortcut is there and names the running `.exe`.
///
/// Called from [`crate::notify::show`], which is reached only from the
/// `--favourite` path. Takes the identifier as an argument so that the one
/// place it is written down stays the one place it is written down.
///
/// Silent about everything except an actual failed write, and even that only
/// reaches the log. See the module documentation for why nothing in here is
/// allowed to fail loudly.
pub fn ensure(aumid: &str) {
    let Some(shortcut) = path() else { return };
    let Ok(target) = std::env::current_exe() else {
        return;
    };

    // Declared before anything COM produces, so that every interface below is
    // released before the apartment is left again.
    let _apartment = Apartment::enter();

    if current(&shortcut, &target, aumid) {
        return;
    }

    if let Err(error) = replace(&shortcut, &target, aumid) {
        crate::log::write(
            crate::log::Kind::Error,
            &format!("start menu shortcut not written: {error:#}"),
        );
    }
}

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\ctxmenu.lnk`
fn path() -> Option<PathBuf> {
    Some(shortcut_in(&dirs::config_dir()?))
}

/// The same path, from a roaming application data directory handed in.
///
/// Split out from [`path`] so the folder and the file name can be checked
/// without a Start menu: `dirs::config_dir` is the roaming one on Windows,
/// which is where a per-user Start menu lives.
fn shortcut_in(roaming: &Path) -> PathBuf {
    roaming.join(FOLDER).join(FILE_NAME)
}

/// Whether the shortcut already says exactly what a fresh one would say.
///
/// Both halves matter, and the second is easy to talk oneself out of:
///
/// * **The target**, because an `.exe` that was moved, renamed or replaced
///   leaves a shortcut behind that points at nothing. Windows would still read
///   the identifier off it, but the entry in the Start menu would be a dead
///   one, and a dead entry is worse than none.
/// * **The identifier**, because everything this file exists for hangs off it.
///   A `ctxmenu.lnk` somebody made by hand, or one whose `SetValue` failed
///   after `SetPath` succeeded, would pass a target-only check for ever and
///   never once raise a banner.
fn current(shortcut: &Path, target: &Path, aumid: &str) -> bool {
    read(shortcut).is_ok_and(|(path, id)| same_file(&path, target) && id == aumid)
}

/// The target path and the identifier of an existing shortcut.
///
/// An `Err` covers the ordinary case of there being no file yet, so it is not
/// worth telling apart from a real failure: both mean "write one".
fn read(shortcut: &Path) -> Result<(String, String)> {
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .context("CoCreateInstance(ShellLink)")?;

    let file = wide(shortcut);
    unsafe {
        link.cast::<IPersistFile>()?
            .Load(PCWSTR(file.as_ptr()), STGM_READ)
    }
    .context("IPersistFile::Load")?;

    // `SLGP_RAWPATH`: what the file says, not what the shell would make of it.
    // The resolving form goes looking across the disk for something that
    // moved and answers with whatever it found -- which is precisely the case
    // this check exists to notice.
    let mut buffer = [0u16; 1024];
    unsafe { link.GetPath(&mut buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32) }
        .context("IShellLinkW::GetPath")?;

    let store: IPropertyStore = link.cast().context("IPropertyStore")?;
    let value =
        unsafe { store.GetValue(&PKEY_APPUSERMODEL_ID) }.context("IPropertyStore::GetValue")?;

    // `PropVariantToBSTR` under the hood, which reads `VT_LPWSTR` as happily
    // as `VT_BSTR`; a store with no such property answers with an error, and
    // an empty identifier then fails the comparison, which is the right
    // outcome.
    let id = BSTR::try_from(&value).unwrap_or_default().to_string();

    Ok((cut_at_nul(&buffer), id))
}

/// Builds the shortcut and puts it in place, whole or not at all.
///
/// The two steps are the point: the `.lnk` is written under a name only this
/// process uses, and only a finished file is renamed onto the real one. A
/// rename within one directory is atomic, so a reader sees either the old
/// shortcut or the new one and never the middle of a write -- which is what
/// ten simultaneous processes make a realistic worry rather than a theoretical
/// one.
fn replace(shortcut: &Path, target: &Path, aumid: &str) -> Result<()> {
    let folder = shortcut
        .parent()
        .context("the shortcut path has no directory")?;
    std::fs::create_dir_all(folder).context("creating the Start menu folder")?;

    let draft = folder.join(draft_name(std::process::id()));

    let written = build(&draft, target, aumid)
        .and_then(|()| std::fs::rename(&draft, shortcut).context("renaming the draft into place"));

    if written.is_err() {
        // Nothing half-written is left in a folder the Start menu reads.
        let _ = std::fs::remove_file(&draft);
    }

    written
}

/// The name a draft is written under, before it becomes the shortcut.
///
/// One per process id, so that ten processes writing at the same moment write
/// ten separate files and then race only on the rename -- where the loser
/// simply overwrites the winner with identical content.
///
/// Not a `.lnk`: for the fraction of a second it exists it lies in a folder
/// the Start menu enumerates, and an extension the shell does not read as a
/// program entry is the cheaper flicker.
fn draft_name(process: u32) -> String {
    format!("ctxmenu.{process}.tmp")
}

/// Creates one shortcut file at `file`.
fn build(file: &Path, target: &Path, aumid: &str) -> Result<()> {
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .context("CoCreateInstance(ShellLink)")?;

    let path = wide(target);
    unsafe { link.SetPath(PCWSTR(path.as_ptr())) }.context("IShellLinkW::SetPath")?;

    let store: IPropertyStore = link.cast().context("IPropertyStore")?;
    let mut id = wide_str(aumid);

    // `VT_LPWSTR`, the shape `InitPropVariantFromString` produces and the one
    // every documented example of this uses -- not the `VT_BSTR` that
    // `PROPVARIANT::from(&str)` would give.
    //
    // `ManuallyDrop` is not tidiness here but the difference between working
    // and corrupting a heap: `PROPVARIANT` drops through `PropVariantClear`,
    // which would hand this pointer to `CoTaskMemFree`, and it points into a
    // `Vec` that Rust frees. `SetValue` copies what it is given, so nothing
    // here needs to outlive the call.
    let value = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    pwszVal: PWSTR(id.as_mut_ptr()),
                },
            }),
        },
    });

    unsafe { store.SetValue(&PKEY_APPUSERMODEL_ID, &*value) }
        .context("IPropertyStore::SetValue")?;
    unsafe { store.Commit() }.context("IPropertyStore::Commit")?;

    let file = wide(file);
    unsafe {
        link.cast::<IPersistFile>()?
            .Save(PCWSTR(file.as_ptr()), true)
    }
    .context("IPersistFile::Save")?;

    Ok(())
}

/// Whether a stored target and the running `.exe` are the same file.
///
/// Case-insensitively, because Windows paths are: the shortcut may hold
/// `C:\Tools\ctxmenu.exe` while `GetModuleFileNameW` answers with whatever
/// spelling the process was started with. Nothing beyond that -- no
/// canonicalising, no resolving of links. The question is whether the file
/// still names the running program, and both strings come from Windows itself.
fn same_file(stored: &str, running: &Path) -> bool {
    stored.to_lowercase() == running.to_string_lossy().to_lowercase()
}

/// A path as the NUL-terminated wide string every call below wants.
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// The same for a plain string.
fn wide_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A wide buffer Windows filled, up to the terminator it wrote.
///
/// The whole buffer would otherwise come back with a thousand NULs on the end,
/// and a comparison against that never matches anything.
fn cut_at_nul(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// A COM apartment for the length of one call, left exactly as it was found.
///
/// [`crate::notify`] deliberately initialises nothing: `windows-core` answers
/// `CO_E_NOTINITIALIZED` with `CoIncrementMTAUsage` and retries, so the first
/// WinRT call arranges its own apartment. `IShellLink` gets no such treatment
/// -- `CoCreateInstance` simply fails without an apartment -- and this runs
/// *before* the first WinRT call, so it has to make one itself.
///
/// The apartment is given back before returning, which leaves the thread
/// precisely as [`crate::notify`] documents it: in none, so that the toast
/// that follows still lands in the implicit MTA it prefers.
struct Apartment {
    /// Whether the initialisation was ours and is therefore ours to undo.
    ours: bool,
}

impl Apartment {
    /// `COINIT_APARTMENTTHREADED`, because it is the one that never collides.
    /// The `--favourite` thread is in no apartment and enters this one; a
    /// caller that already sits in an STA -- `run_native` owns the main thread
    /// in window mode -- gets `S_FALSE` and keeps the one it has. The other
    /// way round, asking for the MTA, would answer `RPC_E_CHANGED_MODE` there.
    /// `ShellLink` is registered `ThreadingModel = Both`, so it is at home
    /// either way and nothing is marshalled.
    fn enter() -> Self {
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };

        // `S_OK` and `S_FALSE` both raise the count `CoUninitialize` lowers,
        // so both are ours to balance. `RPC_E_CHANGED_MODE` is the third
        // answer: the thread is in the other apartment, COM works there too,
        // and undoing an initialisation somebody else made would pull the
        // apartment out from under its owner.
        Self {
            ours: result.is_ok(),
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.ours {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_lands_in_the_users_own_start_menu() {
        let path = shortcut_in(Path::new(r"C:\Users\alice\AppData\Roaming"));
        assert_eq!(
            path,
            Path::new(
                r"C:\Users\alice\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\ctxmenu.lnk"
            )
        );
    }

    #[test]
    fn the_property_key_was_typed_correctly() {
        // The one constant in this file that is copied out of a Microsoft
        // header instead of imported, so it is checked against the same GUID
        // written the other way round. A digit out of place here costs the
        // banner and says nothing about why.
        assert_eq!(
            PKEY_APPUSERMODEL_ID.fmtid,
            GUID::from_values(
                0x9f4c_2855,
                0x9f79,
                0x4b39,
                [0xa8, 0xd0, 0xe1, 0xd4, 0x2d, 0xe1, 0xd5, 0xf3]
            )
        );
        assert_eq!(PKEY_APPUSERMODEL_ID.pid, 5, "System.AppUserModel.ID");
    }

    #[test]
    fn a_target_is_compared_the_way_windows_compares_paths() {
        let running = Path::new(r"D:\Tools\ctxmenu.exe");

        assert!(same_file(r"D:\Tools\ctxmenu.exe", running));
        assert!(
            same_file(r"d:\tools\CTXMENU.EXE", running),
            "the spelling a process was started with is not the spelling on disk"
        );

        assert!(!same_file("", running), "an empty target is not this one");
        assert!(
            !same_file(r"D:\Tools\old\ctxmenu.exe", running),
            "a moved .exe leaves a shortcut that points at nothing"
        );
        assert!(
            !same_file(r"D:\Tools\ctxmenu.exe.bak", running),
            "a longer path that starts the same is a different file"
        );
    }

    #[test]
    fn two_processes_never_write_the_same_draft() {
        // Ten selected files start ten processes at the same moment; the point
        // of the draft is that each writes its own and they race only on the
        // rename.
        assert_ne!(draft_name(1234), draft_name(5678));

        let name = draft_name(1234);
        assert!(
            !name.contains('\\') && !name.contains('/'),
            "the draft lies beside the shortcut, or the rename is not atomic: {name}"
        );
        assert!(
            !name.ends_with(".lnk"),
            "a draft must not be read as a Start menu entry while it exists: {name}"
        );
    }

    #[test]
    fn a_filled_buffer_is_read_up_to_the_terminator() {
        let mut buffer = [0u16; 16];
        for (slot, unit) in buffer.iter_mut().zip("C:\\a.exe".encode_utf16()) {
            *slot = unit;
        }
        assert_eq!(cut_at_nul(&buffer), "C:\\a.exe");

        assert_eq!(cut_at_nul(&[0u16; 8]), "", "nothing written is not a path");
        assert_eq!(
            cut_at_nul(&"ab".encode_utf16().collect::<Vec<_>>()),
            "ab",
            "a buffer with no terminator at all is still a string"
        );
    }

    #[test]
    fn a_wide_string_ends_where_windows_expects_it_to() {
        assert_eq!(wide_str("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(wide(Path::new("a")), vec![b'a' as u16, 0]);
        assert_eq!(*wide_str("").last().unwrap(), 0);
    }
}
