//! Looking on GitHub for a newer release, and installing it.
//!
//! The program is a single file that people put wherever they like. There is no
//! installer to take care of updates and no service to do it in the background,
//! so it asks GitHub itself: what is the latest release, is it newer than this,
//! and does the file that comes down match the digest published beside it.
//!
//! # What is trusted, and why
//!
//! The whole chain rests on **TLS to `api.github.com` and `github.com`**.
//! WinHTTP validates the certificate against the Windows certificate store, the
//! same store every other program on the machine relies on. From that follows:
//!
//! * the release metadata is what the repository owner published,
//! * `checksums.txt` beside the assets is theirs too,
//! * and the archive is only installed when its SHA-256 matches the line in
//!   that file.
//!
//! An attacker who can forge a certificate the machine trusts defeats this, and
//! also defeats every browser on it. What this does *not* protect against is
//! GitHub itself, or someone who takes over the repository account — for that
//! the release would have to be signed with a key held outside GitHub, and the
//! public half compiled in here. That is a worthwhile second step and it is
//! deliberately not this first one: a signature with a key that lives in the
//! same account it protects would be ceremony, not security.
//!
//! # Replacing a running executable
//!
//! Windows will not overwrite a file that is being executed, but it *will*
//! rename one. So: move the running file aside, write the new one under the
//! original name, and delete the leftover on the next start. Nothing is
//! irreversible until the rename succeeds, and if anything after it fails the
//! old file is still there under its new name.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// Where releases are published.
const RELEASES: &str = "https://api.github.com/repos/corgan2222/context-manager/releases/latest";

/// What the running executable is renamed to while it is replaced. Deleted on
/// the next start.
const LEFTOVER: &str = "ctxmenu.exe.old";

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
    /// The archive to fetch.
    pub archive: String,
    /// Where the digests are published.
    pub checksums: String,
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

/// Reads a release into the two addresses an update needs.
///
/// Returns `None` when the release is not newer, or when it does not carry the
/// pair of files this expects. A release that looks half-published is left
/// alone rather than half-installed.
pub fn offer(release: &Release, running: &str) -> Option<Available> {
    if !is_newer(&release.tag_name, running) {
        return None;
    }

    let archive = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".zip") && a.name.starts_with("ctxmenu"))?;
    let checksums = release.assets.iter().find(|a| a.name == "checksums.txt")?;

    Some(Available {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        notes: match release.body.trim().is_empty() {
            true => release.name.clone(),
            false => release.body.clone(),
        },
        archive: archive.browser_download_url.clone(),
        checksums: checksums.browser_download_url.clone(),
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

/// SHA-256, by hand.
///
/// Written out rather than pulled in: the whole hash is sixty lines, and a
/// dependency added for one function is a dependency to keep updated for the
/// life of the program. Checked against the published test vectors below.
fn sha256(input: &[u8]) -> String {
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

    h.iter().map(|word| format!("{word:08x}")).collect()
}

/// Asks GitHub what the latest release is.
///
/// A plain GET against the public API: no token, no account, sixty requests an
/// hour per address, which is sixty more than this needs.
pub fn latest() -> Result<Release> {
    let headers = [
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
    ];
    let body = crate::webtool::http::fetch(RELEASES, &headers).context("GitHub")?;
    serde_json::from_slice(&body).context("\x1eAntwort von GitHub\x1fanswer from GitHub\x1d")
}

/// Puts the new executable in place of the running one.
///
/// The order matters and is the whole trick: Windows refuses to overwrite a
/// running image but allows renaming it, so the running file moves aside first.
/// Until that rename succeeds nothing has changed; after it, the old file is
/// still on disk under another name.
pub fn install(new_bytes: &[u8]) -> Result<std::path::PathBuf> {
    let running = std::env::current_exe().context("\x1eeigener Pfad\x1fown path\x1d")?;
    let directory = running
        .parent()
        .context("\x1ekein Verzeichnis\x1fno directory\x1d")?
        .to_path_buf();
    let leftover = directory.join(LEFTOVER);

    // A leftover from a previous update would block the rename.
    let _ = std::fs::remove_file(&leftover);

    std::fs::rename(&running, &leftover).with_context(|| {
        format!(
            "{} \x1ebeiseite legen\x1fmoving aside\x1d: schreibgeschützt? / read-only?",
            running.display()
        )
    })?;

    // From here on a failure has to put the old file back, or the program is
    // gone from the path it was started from.
    if let Err(error) = std::fs::write(&running, new_bytes) {
        let _ = std::fs::rename(&leftover, &running);
        bail!(
            "{} \x1eschreiben\x1fwriting\x1d: {error}",
            running.display()
        );
    }

    Ok(running)
}

/// Removes what the last update left behind. Called once at startup.
pub fn clean_up() {
    let Ok(running) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = running.parent() else {
        return;
    };
    // Failure is normal and uninteresting: on the very first start after an
    // update the old image may still be mapped for a moment.
    let _ = std::fs::remove_file(directory.join(LEFTOVER));
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

    #[test]
    fn a_release_without_both_files_is_left_alone() {
        let running = "1.0.0";
        assert!(offer(&release("1.1.0", &["ctxmenu.exe"]), running).is_none());
        assert!(offer(&release("1.1.0", &["checksums.txt"]), running).is_none());
        assert!(
            offer(
                &release("1.1.0", &["ctxmenu_1.1.0_windows_amd64.zip"]),
                running
            )
            .is_none(),
            "an archive without digests cannot be checked"
        );
    }

    #[test]
    fn a_complete_newer_release_offers_both_addresses() {
        let found = offer(
            &release(
                "v1.2.0",
                &[
                    "ctxmenu.exe",
                    "ctxmenu_1.2.0_windows_amd64.zip",
                    "checksums.txt",
                ],
            ),
            "1.1.0",
        )
        .expect("newer and complete");

        assert_eq!(found.version, "1.2.0", "without the v");
        assert!(found.archive.ends_with("ctxmenu_1.2.0_windows_amd64.zip"));
        assert!(found.checksums.ends_with("checksums.txt"));
        assert_eq!(found.notes, "Was neu ist");
    }

    #[test]
    fn the_running_version_is_never_offered_to_itself() {
        let same = release(
            &format!("v{}", crate::VERSION),
            &["ctxmenu_x_windows_amd64.zip", "checksums.txt"],
        );
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
