//! Browser öffnen und Dateien in die Zwischenablage legen.
//!
//! The clipboard is the part that makes web tools without an interface usable
//! at all: Squoosh, the TinyPNG page, remove.bg — none of them has an endpoint
//! to send a file to, but every one of them accepts Ctrl+V. So the file goes
//! on the clipboard as `CF_HDROP` (what Explorer puts there when you copy a
//! file) and, for images, additionally under the registered `PNG` format that
//! browsers map to an `image/png` paste.
//!
//! Both formats are set in one clipboard session because the two kinds of web
//! page want different things and there is no way to know in advance which one
//! is about to open.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{Owned, PCWSTR};

/// `CF_HDROP`, the format Explorer uses for copied files.
///
/// Written out rather than imported: the constant lives in `Win32_System_Ole`,
/// a large module pulled in for one number that has been 15 since Windows 3.1.
const CF_HDROP: u32 = 15;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Opens an address with whatever the user has set as their browser.
///
/// `ShellExecuteExW` rather than `ShellExecuteW`, matching
/// [`crate::elevation`]: the simple one answers with an `HINSTANCE` whose
/// value has to be compared against 32 to mean "failed", the other returns a
/// real error.
pub fn open(address: &str) -> Result<()> {
    if !address.starts_with("https://")
        && !address.starts_with("http://")
        && !address.starts_with("file:///")
    {
        // Anything else could be a local program or a protocol handler, and
        // this path exists to open web tools.
        bail!("Keine Web-Adresse / not a web address: {address}");
    }

    let verb = wide("open");
    let target = wide(address);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(target.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    unsafe { ShellExecuteExW(&mut info) }.with_context(|| format!("Öffnen / opening {address}"))
}

/// Says something to a user who has no console.
///
/// The `--favourite` mode is started by a click in the Explorer menu: there is
/// no window, no terminal, and nowhere for `errln!` to go. A message box is
/// the only channel that exists, and for a failed upload it is the difference
/// between "nothing happened" and knowing why.
pub fn report(title: &str, text: &str, kind: Report) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_SETFOREGROUND, MB_TOPMOST, MESSAGEBOX_STYLE,
        MessageBoxW,
    };

    let caption = wide(title);
    let body = wide(text);
    let icon = match kind {
        Report::Info => MB_ICONINFORMATION,
        Report::Error => MB_ICONERROR,
    };

    unsafe {
        MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MESSAGEBOX_STYLE(icon.0 | MB_SETFOREGROUND.0 | MB_TOPMOST.0),
        );
    }
}

pub enum Report {
    Info,
    Error,
}

/// Asks before a file leaves this machine.
///
/// Asked once per favourite, not once per click: the point is an informed
/// decision about *this service*, and repeating it every time would train
/// people to click it away. Answering no is a real answer — nothing is sent.
pub fn ask(title: &str, text: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MESSAGEBOX_STYLE,
        MessageBoxW,
    };

    let caption = wide(title);
    let body = wide(text);

    let answer = unsafe {
        MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MESSAGEBOX_STYLE(MB_YESNO.0 | MB_ICONWARNING.0 | MB_SETFOREGROUND.0 | MB_TOPMOST.0),
        )
    };
    answer == IDYES
}

