//! Turning an icon reference into RGBA pixels.
//!
//! Every handle in this chain has to be released by hand, and almost none of
//! the release functions are `#[must_use]`, so the compiler will not remind
//! anyone. A process is capped at 10.000 GDI objects; a full scan touches
//! hundreds of icons, so a single forgotten `DeleteObject` on an error path
//! ends with an unusable application after a few rescans.
//!
//! Hence: every handle gets an RAII guard, and no early return can skip one.

use core::ffi::c_void;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC, GetObjectW, HBITMAP, HDC, HGDIOBJ,
    ReleaseDC, SelectObject,
};
use windows::Win32::UI::Shell::{ExtractIconExW, SHDefExtractIconW};
use windows::Win32::UI::WindowsAndMessaging::{
    DI_MASK, DI_NORMAL, DestroyIcon, DrawIconEx, GetIconInfo, HICON, ICONINFO,
};
use windows::core::PCWSTR;

use super::parse::IconRef;

/// Extraction size. Context menu icons are drawn small; 32 px still looks
/// right at 150 % scaling without paying for 256 px decodes on every row.
pub const ICON_SIZE: u32 = 32;

/// One decoded icon, ready to hand to egui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, as GDI produces it.
    pub pixels: Vec<u8>,
}

// ---------------------------------------------------------------------------
// RAII guards
// ---------------------------------------------------------------------------

/// Anything released with `DeleteObject`: the two bitmaps from `GetIconInfo`
/// and the DIB section.
struct OwnedGdiObject(HGDIOBJ);

impl Drop for OwnedGdiObject {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0);
            }
        }
    }
}

/// A DC from `CreateCompatibleDC`, which is released with `DeleteDC` — never
/// with `ReleaseDC`. Mixing the two leaks silently.
struct OwnedDc(HDC);

impl Drop for OwnedDc {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
}

/// A DC from `GetDC`, which is released with `ReleaseDC` — never `DeleteDC`.
struct BorrowedDc(HDC);

impl Drop for BorrowedDc {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                ReleaseDC(None, self.0);
            }
        }
    }
}

/// Puts a DC's previous object back.
///
/// Without this, `DeleteObject` on the DIB returns FALSE because it is still
/// selected, and the bitmap memory outlives the DC. `SelectObject` is not
/// `#[must_use]`, so dropping its return value compiles silently.
struct Selection(HDC, HGDIOBJ);

impl Drop for Selection {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.0, self.1);
        }
    }
}

/// An extracted icon handle, released with `DestroyIcon`.
struct OwnedIcon(HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Loads the icon a reference points at.
///
/// Returns `None` for anything that cannot be extracted — a missing file, a
/// resource ID that does not exist, an unreadable format. A missing icon is a
/// cosmetic problem and never a reason to fail a scan.
pub fn load(reference: &IconRef) -> Option<Rgba> {
    let icon = extract_hicon(&reference.path, reference.index, ICON_SIZE)?;
    to_rgba(icon.0)
}

fn extract_hicon(path: &str, index: i32, size: u32) -> Option<OwnedIcon> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let file = PCWSTR::from_raw(wide.as_ptr());

    unsafe {
        // Preferred because the target size is explicit, which gives
        // DPI-correct results instead of a scaled 32 px icon.
        let mut icon = HICON::default();
        let hr = SHDefExtractIconW(file, index, 0, Some(&mut icon), None, size);
        // S_FALSE is a *success* code meaning "no icon at that size", so the
        // handle has to be checked separately from the HRESULT.
        if hr.is_ok() && !icon.is_invalid() {
            return Some(OwnedIcon(icon));
        }

        let mut fallback = HICON::default();
        let count = ExtractIconExW(file, index, Some(&mut fallback), None, 1);
        if count > 0 && count != u32::MAX && !fallback.is_invalid() {
            Some(OwnedIcon(fallback))
        } else {
            None
        }
    }
}

