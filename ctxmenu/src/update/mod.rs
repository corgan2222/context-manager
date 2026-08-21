//! Looking on GitHub for a newer release, and installing it.
//!
//! The program is a single file that people put wherever they like. There is no
//! installer to take care of updates and no service to do it in the background,
//! so it asks GitHub itself: what is the latest release, is it newer than this,
//! is it signed with the author's key, and does the file that comes down match
//! the digest published beside it.
//!
//! # What is trusted, and why
//!
//! Two independent things, and an attacker needs both.
//!
//! **TLS to `api.github.com` and `github.com`.** WinHTTP validates the
//! certificate against the Windows certificate store, the same store every
//! other program on the machine relies on. That is what says the release
//! metadata and the files beside it came from GitHub at all.
//!
//! **The signature over `checksums.txt`.** The private half lives in a
//! repository secret and is used by one step of `release.yml`; the public half
//! is compiled into this binary — see [`signature`]. That is what says the
//! release came from the author, and it is the half that still holds when the
//! repository account does not: somebody who can publish a release cannot sign
//! one. The earlier version of this file called such a signature "ceremony,
//! not security", and it was right about the shape it described — a key kept
//! in the account it protects. This key is not kept there.
//!
//! Everything hangs off that one signed file: `checksums.txt` names every asset
//! and its SHA-256, so a digest that matches a signed line is an asset the
//! author published. An unsigned release, or one whose signature does not
//! verify, is not offered at all — including every release published before
//! this feature existed, which is correct and deliberate. Nothing is installed
//! that this chain did not check end to end.
//!
//! # Replacing a running executable
//!
//! Windows will not overwrite a file that is being executed, but it *will*
//! rename one. So the new file is written beside the old one first, and then
//! two renames swap them:
//!
//! 1. write the new bytes to `ctxmenu.exe.new`,
//! 2. rename the running file to `ctxmenu.exe.old`,
//! 3. rename `ctxmenu.exe.new` over the original name.
//!
//! The download is on disk and complete before anything moves, so the window in
//! which the original name has no file behind it is the gap between two renames
//! — microseconds, not the length of a write. If step 3 fails anyway, step 2 is
//! undone. `ctxmenu.exe.old` is what the next start deletes; see [`clean_up`].

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

pub mod signature;

/// Where releases are published.
const RELEASES: &str = "https://api.github.com/repos/corgan2222/context-manager/releases/latest";

/// The asset that is installed. The bare executable rather than the zip beside
/// it: unpacking a zip means a DEFLATE decoder, and a DEFLATE decoder written
/// for this one job would be three hundred lines of bit shuffling to save a
/// megabyte of download, once. The zip stays in the release for people who
/// download it in a browser.
const EXE_ASSET: &str = "ctxmenu.exe";

/// The list of digests, and the signature over it.
const CHECKSUMS_ASSET: &str = "checksums.txt";
const SIGNATURE_ASSET: &str = "checksums.txt.sig";

/// The archive's name, which is the one place a version number appears inside
/// the signed file. See [`names_this_version`].
fn archive_named(version: &str) -> String {
    format!("ctxmenu_{version}_windows_amd64.zip")
}

/// What the running executable is renamed to while it is replaced. Deleted on
/// the next start.
const LEFTOVER: &str = "ctxmenu.exe.old";

/// Where the download is written before the swap.
const INCOMING: &str = "ctxmenu.exe.new";

/// What a released executable may plausibly weigh.
///
/// Not a security measure -- the digest is that -- but the difference between
/// "the digest does not match" and a sentence that names the actual problem.
/// Measured 2026-08-20: the released binary is 9.7 MB. The floor is set far
/// below that on purpose -- a GitHub error page is a few kilobytes of HTML and
/// would otherwise arrive as a digest mismatch, and nothing here should have an
/// opinion about how large a future release may be.
const PLAUSIBLE_SIZE: std::ops::RangeInclusive<usize> = 1_000_000..=64_000_000;

