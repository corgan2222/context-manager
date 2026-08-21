//! Turning an icon *address* into an icon *file*.
//!
//! Neither the Explorer nor the handler can show a web address as a menu
//! icon — the shell reads `.ico` files and PE resources from disk. What
//! they can show is a local copy: an `https://…/logo.png` typed or
//! templated into an icon field is fetched once when the form is saved,
//! wrapped into an `.ico` under `%LOCALAPPDATA%\ctxmenu\icons\`, and the
//! stored value becomes that local path.
//!
//! The wrap is byte work, not image work: since Vista an ICO frame may be a
//! raw PNG, and width and height sit uncompressed in the PNG's own header.
//! Nothing here decodes a pixel, which keeps `decisions/0027` intact — this
//! binary still has no image decoder.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};

/// Replaces a web address with the path of a local `.ico` copy.
///
/// Anything that is not `http(s)://` is already local and comes back
/// unchanged. Runs when a form is saved, never in the frame path: it is one
/// blocking download with WinHTTP's timeouts, same as every other network
/// step a click triggers.
pub fn localise(icon: &str) -> Result<String> {
    let trimmed = icon.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }

    // No headers: a logo is a public address by definition, and the key of a
    // service has no business in a request for one.
    let bytes = crate::webtool::http::download(trimmed, &[])?;
    let ico = match &bytes {
        png if png.starts_with(&[0x89, b'P', b'N', b'G']) => ico_from_png(png)?,
        ico if ico.starts_with(&[0, 0, 1, 0]) => bytes.clone(),
        _ => bail!(
            "\x1eDie Adresse liefert weder PNG noch ICO\
             \x1fthe address serves neither PNG nor ICO\x1d: {trimmed}"
        ),
    };

    let path = icon_path(trimmed)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("\x1eOrdner anlegen\x1fcreating\x1d: {}", parent.display()))?;
    }
    std::fs::write(&path, ico)
        .with_context(|| format!("\x1eSchreiben\x1fwriting\x1d: {}", path.display()))?;
    Ok(path.display().to_string())
}

/// Writes bytes that are already in hand as an `.ico`, and answers with its
/// path.
///
/// The other half of [`localise`], for a picture that never was on the web:
/// the catalogue carries its logos inside the binary, and the window can only
/// draw a file. Written once -- an existing file of the right name is taken as
/// the same picture, because the name comes from the catalogue and the
/// catalogue ships with the program.
pub fn stored(name: &str, png: &[u8]) -> Result<String> {
    let base = dirs::data_local_dir().context("kein LOCALAPPDATAno local data directory")?;
    let path = base
        .join("ctxmenu")
        .join("icons")
        .join(format!("catalogue-{name}.ico"));

    if path.exists() {
        return Ok(path.display().to_string());
    }

    let ico = ico_from_png(png)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Ordner anlegencreating: {}", parent.display()))?;
    }
    std::fs::write(&path, ico)
        .with_context(|| format!("Schreibenwriting: {}", path.display()))?;
    Ok(path.display().to_string())
}

/// `%LOCALAPPDATA%\ctxmenu\icons\<stem>-<hash>.ico`.
///
/// The stem keeps the file recognisable, the hash keeps two services with a
/// `logo.png` each from overwriting one another — and it is FNV-1a rather
/// than the standard hasher for the same reason `model::stable_id` is: the
/// name must not change across builds, or every update would strand the old
/// file and write a new one.
fn icon_path(url: &str) -> Result<PathBuf> {
    let base =
        dirs::data_local_dir().context("\x1ekein LOCALAPPDATA\x1fno local data directory\x1d")?;

    let stem = url
        .rsplit('/')
        .next()
        .unwrap_or("icon")
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or("icon");
    let stem: String = stem
        .chars()
        .map(
            |c| match c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                true => c,
                false => '_',
            },
        )
        .take(40)
        .collect();

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    Ok(base
        .join("ctxmenu")
        .join("icons")
        .join(format!("{stem}-{hash:08x}.ico")))
}

/// Wraps one PNG into a single-frame ICO container.
///
/// Width and height come straight out of the PNG's IHDR chunk — big-endian
/// `u32` at offsets 16 and 20, uncompressed by design. 256 is written as 0,
/// which is how the ICO format spells its maximum.
fn ico_from_png(png: &[u8]) -> Result<Vec<u8>> {
    if png.len() < 24 {
        bail!("\x1ePNG zu kurz für einen Kopf\x1fPNG too short for a header\x1d");
    }
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    if width == 0 || height == 0 || width > 256 || height > 256 {
        bail!(
            "\x1eAls Menü-Icon taugen bis zu 256 Pixel, das Bild hat {width}x{height}\
             \x1fa menu icon takes up to 256 pixels, this image is {width}x{height}\x1d"
        );
    }

    let entry_size = |value: u32| -> u8 {
        match value {
            256 => 0,
            other => other as u8,
        }
    };

    let mut out = Vec::with_capacity(22 + png.len());
    // ICONDIR: reserved, type 1 (icon), one image.
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    // ICONDIRENTRY: dimensions, no palette, planes 1, 32 bpp, size, offset.
    out.push(entry_size(width));
    out.push(entry_size(height));
    out.extend_from_slice(&[0, 0, 1, 0, 32, 0]);
    out.extend_from_slice(&(png.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(png);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real PNG that already lives in the repository — the handler's
    /// 48x48 logo, rendered from `app.ico` on 2026-08-20.
    const TINY_PNG: &[u8] = include_bytes!("../../../ctxmenu-handler/logo.png");

    #[test]
    fn a_png_becomes_an_ico_windows_can_open() {
        let ico = ico_from_png(TINY_PNG).expect("a valid PNG wraps");
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0], "ICONDIR for one icon");
        assert_eq!(ico[6], 48, "width from the IHDR");
        assert_eq!(&ico[22..], TINY_PNG, "the frame is the PNG, untouched");
    }

    #[test]
    fn junk_is_refused_not_wrapped() {
        assert!(ico_from_png(b"<html>not an image</html>").is_err());
        assert!(ico_from_png(b"\x89PNG").is_err(), "too short");
    }

    #[test]
    fn a_local_path_passes_through_untouched() {
        let local = r"C:\x\snapotter.ico";
        assert_eq!(localise(local).unwrap(), local);
        assert_eq!(localise("  C:\\x\\a.ico  ").unwrap(), r"C:\x\a.ico");
    }

    #[test]
    fn the_file_name_is_stable_and_tells_its_origin() {
        let a = icon_path("https://example.org/branding/logo-64.png").unwrap();
        let b = icon_path("https://example.org/branding/logo-64.png").unwrap();
        let other = icon_path("https://other.example/branding/logo-64.png").unwrap();

        assert_eq!(a, b, "same address, same file, across runs");
        assert_ne!(a, other, "same stem, different address, different file");
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("logo-64-"), "recognisable: {name}");
        assert!(name.ends_with(".ico"));
    }
}