/// Puts one file on the clipboard, the way Explorer's "copy" does.
///
/// For an image, the bytes are additionally offered under the registered
/// format `PNG`, which is what a browser turns into an `image/png` item in a
/// paste event. A page that wants a file gets the file, a page that wants an
/// image gets the image, and neither has to be guessed at beforehand.
pub fn copy_file_to_clipboard(file: &Path) -> Result<()> {
    let absolute = std::fs::canonicalize(file)
        .with_context(|| format!("{}", file.display()))?
        .to_string_lossy()
        // canonicalize hands back the \\?\ form; Explorer and browsers both
        // dislike it.
        .trim_start_matches(r"\\?\")
        .to_string();

    let drop_block = hdrop_block(&absolute)?;
    let image_bytes = image_payload(file);

    let _session = Clipboard::open()?;
    unsafe { EmptyClipboard() }.context("EmptyClipboard")?;

    // The file first: that is the format with the widest reach, and if the
    // second one fails the useful half is already in place.
    place(CF_HDROP, drop_block)?;

    if let Some(bytes) = image_bytes {
        let png = wide("PNG");
        let format = unsafe { RegisterClipboardFormatW(PCWSTR(png.as_ptr())) };
        if format != 0
            && let Ok(block) = global_block(&bytes)
        {
            // Best effort by design: a browser that ignores this still gets
            // the file, and no format is worth failing the whole operation.
            let _ = place(format, block);
        }
    }

    Ok(())
}

/// The raw bytes to offer as an image, if this file is one.
///
/// Only PNG is passed through unchanged, because only PNG can be: the
/// registered `PNG` clipboard format means PNG bytes. Converting a JPEG would
/// need a decoder, which this program deliberately does not carry.
fn image_payload(file: &Path) -> Option<Vec<u8>> {
    let is_png = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase() == "png")
        .unwrap_or(false);

    if !is_png {
        return None;
    }

    let bytes = std::fs::read(file).ok()?;
    // Ten megabytes of clipboard for a picture nobody may paste is not a
    // trade worth making silently.
    (bytes.len() <= 32 * 1024 * 1024).then_some(bytes)
}

/// Hands a block to the clipboard, which owns it from then on.
fn place(format: u32, block: Owned<HGLOBAL>) -> Result<()> {
    match unsafe { SetClipboardData(format, Some(HANDLE(block.0))) } {
        Ok(_) => {
            // The clipboard owns the memory now; dropping the guard would free
            // it a second time.
            std::mem::forget(block);
            Ok(())
        }
        Err(error) => Err(anyhow::Error::from(error).context(format!("SetClipboardData {format}"))),
    }
}

/// A moveable global block holding `bytes`.
fn global_block(bytes: &[u8]) -> Result<Owned<HGLOBAL>> {
    unsafe {
        let block = Owned::new(GlobalAlloc(GMEM_MOVEABLE, bytes.len()).context("GlobalAlloc")?);
        let target = GlobalLock(*block);
        if target.is_null() {
            bail!("GlobalLock lieferte NULL / returned NULL");
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target as *mut u8, bytes.len());
        let _ = GlobalUnlock(*block);
        Ok(block)
    }
}

/// A `DROPFILES` header followed by one double-NUL terminated wide path.
fn hdrop_block(path: &str) -> Result<Owned<HGLOBAL>> {
    use windows::Win32::UI::Shell::DROPFILES;

    let mut wide_path: Vec<u16> = path.encode_utf16().collect();
    wide_path.push(0); // ends this path
    wide_path.push(0); // ends the list

    let header = size_of::<DROPFILES>();
    let total = header + wide_path.len() * 2;

    unsafe {
        let block = Owned::new(GlobalAlloc(GMEM_MOVEABLE, total).context("GlobalAlloc")?);
        let base = GlobalLock(*block);
        if base.is_null() {
            bail!("GlobalLock lieferte NULL / returned NULL");
        }

        let files = DROPFILES {
            // Byte offset from the start of the block to the first path.
            pFiles: header as u32,
            pt: Default::default(),
            fNC: false.into(),
            // The paths are UTF-16, and saying otherwise turns them into
            // mojibake in the receiving application.
            fWide: true.into(),
        };
        // DROPFILES is packed(1); a plain write through a misaligned pointer
        // is undefined behaviour, and taking a reference to a packed field is
        // an outright compile error in edition 2024.
        std::ptr::write_unaligned(base as *mut DROPFILES, files);
        std::ptr::copy_nonoverlapping(
            wide_path.as_ptr(),
            (base as *mut u8).add(header) as *mut u16,
            wide_path.len(),
        );

        let _ = GlobalUnlock(*block);
        Ok(block)
    }
}