/// One release, as much of it as matters here.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// A newer release than the one running, if there is one.
#[derive(Debug, Clone, PartialEq)]
pub struct Available {
    pub version: String,
    pub notes: String,
    /// The executable to fetch.
    pub exe: String,
    /// Where the digests are published.
    pub checksums: String,
    /// Where the signature over those digests is published.
    pub signature: String,
}

/// Compares two versions the way this project numbers them.
///
/// `X.Y.Z`, each part a number, a leading `v` allowed on either side. Anything
/// that does not parse counts as *not newer*: an update offered because a tag
/// was misread is worse than an update missed.
pub fn is_newer(candidate: &str, running: &str) -> bool {
    let (Some(new), Some(old)) = (parts(candidate), parts(running)) else {
        return false;
    };
    new > old
}

fn parts(version: &str) -> Option<(u32, u32, u32)> {
    let trimmed = version.trim().trim_start_matches(['v', 'V']);
    // Everything from a `-` or `+` on is a pre-release or build tag; this
    // project does not publish those, and comparing them properly is a whole
    // specification of its own.
    let core = trimmed
        .split(['-', '+'])
        .next()
        .unwrap_or(trimmed)
        .trim_end();
    let mut numbers = core.split('.');
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;
    let patch = numbers.next()?.parse().ok()?;
    // A fourth part means this is not the shape we know.
    if numbers.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Reads a release into the three addresses an update needs.
///
/// Returns `None` when the release is not newer, or when it does not carry all
/// three files this expects. A release that looks half-published is left alone
/// rather than half-installed — and one published before signing existed has no
/// `checksums.txt.sig`, so it lands in the same place: not offered. That is not
/// a gap, it is the point. An asset set that may arrive short is one an
/// attacker gets to shorten.
pub fn offer(release: &Release, running: &str) -> Option<Available> {
    if !is_newer(&release.tag_name, running) {
        return None;
    }

    let asset = |wanted: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == wanted)
            .map(|a| a.browser_download_url.clone())
    };

    Some(Available {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        notes: match release.body.trim().is_empty() {
            true => release.name.clone(),
            false => release.body.clone(),
        },
        exe: asset(EXE_ASSET)?,
        checksums: asset(CHECKSUMS_ASSET)?,
        signature: asset(SIGNATURE_ASSET)?,
    })
}

/// Finds the digest of one file in a `sha256sum`-style list.
///
/// The format is fixed by the release workflow: an uppercase digest, exactly two
/// spaces, the file name. Parsed leniently on whitespace anyway, because a file
/// this small is not worth a parse error, but the digest itself has to be
/// sixty-four hex characters or it is not one.
pub fn digest_of<'a>(checksums: &'a str, name: &str) -> Option<&'a str> {
    checksums.lines().find_map(|line| {
        let (digest, file) = line.trim().split_once(char::is_whitespace)?;
        if file.trim() != name {
            return None;
        }
        let looks_right = digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit());
        looks_right.then_some(digest)
    })
}

/// Whether these bytes are what the digest says they should be.
pub fn matches_digest(bytes: &[u8], expected: &str) -> bool {
    sha256(bytes).eq_ignore_ascii_case(expected)
}

