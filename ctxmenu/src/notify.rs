//! Saying something to a user who has no window and no console.
//!
//! `--favourite` is started by a click in the Explorer menu. There is no
//! terminal for `errln!` and no window to hang a dialog on, so whatever this
//! program has to say has to find its own channel.
//!
//! # Why a notification and not a message box
//!
//! The registry command of a web tool favourite ends in `"%1"`, and Windows
//! reads that as "once per file": ten selected files start ten processes.
//! Every one of them used to finish with a `MessageBoxW`, so ten modal windows
//! piled up on top of each other and each one had to be clicked away by hand.
//! A notification is handed to the shell and forgotten; nothing waits for a
//! click, and ten of them do not stack.
//!
//! # Why the tray icon and not the WinRT toast
//!
//! The obvious route, `ToastNotificationManager::CreateToastNotifier(aumid)`,
//! wants a registered AppUserModelID, and an AUMID wants a shortcut in the
//! Start menu or a packaged identity. This program is a single portable `.exe`
//! that installs nothing — that is a promise the README makes — so it has
//! neither. `Shell_NotifyIconW` with `NIF_INFO` needs no identity at all: the
//! shell hands the balloon to the Windows 10 notification platform and files
//! it under an identity of its own making.
//!
//! Measured on this machine (Windows 10 Pro 19045) with a prototype that
//! logged every return value and every callback the shell sent back:
//!
//! * `NIM_ADD`, `NIM_SETVERSION` and `NIM_MODIFY(NIF_INFO)` all answered
//!   `true` from a plain `.exe` in a temp directory — no shortcut, no AUMID,
//!   no package. The shell takes the balloon from a program it knows nothing
//!   about.
//! * Ten processes at once: ten `NIM_ADD` and ten `NIM_MODIFY` answered
//!   `true`, and the shell sent back all ten `NIN_BALLOONSHOW` within 660 ms.
//!   They do not collide, because no GUID is set — the identity of an icon is
//!   `(hWnd, uID)`, and each process brings its own window.
//! * **The balloon outlives its icon, so the process may exit at once.**
//!   Deleting the icon 6 ms after `NIM_MODIFY` did not silence it: the shell
//!   still answered `NIN_BALLOONSHOW` 21 ms later. It has the text before
//!   `Shell_NotifyIconW` returns. Hence no message pump here, no artificial
//!   sleep, and nothing that could change an `ExitCode` or leave a process
//!   behind.
//!
//! # What a balloon is not: a message that waits
//!
//! A `NIF_INFO` balloon is **transient**. It is shown and then it is over; it
//! is never written to the Action Center store, so there is nothing to scroll
//! back to. Measured by reading the shell's own `wpndatabase.db`, with a real
//! WinRT toast fired into the same query as a control: the control turns up in
//! the store within seconds, the balloon never does — neither while the tray
//! icon still exists nor afterwards.
//!
//! The consequence is worth stating plainly, because it decides what
//! [`crate::webtool::shell::report`] does with a failure: **a balloon Windows
//! chooses not to draw is gone.** With Focus Assist switched on the shell
//! still answers `true` and the platform still records the attempt, but the
//! user sees nothing and has nothing to look up afterwards. That is why the
//! caller writes every error to the log before it gets here — the log, not the
//! Action Center, is what survives a notification nobody saw.
//!
//! Not verified on this machine: that the balloon is *drawn* when Windows is
//! willing to draw it. Focus Assist was switched on throughout, and it
//! swallowed a properly registered WinRT toast in exactly the same way, so
//! there was no state in which either channel could be photographed. What was
//! confirmed is everything up to the shell's own door.
//!
//! # A message-only window is enough
//!
//! The window exists only to be an address: `Shell_NotifyIconW` needs an
//! `HWND` to hang the icon on. `HWND_MESSAGE` gives one that is never drawn,
//! never appears on the taskbar and never steals focus — which is exactly what
//! a program started by a right-click needs.

use anyhow::{Context as _, Result, bail};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HICON, HWND_MESSAGE, IDI_APPLICATION,
    IDI_INFORMATION, LoadIconW, RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

/// Which of the two things happened.
///
/// The same split the message box drew with `MB_ICONINFORMATION` and
/// `MB_ICONERROR`, so a failed upload still looks different from a finished
/// one at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