/// The `GetIconInfo` → `CreateDIBSection` → `DrawIconEx` chain.
fn to_rgba(icon: HICON) -> Option<Rgba> {
    if icon.is_invalid() {
        return None;
    }

    unsafe {
        // GetIconInfo *creates* two bitmaps that the caller now owns. Nothing
        // in the signature says so — they are plain fields in a Copy struct.
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let _mask = OwnedGdiObject(HGDIOBJ::from(info.hbmMask));
        let _color = OwnedGdiObject(HGDIOBJ::from(info.hbmColor));

        let measured: HBITMAP = if info.hbmColor.is_invalid() {
            info.hbmMask
        } else {
            info.hbmColor
        };

        let mut bitmap = BITMAP::default();
        let written = GetObjectW(
            HGDIOBJ::from(measured),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut BITMAP as *mut c_void),
        );
        if written == 0 {
            return None;
        }

        let width = bitmap.bmWidth;
        // A monochrome icon has no colour bitmap; its mask stacks the AND and
        // XOR halves, so the real height is half of what is reported.
        let height = if info.hbmColor.is_invalid() {
            bitmap.bmHeight / 2
        } else {
            bitmap.bmHeight
        };
        if width <= 0 || height <= 0 {
            return None;
        }

        let screen = BorrowedDc(GetDC(None));
        if screen.0.is_invalid() {
            return None;
        }
        let memory = OwnedDc(CreateCompatibleDC(Some(screen.0)));
        if memory.0.is_invalid() {
            return None;
        }

        let mut header = BITMAPINFO::default();
        header.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        header.bmiHeader.biWidth = width;
        // Negative height means top-down, which matches the row order egui
        // expects. Nothing type-checks this; a positive value silently yields
        // an upside-down icon.
        header.bmiHeader.biHeight = -height;
        header.bmiHeader.biPlanes = 1;
        header.bmiHeader.biBitCount = 32;
        header.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib =
            CreateDIBSection(Some(screen.0), &header, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let _dib = OwnedGdiObject(HGDIOBJ::from(dib));
        if bits.is_null() {
            return None;
        }

        // Declared after the DIB guard so it drops first: the DIB must be
        // deselected before DeleteObject can succeed.
        let _selection = Selection(memory.0, SelectObject(memory.0, HGDIOBJ::from(dib)));

        let len = (width as usize) * (height as usize) * 4;
        std::ptr::write_bytes(bits as *mut u8, 0, len);
        DrawIconEx(memory.0, 0, 0, icon, width, height, 0, None, DI_NORMAL).ok()?;
        // GDI batches per thread. Without the flush the buffer may still be
        // half drawn, which shows up as intermittently black icons.
        let _ = GdiFlush();

        let mut pixels = std::slice::from_raw_parts(bits as *const u8, len).to_vec();

        // Windows delivers BGRA, egui wants RGBA.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        // Old 4-bpp and 8-bpp icons carry no alpha at all, so every alpha byte
        // is zero and the icon would render fully transparent. Rebuild alpha
        // from the AND mask: a set mask bit means transparent.
        if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
            std::ptr::write_bytes(bits as *mut u8, 0, len);
            DrawIconEx(memory.0, 0, 0, icon, width, height, 0, None, DI_MASK).ok()?;
            let _ = GdiFlush();

            let mask = std::slice::from_raw_parts(bits as *const u8, len);
            for (pixel, mask) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                pixel[3] = if mask[0] == 0 { 255 } else { 0 };
            }
        }

        Some(Rgba {
            width: width as u32,
            height: height as u32,
            pixels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::parse;

    fn reference(raw: &str) -> IconRef {
        parse::parse(raw).expect("test reference should parse")
    }

    #[test]
    fn a_shell32_resource_id_yields_pixels() {
        let icon = load(&reference(r"%SystemRoot%\system32\shell32.dll,-244"))
            .expect("shell32 always has icons");

        assert_eq!(icon.width, ICON_SIZE);
        assert_eq!(icon.height, ICON_SIZE);
        assert_eq!(icon.pixels.len() as u32, icon.width * icon.height * 4);
        assert!(
            icon.pixels.chunks_exact(4).any(|p| p[3] != 0),
            "a fully transparent icon means the alpha reconstruction failed"
        );
    }

    #[test]
    fn a_positional_index_works_too() {
        // A positional index counts from the front of the file; a resource ID
        // is negative and names the resource outright. That distinction is
        // what this test is about.
        //
        // What actually sits at a given position is the host's business, and
        // it is not the same everywhere: on this machine imageres.dll,0 is a
        // drawn icon, on the GitHub windows-latest runner it comes back fully
        // blank, which is what turned this test red the first time CI ever ran
        // (2026-08-20). So the shape is asserted for index 0 -- the positional
        // path resolved and produced a bitmap -- while the "something was
        // actually drawn" half asks the first few icons rather than betting the
        // suite on one of them.
        let icon =
            load(&reference(r"%SystemRoot%\system32\imageres.dll,0")).expect("imageres has icons");
        assert_eq!(icon.width, ICON_SIZE);
        assert_eq!(icon.height, ICON_SIZE);
        assert_eq!(icon.pixels.len() as u32, icon.width * icon.height * 4);

        let drawn = (0..8)
            .filter_map(|index| {
                load(&reference(&format!(
                    r"%SystemRoot%\system32\imageres.dll,{index}"
                )))
            })
            .any(|icon| icon.pixels.iter().any(|&byte| byte != 0));
        assert!(
            drawn,
            "none of the first eight icons in imageres.dll had a single non-zero byte"
        );
    }

    #[test]
    fn missing_files_and_ids_yield_nothing_rather_than_failing() {
        assert!(load(&reference(r"C:\does\not\exist.dll,-1")).is_none());
        assert!(load(&reference(r"C:\does\not\exist.dll")).is_none());
        // A resource ID that no shell32 has.
        assert!(load(&reference(r"%SystemRoot%\system32\shell32.dll,-999999")).is_none());
    }

    /// The GDI leak test for the handle guards, made automatic.
    ///
    /// A missing `DeleteObject` costs at least two objects per icon, so 300
    /// extractions would add 600 handles. The threshold is generous because
    /// the process also draws a window; what matters is that the number does
    /// not grow with the number of extractions.
    #[test]
    fn repeated_extraction_does_not_leak_gdi_objects() {
        use windows::Win32::System::Threading::{
            GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
        };

        let icon = reference(r"%SystemRoot%\system32\shell32.dll,-244");

        // Warm-up: the first calls populate caches inside the shell.
        for _ in 0..20 {
            let _ = load(&icon);
        }

        let before = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        for _ in 0..300 {
            let _ = load(&icon);
        }
        let after = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };

        assert!(
            after <= before + 32,
            "GDI objects grew from {before} to {after} over 300 extractions -- something leaks"
        );
    }
}