/// SHA-256 as the sixty-four characters people compare by eye.
///
/// Public because a second caller wants exactly this form: a service that
/// checks files names its page after the file's own digest, so
/// `webtool::built` reaches for it when an address template says `{sha256}`.
pub fn sha256(input: &[u8]) -> String {
    sha256_bytes(input)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// SHA-256, by hand.
///
/// Written out rather than pulled in: the whole hash is sixty lines, and a
/// dependency added for one function is a dependency to keep updated for the
/// life of the program. Checked against the published test vectors below.
///
/// The raw digest rather than the hex string, because [`signature::verify`]
/// hands exactly these thirty-two bytes to CNG.
fn sha256_bytes(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut message = input.to_vec();
    let bits = (input.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut digest = [0u8; 32];
    for (slot, word) in digest.chunks_exact_mut(4).zip(h) {
        slot.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// The two headers every request in this file carries.
fn headers() -> [crate::favourites::Header; 2] {
    [
        crate::favourites::Header {
            // The API refuses a request without one, and a name is more use to
            // whoever reads GitHub's logs than a browser string would be.
            name: "User-Agent".into(),
            value: format!("ctxmenu/{}", crate::VERSION),
        },
        crate::favourites::Header {
            name: "Accept".into(),
            value: "application/vnd.github+json".into(),
        },
    ]
}

/// Asks GitHub what the latest release is.
///
/// A plain GET against the public API: no token, no account, sixty requests an
/// hour per address, which is sixty more than this needs.
pub fn latest() -> Result<Release> {
    let body = crate::webtool::http::fetch(RELEASES, &headers()).context("GitHub")?;
    serde_json::from_slice(&body).context("\x1eAntwort von GitHub\x1fanswer from GitHub\x1d")
}

/// What asking GitHub came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing newer exists.
    Current,
    /// Newer, and everything an update needs is there.
    Available(Box<Available>),
    /// Newer, but the release is missing at least one of the three files.
    /// Carries the version so the window can name it.
    Incomplete(String),
}

/// The whole question in one call. Belongs on a worker thread — it is a network
/// request, and the frame path may not wait for one.
pub fn check() -> Result<Outcome> {
    Ok(examine(&latest()?, crate::VERSION))
}

/// Sorts one release into the three answers.
///
/// The distinction between *current* and *incomplete* is not pedantry, it is
/// the ten minutes after every release: the draft is published by hand, and the
/// workflow that builds and attaches the assets starts at that click and takes
/// its time. In that window `api.github.com` already reports the new tag and
/// the assets are not there yet. Folding that into "you are up to date" would
/// be telling the user something false at the one moment it is most likely to
/// be asked.
pub fn examine(release: &Release, running: &str) -> Outcome {
    if !is_newer(&release.tag_name, running) {
        return Outcome::Current;
    }
    match offer(release, running) {
        Some(found) => Outcome::Available(Box::new(found)),
        None => Outcome::Incomplete(release.tag_name.trim_start_matches(['v', 'V']).to_string()),
    }
}

/// Fetches the new executable and proves it is the right one before returning
/// a single byte of it to the caller.
///
/// In order, and the order is the whole security argument:
///
/// 1. `checksums.txt`, as bytes.
/// 2. `checksums.txt.sig`, which has to verify against the key compiled into
///    this binary. Nothing below this line runs if it does not.
/// 3. The digest for `ctxmenu.exe`, read out of the file that was just proved
///    to be the author's.
/// 4. The executable, accepted only when its SHA-256 is that digest.
pub fn download(available: &Available) -> Result<Vec<u8>> {
    let checksums = crate::webtool::http::fetch_following(&available.checksums, &headers())
        .context(CHECKSUMS_ASSET)?;
    let sig = crate::webtool::http::fetch_following(&available.signature, &headers())
        .context(SIGNATURE_ASSET)?;
    let sig = String::from_utf8(sig)
        .context("\x1eSignatur ist kein Text\x1fthe signature is not text\x1d")?;

    let key = signature::release_key()?;
    signature::verify(&checksums, &sig, &key).context(
        "\x1edie Signatur dieser Fassung stimmt nicht\x1fthis release's signature does not check out\x1d",
    )?;

    // Only now is this file worth reading.
    let checksums = String::from_utf8(checksums)
        .context("\x1echecksums.txt ist kein Text\x1fchecksums.txt is not text\x1d")?;
    let expected = digest_of(&checksums, EXE_ASSET).with_context(|| {
        format!("\x1ekein Eintrag f\u{fc}r\x1fno entry for\x1d {EXE_ASSET} in {CHECKSUMS_ASSET}")
    })?;

    // The signature says these digests are the author's. It does not say which
    // *version* they belong to -- and without that, a release is a container
    // somebody else can fill. See [`names_this_version`].
    ensure!(
        names_this_version(&checksums, &available.version),
        "\x1edie signierte Pr\u{fc}fsummenliste geh\u{f6}rt nicht zu Fassung {}\x1fthe signed list of checksums does not belong to version {}\x1d",
        available.version,
        available.version
    );

    let bytes =
        crate::webtool::http::fetch_following(&available.exe, &headers()).context(EXE_ASSET)?;
    ensure!(
        PLAUSIBLE_SIZE.contains(&bytes.len()) && bytes.starts_with(b"MZ"),
        "\x1e{} Bytes und kein Windows-Programm \u{2014} das ist nicht die .exe\x1f{} bytes and not a Windows executable \u{2014} that is not the .exe\x1d",
        bytes.len(),
        bytes.len()
    );
    ensure!(
        matches_digest(&bytes, expected),
        "\x1edie heruntergeladene Datei hat nicht die ver\u{f6}ffentlichte Pr\u{fc}fsumme\x1fthe downloaded file does not have the published checksum\x1d"
    );

    Ok(bytes)
}

/// Does this signed list of checksums belong to the version being offered?
///
/// It has to be asked, and the reason is the one attack the signature exists
/// for. The signature covers the digests and nothing else: not the tag, not the
/// release, not the name it is published under. Somebody who takes over the
/// GitHub account cannot sign anything — but they can create a release tagged
/// `v99.0.0` and attach the `checksums.txt`, `checksums.txt.sig` and
/// `ctxmenu.exe` of an old, genuinely signed 1.5.0. Every check downstream
/// passes: the signature is real, the digest matches the file. The user is
/// quietly moved *back* to a version with a hole in it, and because 99.0.0 is
/// still newer than what they now run, they are offered it again on every
/// start.
///
/// What closes it is already in the file. The release workflow lists every
/// asset, and one of them carries the version in its name:
/// `ctxmenu_1.5.0_windows_amd64.zip`. A line for
/// `ctxmenu_{running_offer}_windows_amd64.zip` can only be in a list that was
/// signed for exactly that version, so requiring one binds the signature to the
/// version. The archive itself is never downloaded; only its name is needed.
fn names_this_version(checksums: &str, version: &str) -> bool {
    digest_of(checksums, &archive_named(version)).is_some()
}

/// Puts the new executable in place of the running one, and says where it now
/// is.
///
/// The order is the trick, and it is written out at the top of this file:
/// Windows refuses to overwrite a running image but allows renaming it, so the
/// new bytes land beside it first and two renames do the swap. The only moment
/// the original name has no file behind it is the gap between those two
/// renames.
pub fn install(new_bytes: &[u8]) -> Result<std::path::PathBuf> {
    let running = std::env::current_exe().context("\x1eeigener Pfad\x1fown path\x1d")?;
    install_at(&running, new_bytes)?;
    Ok(running)
}

/// The swap itself, on a named path rather than on this process's own.
///
/// Split out from [`install`] for one reason: `current_exe()` is not something
/// a test can be lied to about, and the file swap is the one part of this
/// module where a mistake costs the user their program rather than an error
/// message. On a path it can be driven in a temporary folder, which is what the
/// tests at the bottom of this file do.
pub fn install_at(running: &std::path::Path, new_bytes: &[u8]) -> Result<()> {
    let directory = running
        .parent()
        .context("\x1ekein Verzeichnis\x1fno directory\x1d")?
        .to_path_buf();
    let leftover = directory.join(LEFTOVER);
    let incoming = directory.join(INCOMING);

    // Against a second copy of this program doing the same thing at the same
    // time. Two swaps interleaved can leave neither file under the original
    // name — each one deletes the other's working files by name before it
    // starts. Nothing is refused if the lock cannot be had; it is a narrow
    // window and a lock nobody can take is not a reason to decline an update.
    let _swap = SwapLock::take();

    // Leftovers from a previous attempt would block both renames.
    let _ = std::fs::remove_file(&leftover);
    let _ = std::fs::remove_file(&incoming);

    // The one step whose failure really does mean the folder: writing a new
    // file into it. Both renames below fail with the same access denied for
    // half a dozen other reasons — a virus scanner holding the file open is the
    // common one — so only this one gets to name the folder as the cause.
    std::fs::write(&incoming, new_bytes).with_context(|| unwritable(&directory))?;

    if let Err(error) = rename_patiently(running, &leftover) {
        let _ = std::fs::remove_file(&incoming);
        return Err(error.context(format!(
            "{} \x1ebeiseite legen\x1fmoving aside\x1d",
            running.display()
        )));
    }

    // From here a failure has to put the old file back, or the program is gone
    // from the path it was started from — and if even that fails, the user has
    // to be told where their program now is. Anything less would leave them
    // looking at "renaming: access denied" with no .exe and no idea that one is
    // lying there under another name.
    if let Err(error) = rename_patiently(&incoming, running) {
        let _ = std::fs::remove_file(&incoming);
        return Err(match rename_patiently(&leftover, running) {
            Ok(()) => error.context(format!(
                "{} \x1e\u{2014} die alte Fassung steht wieder an ihrem Platz\x1f\u{2014} the old version is back in place\x1d",
                running.display()
            )),
            Err(_) => error.context(format!(
                "{} \x1e\u{2014} und das Zur\u{fc}cklegen ist auch misslungen: das Programm liegt jetzt als {} daneben und muss von Hand zur\u{fc}ckbenannt werden\x1f\u{2014} and putting it back failed as well: the program is now lying beside it as {}, and has to be renamed back by hand\x1d",
                running.display(),
                LEFTOVER,
                LEFTOVER
            )),
        });
    }

    Ok(())
}

/// Renames, and tries again for a moment when Windows says no.
///
/// A rename of an executable fails often enough for reasons that pass on their
/// own: a scanner opened the freshly written file, the shell is still holding
/// the old image, an indexer walked past. [`clean_up`] has always known this
/// about deleting; the two renames here need it more, because one of them
/// failing is what leaves the program without a file at its own path.
fn rename_patiently(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..5u64 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1)));
    }
    Err(anyhow::Error::new(
        last.expect("the loop ran at least once"),
    ))
}