/// The class name of the invisible window the icon hangs on.
const CLASS: PCWSTR = w!("ctxmenu_notify_host");

/// How many `u16` the shell reads out of each field of `NOTIFYICONDATAW`.
///
/// Written down rather than taken from the struct so the truncation can be
/// tested without building a `NOTIFYICONDATAW` — and so a reader can see what
/// the numbers are. They are fixed by the Win32 header.
const TITLE_CAPACITY: usize = 64;
const TEXT_CAPACITY: usize = 256;
const TIP_CAPACITY: usize = 128;

/// Shows one Windows notification. Both texts are already cut to one language.
///
/// An `Err` means the shell refused the notification — during a restart of
/// Explorer, or on a desktop that has no notification area at all. The caller
/// is expected to have something else to say it with; see
/// [`crate::webtool::shell::report`].
pub fn show(title: &str, text: &str, level: Level) -> Result<()> {
    let host = Host::new()?;

    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: host.window,
        // No `NIF_GUID` on purpose. A GUID would give every one of the ten
        // processes the same identity, and the second one would be told the
        // icon already exists. Without it the shell keys on `(hWnd, uID)`, and
        // each process brings its own window.
        uID: 1,
        uFlags: NIF_ICON | NIF_TIP,
        hIcon: icon(),
        ..Default::default()
    };
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    fill(&mut data.szTip, "ctxmenu", TIP_CAPACITY);

    // The documented order, and it matters: the icon has to exist before a
    // balloon can be attached to it, and the version has to be declared before
    // the shell knows which behaviour is being asked for.
    if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        bail!("\x1eShell_NotifyIcon NIM_ADD abgelehnt\x1frefused NIM_ADD\x1d");
    }

    // From here on the icon exists, so every way out has to take it with it.
    let _icon = TrayIcon(data);

    // Not fatal on its own: without it the shell falls back to the old
    // behaviour, which still shows the balloon.
    let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };

    data.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
    fill(&mut data.szInfoTitle, title, TITLE_CAPACITY);
    fill(&mut data.szInfo, text, TEXT_CAPACITY);
    data.dwInfoFlags = match level {
        Level::Info => NIIF_INFO,
        Level::Error => NIIF_ERROR,
    };

    if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
        bail!("\x1eShell_NotifyIcon NIM_MODIFY abgelehnt\x1frefused NIM_MODIFY\x1d");
    }

    // `_icon` and `host` are dropped here, which removes the icon and the
    // window. That is deliberate and measured: the shell has the text by now,
    // and holding on any longer would only leave an icon in the tray for a
    // process that has nothing left to do.
    Ok(())
}

/// This program's own icon, so the notification is recognisable.
///
/// Resource 1 is what `winresource` writes `assets/app.ico` to. If it is ever
/// numbered differently, or the icon is missing from a build, the notification
/// is still worth more than the icon on it — hence the two fallbacks rather
/// than an error.
fn icon() -> HICON {
    unsafe {
        if let Ok(instance) = GetModuleHandleW(None)
            // `MAKEINTRESOURCE`: a resource is named either by a string or by a
            // number squeezed into the pointer itself. `without_provenance` says
            // that in Rust's own words -- this is a bare address that is never
            // dereferenced, not a pointer to a `u16` somewhere.
            && let Ok(own) = LoadIconW(
                Some(instance.into()),
                PCWSTR(std::ptr::without_provenance(1)),
            )
        {
            return own;
        }
        LoadIconW(None, IDI_INFORMATION)
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default()
    }
}

/// Copies `text` into one of the fixed-size fields, NUL-terminated.
fn fill(field: &mut [u16], text: &str, capacity: usize) {
    let units = clipped(text, capacity);
    field[..units.len()].copy_from_slice(&units);
}

/// `text` as NUL-terminated UTF-16, never longer than `capacity` units.
///
/// The shell reads a fixed number of `u16` out of each field and stops at the
/// first zero, so a text that does not fit has to be shortened here rather
/// than run past the end of the array. Cut along `chars`, never along `u16`:
/// an emoji or anything else outside the basic plane is a surrogate *pair*,
/// and half a pair is not a character — it arrives on screen as a replacement
/// box, which is a worse way to lose the last letter than simply not showing
/// it.
fn clipped(text: &str, capacity: usize) -> Vec<u16> {
    let room = capacity.saturating_sub(1);
    let mut out: Vec<u16> = Vec::with_capacity(capacity);

    for character in text.chars() {
        if out.len() + character.len_utf16() > room {
            break;
        }
        let mut buffer = [0u16; 2];
        out.extend_from_slice(character.encode_utf16(&mut buffer));
    }

    out.push(0);
    out
}