/// Holds the clipboard open and closes it whatever happens.
///
/// Without this, one `?` between `OpenClipboard` and `CloseClipboard` strands
/// a lock that is global to the whole desktop — every other application's copy
/// and paste stops working until this process ends.
struct Clipboard;

impl Clipboard {
    fn open() -> Result<Self> {
        // Another process holding it for a moment is normal, not an error:
        // every paste in every window takes the same lock.
        let mut last = None;
        for attempt in 0..8 {
            match unsafe { OpenClipboard(None) } {
                Ok(()) => return Ok(Clipboard),
                Err(error) => {
                    last = Some(error);
                    std::thread::sleep(std::time::Duration::from_millis(10 * (attempt + 1)));
                }
            }
        }

        Err(anyhow::Error::from(last.expect("at least one attempt"))
            .context("Zwischenablage bleibt belegt / the clipboard stays busy"))
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_are_opened() {
        // The point is not politeness: this runs from a context menu entry,
        // and a favourite whose address was edited into a program path would
        // otherwise start that program.
        assert!(open("C:\\Windows\\System32\\cmd.exe").is_err());
        assert!(open("cmd.exe /c del /s C:\\").is_err());
        assert!(open("javascript:alert(1)").is_err());
        assert!(open("ftp://example.invalid").is_err());
    }

    #[test]
    fn a_file_lands_on_the_clipboard_in_the_explorer_format() {
        let directory = std::env::temp_dir().join("ctxmenu_clipboard_test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp directory");
        let file = directory.join("Bild mit Leerzeichen.png");
        std::fs::write(&file, b"nicht wirklich ein PNG").expect("write");

        copy_file_to_clipboard(&file).expect("clipboard");

        // Read it back the way a receiving application would.
        use windows::Win32::System::DataExchange::{GetClipboardData, IsClipboardFormatAvailable};
        let _session = Clipboard::open().expect("open");
        unsafe {
            IsClipboardFormatAvailable(CF_HDROP).expect("CF_HDROP must be on offer");
            let handle = GetClipboardData(CF_HDROP).expect("data");
            let base = GlobalLock(HGLOBAL(handle.0));
            assert!(!base.is_null());

            let files: windows::Win32::UI::Shell::DROPFILES =
                std::ptr::read_unaligned(base as *const _);
            assert!(files.fWide.as_bool(), "paths must be wide");

            let start = (base as *const u8).add(files.pFiles as usize) as *const u16;
            let mut characters = Vec::new();
            let mut index = 0;
            loop {
                let value = *start.add(index);
                if value == 0 {
                    break;
                }
                characters.push(value);
                index += 1;
            }
            let path = String::from_utf16_lossy(&characters);

            assert!(path.ends_with("Bild mit Leerzeichen.png"), "got {path}");
            assert!(
                !path.starts_with(r"\\?\"),
                "the verbatim form is rejected by browsers: {path}"
            );
            assert!(
                Path::new(&path).is_absolute(),
                "a relative path is useless to the receiver: {path}"
            );

            let _ = GlobalUnlock(HGLOBAL(handle.0));
        }

        drop(_session);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn only_a_png_is_offered_as_an_image() {
        let directory = std::env::temp_dir().join("ctxmenu_clipboard_image_test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp directory");

        let png = directory.join("a.PNG");
        std::fs::write(&png, b"bytes").expect("write");
        assert!(
            image_payload(&png).is_some(),
            "the extension is not case sensitive"
        );

        let jpeg = directory.join("a.jpg");
        std::fs::write(&jpeg, b"bytes").expect("write");
        assert!(
            image_payload(&jpeg).is_none(),
            "a JPEG would have to be decoded first, and this program carries no decoder"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }
}