/// Held while the executable is being swapped, so that two copies of this
/// program do not do it at once.
///
/// `Local\`, like the one in `webtool::batch`: the copies to coordinate are
/// this user's, in this session. A handle that could not be had is simply
/// `None` — see the note at the call site on why that is not refused.
struct SwapLock(Option<windows::Win32::Foundation::HANDLE>);

impl SwapLock {
    fn take() -> SwapLock {
        use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

        let Ok(handle) = (unsafe {
            CreateMutexW(
                None,
                false,
                &windows::core::HSTRING::from(r"Local\ctxmenu-self-update"),
            )
        }) else {
            return SwapLock(None);
        };

        // Thirty seconds: a swap is two renames and a write, so anything
        // waiting longer than this is waiting on a download that is not going
        // to finish.
        let waited = unsafe { WaitForSingleObject(handle, 30_000) };
        match waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
            true => SwapLock(Some(handle)),
            false => {
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
                SwapLock(None)
            }
        }
    }
}

impl Drop for SwapLock {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe {
                let _ = windows::Win32::System::Threading::ReleaseMutex(handle);
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
        }
    }
}

/// The sentence for the one failure that is not a bug but a location.
///
/// A copy under `C:\Program Files` belongs to the administrators, and every
/// step of the swap fails there with the same access denied — which on its own
/// reads like a broken update rather than like a folder this account may not
/// write to.
fn unwritable(directory: &std::path::Path) -> String {
    format!(
        "{} \u{2014} \x1ehier darf dieses Konto nicht schreiben; das Programm aus einem eigenen Ordner starten oder die neue Fassung von Hand herunterladen\x1fthis account may not write here; run the program from a folder of your own, or download the new version by hand\x1d",
        directory.display()
    )
}