/// Hands every message straight back to Windows.
///
/// A thunk rather than `DefWindowProcW` itself: the `windows` crate exposes
/// that one as a Rust function, and a window class wants a raw `system` one.
/// Nothing here reacts to a message — no `NIF_MESSAGE` is asked for and
/// nothing pumps this queue, because the shell has the text before
/// `Shell_NotifyIconW` returns.
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The invisible window the icon is addressed by, destroyed on the way out.
struct Host {
    window: HWND,
}

impl Host {
    fn new() -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;

            // Registering the same class twice fails, and this may be called
            // more than once in a process. The failure is ignored rather than
            // guarded by a flag: the only reason it can fail here is that the
            // class is already there, which is precisely the state wanted.
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            let _ = RegisterClassW(&class);

            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                w!("ctxmenu"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                // Message-only: never drawn, never on the taskbar, never in
                // the way of whatever the user is doing.
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
            .context("\x1eunsichtbares Fenster\x1fmessage-only window\x1d")?;

            Ok(Self { window })
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.window);
        }
    }
}

/// Holds the added icon and removes it however this function is left.
///
/// Without it an early return between `NIM_ADD` and the end would leave an
/// icon in the notification area belonging to a process that has already
/// exited — the ghost icon that only disappears once the mouse passes over it.
struct TrayIcon(NOTIFYICONDATAW);

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_is_copied_with_its_terminator() {
        assert_eq!(clipped("ok", 64), vec![b'o' as u16, b'k' as u16, 0]);
        assert_eq!(clipped("", 64), vec![0]);
    }

    #[test]
    fn a_long_text_is_cut_to_the_field() {
        // 300 characters into the 256-unit body field: 255 of them plus the
        // terminator, and not one unit more -- the shell reads a fixed number
        // of u16 out of that array.
        let long = "a".repeat(300);
        let units = clipped(&long, TEXT_CAPACITY);
        assert_eq!(units.len(), TEXT_CAPACITY);
        assert_eq!(units.last(), Some(&0));
        assert!(units[..units.len() - 1].iter().all(|&u| u == b'a' as u16));
    }

    #[test]
    fn a_surrogate_pair_is_never_cut_in_half() {
        // Every character here is two u16, so an odd amount of room has to
        // leave one unit unused rather than write half a character. A lone
        // surrogate is not text: it reaches the screen as a replacement box.
        let text = "\u{1F600}\u{1F600}\u{1F600}";
        let units = clipped(text, 6); // room for 5 units = two pairs and a gap
        assert_eq!(units.len(), 5, "two pairs plus the terminator");
        assert_eq!(units.last(), Some(&0));
        for unit in &units[..4] {
            assert!(
                (0xD800..=0xDFFF).contains(unit),
                "the two pairs survive whole"
            );
        }
    }

    #[test]
    fn german_text_survives_the_cut() {
        // The body is measured in u16, not in bytes: umlauts are one unit
        // each, so a sentence that fits must not be shortened for being
        // multi-byte in UTF-8.
        let text = "Öffnen fehlgeschlagen: Größe überschritten";
        let units = clipped(text, TEXT_CAPACITY);
        assert_eq!(units.len(), text.chars().count() + 1);
    }

    #[test]
    fn a_capacity_of_one_still_terminates() {
        // Not a real field size, but the arithmetic must not underflow.
        assert_eq!(clipped("text", 1), vec![0]);
        assert_eq!(clipped("text", 0), vec![0]);
    }

    #[test]
    fn the_title_field_is_smaller_than_the_body() {
        // A reminder in test form: the two fields do not share a limit, and
        // filling a title with the body's capacity would write past its end.
        const { assert!(TITLE_CAPACITY < TEXT_CAPACITY) };
        let long = "b".repeat(100);
        assert_eq!(clipped(&long, TITLE_CAPACITY).len(), TITLE_CAPACITY);
    }
}