/// Starts the freshly installed program and hands it this run's arguments.
///
/// The caller closes the window right after. Two copies overlap for the moment
/// in between, and that overlap is what makes the old one able to start the new
/// one at all: the new file is already at the original path, and the old image
/// is only still mapped, not still on it.
///
/// The arguments are passed on because the new copy should open the window this
/// one had — `--tab`, `--lang` and `--window` are how it got here. The working
/// directory is deliberately *not* inherited: this program is started from
/// Explorer's context menu, and a child that keeps the clicked folder as its
/// working directory holds that folder open for as long as it runs.
pub fn relaunch(exe: &std::path::Path) -> Result<()> {
    let mut command = std::process::Command::new(exe);
    command.args(std::env::args().skip(1));
    if let Some(directory) = exe.parent() {
        command.current_dir(directory);
    }
    command
        .spawn()
        .with_context(|| format!("{} \x1estarten\x1fstarting\x1d", exe.display()))?;
    Ok(())
}

/// Removes what the last update left behind. Called once at startup.
///
/// A few attempts rather than one: the new copy is started by the old one and
/// may well reach this line while the old image is still mapped, and a single
/// try would then leave the file lying there until some later start. Costs
/// nothing on an ordinary start, where neither file exists and the loop is not
/// entered at all.
pub fn clean_up() {
    let Ok(running) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = running.parent() else {
        return;
    };
    clean_up_at(directory);
}

/// The same, in a named folder. Split out for the same reason as
/// [`install_at`]: a test cannot be lied to about `current_exe()`.
pub fn clean_up_at(directory: &std::path::Path) {
    // `ctxmenu.exe.old` only, deliberately, and `ctxmenu.exe.new` deliberately
    // not. The incoming file exists for the seconds between a download
    // finishing and the swap completing — and this runs at every start, so a
    // second copy started in those seconds would delete the first copy's
    // download out from under it. What `install_at` leaves behind if it dies
    // mid-swap is cleared by the next `install_at`, which removes both names
    // before it begins.
    let path = directory.join(LEFTOVER);
    for attempt in 0..5u64 {
        if !path.exists() {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            break;
        }
        // Failure is normal and uninteresting; what is left behind is a stale
        // file, not a problem.
        std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            name: format!("ctxmenu {tag}"),
            body: "Was neu ist".into(),
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: (*name).into(),
                    browser_download_url: format!("https://example.invalid/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn a_release_is_newer_only_when_its_numbers_say_so() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("v1.1.1", "1.1.0"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"), "9 is not more than 1.0");
        assert!(!is_newer("1.1.0", "v1.1.0"), "a leading v is not a version");
    }

    #[test]
    fn a_tag_that_is_not_a_version_never_counts_as_newer() {
        // An update offered because a tag was misread is worse than one missed.
        for tag in ["nightly", "1.2", "1.2.3.4", "", "v", "1.2.x", "latest"] {
            assert!(!is_newer(tag, "1.0.0"), "{tag:?} must not offer an update");
        }
    }

    /// Everything a release has to carry to be installable.
    const COMPLETE: &[&str] = &[
        "ctxmenu.exe",
        "ctxmenu_1.2.0_windows_amd64.zip",
        "checksums.txt",
        "checksums.txt.sig",
    ];

    #[test]
    fn a_release_missing_any_one_of_the_three_files_is_left_alone() {
        // Each run drops exactly one of the three the updater needs, so a
        // future packaging change that forgets one cannot pass unnoticed.
        for missing in [EXE_ASSET, CHECKSUMS_ASSET, SIGNATURE_ASSET] {
            let assets: Vec<&str> = COMPLETE
                .iter()
                .copied()
                .filter(|name| *name != missing)
                .collect();
            assert!(
                offer(&release("1.1.0", &assets), "1.0.0").is_none(),
                "without {missing} nothing may be offered"
            );
        }
        // And the zip on its own, which is what the release looked like before
        // there was anything to check it against.
        assert!(
            offer(
                &release("1.1.0", &["ctxmenu_1.1.0_windows_amd64.zip"]),
                "1.0.0"
            )
            .is_none()
        );
    }

    #[test]
    fn a_release_published_before_signing_existed_is_never_offered() {
        // v1.3.2 and everything before it: exe, zip and digests, no signature.
        // Refusing them is the point -- an asset set that may arrive short is
        // one an attacker gets to shorten.
        let older = release(
            "v9.9.9",
            &[
                "ctxmenu.exe",
                "ctxmenu_9.9.9_windows_amd64.zip",
                "checksums.txt",
            ],
        );
        assert!(offer(&older, "1.0.0").is_none());
    }

    #[test]
    fn a_complete_newer_release_offers_all_three_addresses() {
        let found = offer(&release("v1.2.0", COMPLETE), "1.1.0").expect("newer and complete");

        assert_eq!(found.version, "1.2.0", "without the v");
        assert!(found.exe.ends_with("/ctxmenu.exe"), "got {}", found.exe);
        assert!(found.checksums.ends_with("/checksums.txt"));
        assert!(found.signature.ends_with("/checksums.txt.sig"));
        assert_eq!(found.notes, "Was neu ist");
    }

    #[test]
    fn the_zip_is_never_what_gets_installed() {
        // The archive is there for people downloading in a browser. Picking it
        // up here would mean a DEFLATE decoder, and the .exe is right beside
        // it under the same signed digest list.
        let found = offer(&release("v1.2.0", COMPLETE), "1.1.0").expect("complete");
        assert!(!found.exe.ends_with(".zip"), "got {}", found.exe);
    }

    #[test]
    fn an_asset_whose_name_only_looks_right_is_not_taken() {
        // `starts_with` and `ends_with` were how the first version matched,
        // and both would accept these.
        let decoys = release(
            "v1.2.0",
            &[
                "ctxmenu.exe.sig",
                "ctxmenu.exe.asc",
                "checksums.txt.asc",
                "not-ctxmenu.exe",
            ],
        );
        assert!(offer(&decoys, "1.1.0").is_none());
    }

    #[test]
    fn a_release_whose_assets_are_still_uploading_is_told_apart_from_no_release() {
        // The ten minutes after every publish: the tag is announced, the
        // workflow is still building. Saying "you are up to date" there would
        // be false at the one moment the question is most likely to be asked.
        let announced = release("v9.9.9", &[]);
        assert_eq!(
            examine(&announced, "1.0.0"),
            Outcome::Incomplete("9.9.9".into())
        );

        let ready = release("v9.9.9", COMPLETE);
        assert!(matches!(examine(&ready, "1.0.0"), Outcome::Available(_)));

        assert_eq!(examine(&ready, "9.9.9"), Outcome::Current);
        assert_eq!(
            examine(&release("v0.1.0", COMPLETE), "1.0.0"),
            Outcome::Current
        );
    }

    #[test]
    fn the_running_version_is_never_offered_to_itself() {
        let same = release(&format!("v{}", crate::VERSION), COMPLETE);
        assert!(offer(&same, crate::VERSION).is_none());
    }

    #[test]
    fn a_digest_is_found_by_name_and_only_when_it_looks_like_one() {
        let list = "\
E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855  ctxmenu.exe
0000111122223333444455556666777788889999AAAABBBBCCCCDDDDEEEEFFFF  ctxmenu_1.2.0_windows_amd64.zip
nonsense  broken.zip
";
        assert_eq!(
            digest_of(list, "ctxmenu.exe"),
            Some("E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855")
        );
        assert!(digest_of(list, "ctxmenu_1.2.0_windows_amd64.zip").is_some());
        assert_eq!(digest_of(list, "broken.zip"), None, "not 64 hex digits");
        assert_eq!(digest_of(list, "absent.zip"), None);
    }

    #[test]
    fn a_signed_list_from_another_version_is_refused() {
        // The attack `names_this_version` exists for: a real signature over a
        // real list, published under a tag it does not belong to.
        let list = "\
AAAA1111222233334444555566667777888899990000AAAABBBBCCCCDDDDEEEE  ctxmenu.exe
BBBB1111222233334444555566667777888899990000AAAABBBBCCCCDDDDEEEE  ctxmenu_1.5.0_windows_amd64.zip
";
        assert!(names_this_version(list, "1.5.0"), "its own version");
        assert!(
            !names_this_version(list, "99.0.0"),
            "a list signed for 1.5.0 must not pass as 99.0.0"
        );
        assert!(!names_this_version(list, "1.5.1"));
        assert!(!names_this_version(list, ""));
        // A list that names no archive at all cannot vouch for any version.
        assert!(!names_this_version(
            "AAAA1111222233334444555566667777888899990000AAAABBBBCCCCDDDDEEEE  ctxmenu.exe\n",
            "1.5.0"
        ));
    }

    /// A folder of this test's own, named after the test that asked for it.
    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("ctxmenu-install-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a folder in TEMP");
        directory
    }

    #[test]
    fn the_swap_puts_the_new_file_at_the_old_name_and_keeps_the_old_one() {
        let directory = scratch("swap");
        let running = directory.join("ctxmenu.exe");
        std::fs::write(&running, b"the old program").expect("write");

        install_at(&running, b"the new program").expect("the swap");

        assert_eq!(std::fs::read(&running).expect("read"), b"the new program");
        assert_eq!(
            std::fs::read(directory.join(LEFTOVER)).expect("the old one is kept"),
            b"the old program",
            "nothing is thrown away until the next start"
        );
        assert!(
            !directory.join(INCOMING).exists(),
            "the incoming name is free again"
        );

        // And the next start clears the leftover, which is the other half of
        // the arrangement.
        clean_up_at(&directory);
        assert!(!directory.join(LEFTOVER).exists());

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn leftovers_from_a_failed_attempt_do_not_block_the_next_one() {
        // Both names occupied by junk, which is what a swap interrupted
        // half-way leaves behind. The next attempt has to go through anyway.
        let directory = scratch("leftovers");
        let running = directory.join("ctxmenu.exe");
        std::fs::write(&running, b"the old program").expect("write");
        std::fs::write(directory.join(LEFTOVER), b"junk from last time").expect("write");
        std::fs::write(directory.join(INCOMING), b"half a download").expect("write");

        install_at(&running, b"the new program").expect("the swap");

        assert_eq!(std::fs::read(&running).expect("read"), b"the new program");
        assert_eq!(
            std::fs::read(directory.join(LEFTOVER)).expect("read"),
            b"the old program",
            "the leftover is the file that was just replaced, not the older junk"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_swap_that_cannot_start_leaves_everything_as_it_was() {
        // No such folder, so writing the incoming file fails at the first step.
        // The point is what is *not* touched: nothing has been renamed yet.
        let missing = std::env::temp_dir()
            .join(format!("ctxmenu-absent-{}", std::process::id()))
            .join("ctxmenu.exe");
        let error = install_at(&missing, b"the new program")
            .expect_err("a folder that is not there")
            .to_string();
        assert!(
            error.contains("write here") || error.contains("schreiben"),
            "the message names the folder as the problem, got: {error}"
        );
    }

    #[test]
    fn cleaning_up_never_removes_a_download_in_progress() {
        // `ctxmenu.exe.new` belongs to whichever copy is installing right now.
        // A second copy starting in those seconds must not delete it -- that
        // was how two instances could leave the folder without an .exe at all.
        let directory = scratch("cleanup");
        std::fs::write(directory.join(LEFTOVER), b"yesterday").expect("write");
        std::fs::write(directory.join(INCOMING), b"a download under way").expect("write");

        clean_up_at(&directory);

        assert!(!directory.join(LEFTOVER).exists(), "the old one goes");
        assert!(
            directory.join(INCOMING).exists(),
            "the incoming one stays -- it is not this process's to delete"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_hash_agrees_with_the_published_test_vectors() {
        // The three vectors from FIPS 180-4 and the empty string, so a mistake
        // in the implementation cannot pass unnoticed.
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // Long enough to need more than one block, which is where a wrong
        // padding shows up.
        assert_eq!(
            sha256(&vec![b'a'; 1_000]),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn a_download_is_only_accepted_when_it_matches_its_digest() {
        let bytes = b"ctxmenu";
        let right = sha256(bytes);
        assert!(matches_digest(bytes, &right));
        assert!(
            matches_digest(bytes, &right.to_uppercase()),
            "the workflow writes them upper case"
        );
        assert!(!matches_digest(bytes, &sha256(b"ctxmenu ")));
        assert!(!matches_digest(bytes, ""));
    }
}
