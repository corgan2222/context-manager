//! Does this release carry the author's signature?
//!
//! TLS already says the bytes came from GitHub. This says they came from
//! whoever holds the private key — which is the one thing GitHub cannot say
//! for itself, and the one thing that still holds if the repository account is
//! taken over. Without it the update chain trusts an account; with it, a key
//! that never touches the account at all.
//!
//! # The shape of it
//!
//! * `checksums.txt` names every asset and its SHA-256. The updater already
//!   refuses a download whose digest is not in there.
//! * `checksums.txt.sig` is an RSA PKCS#1 v1.5 signature over the *bytes* of
//!   that file, SHA-256, written out as base64 — one file, one signature, and
//!   every asset covered through its digest.
//! * The public half is [`RELEASE_KEY`], compiled in from
//!   `ctxmenu/release-signing.pub.pem`. The private half lives in the
//!   `RELEASE_SIGNING_KEY` secret of the repository and is used by one step of
//!   `release.yml`; see `tools/new-release-key.ps1` for how the pair was made.
//!
//! # Why the arithmetic is not written out here
//!
//! The SHA-256 next door is sixty lines and has published test vectors.
//! RSA is neither: a constant-time modular exponentiation over a 4096-bit
//! number is a piece of cryptography, and a hand-rolled one is a liability,
//! not an achievement. Windows has been doing it since Vista, so
//! [`BCryptVerifySignature`] does the arithmetic and this file does the
//! parsing — base64, DER, and the key blob CNG wants. Those *are* worth
//! writing out: they are pure functions over bytes, and every one of them is
//! tested below.

use anyhow::{Result, bail, ensure};
use windows::Win32::Security::Cryptography::{
    BCRYPT_KEY_HANDLE, BCRYPT_PAD_PKCS1, BCRYPT_PKCS1_PADDING_INFO, BCRYPT_RSA_ALG_HANDLE,
    BCRYPT_RSAKEY_BLOB, BCRYPT_RSAPUBLIC_BLOB, BCRYPT_RSAPUBLIC_MAGIC, BCRYPT_SHA256_ALGORITHM,
    BCryptDestroyKey, BCryptImportKeyPair, BCryptVerifySignature,
};

/// The public half of the release signing key, as published in this
/// repository.
///
/// Compiled in rather than fetched: a key that arrives over the same channel
/// as the thing it vouches for vouches for nothing. Replacing it is a breaking
/// change for every installed copy — an older build carries the older key and
/// will decline a release signed with the new one, which is the correct
/// behaviour and also the reason not to rotate this on a whim.
pub const RELEASE_KEY: &str = include_str!("../../release-signing.pub.pem");

/// The smallest key this accepts. Anything shorter is not a key, it is a
/// typo.
const MINIMUM_BITS: usize = 2048;

/// An RSA public key: two big-endian numbers and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    /// The modulus, big-endian, without the sign byte DER puts in front.
    modulus: Vec<u8>,
    /// The public exponent, big-endian. Almost always the three bytes of
    /// 65537.
    exponent: Vec<u8>,
}

impl PublicKey {
    /// How long the modulus is, in bits — the number people mean by "a 4096
    /// bit key".
    pub fn bits(&self) -> usize {
        match self.modulus.first() {
            None => 0,
            Some(first) => (self.modulus.len() - 1) * 8 + (8 - first.leading_zeros() as usize),
        }
    }
}

/// The key this build trusts.
pub fn release_key() -> Result<PublicKey> {
    parse_pem(RELEASE_KEY)
}

/// Reads a PEM-armoured RSA public key.
///
/// Both shapes are accepted, because both are one command away and nobody
/// should have to remember which:
///
/// * `BEGIN PUBLIC KEY` — SubjectPublicKeyInfo, what `openssl genpkey` and
///   .NET's `ExportSubjectPublicKeyInfoPem` write.
/// * `BEGIN RSA PUBLIC KEY` — PKCS#1, the bare pair of numbers.
///
/// A private key is refused by name. It would not parse anyway, but the error
/// someone gets for pointing at the wrong file should say what is wrong rather
/// than "unexpected tag".
pub fn parse_pem(pem: &str) -> Result<PublicKey> {
    if pem.contains("PRIVATE KEY") {
        bail!("\x1edas ist ein privater Schl\u{fc}ssel\x1fthat is a private key\x1d");
    }

    let body = armour(pem)?;
    let der = base64_decode(body)?;
    parse_der(&der)
}

/// The base64 between the two dashed lines.
fn armour(pem: &str) -> Result<&str> {
    let start = pem
        .find("-----BEGIN ")
        .and_then(|at| pem[at..].find('\n').map(|end| at + end + 1));
    let (Some(start), Some(end)) = (start, pem.rfind("-----END ")) else {
        bail!("\x1ekeine PEM-Datei\x1fnot a PEM file\x1d");
    };
    ensure!(end > start, "\x1eEND vor BEGIN\x1fEND before BEGIN\x1d");
    Ok(&pem[start..end])
}

/// Reads the modulus and exponent out of DER.
///
/// Handles both of the shapes [`parse_pem`] accepts: an outer `SEQUENCE` whose
/// first element is another `SEQUENCE` is SubjectPublicKeyInfo and the numbers
/// sit one `BIT STRING` further in; one that starts with an `INTEGER` is PKCS#1
/// and the numbers are right there.
fn parse_der(der: &[u8]) -> Result<PublicKey> {
    const SEQUENCE: u8 = 0x30;
    const INTEGER: u8 = 0x02;
    const BIT_STRING: u8 = 0x03;
    /// `1.2.840.113549.1.1.1`, rsaEncryption, as it appears in DER including
    /// its tag and length. Compared whole rather than decoded: this is the
    /// only OID that may appear here, so recognising it is the entire job.
    const RSA_ENCRYPTION: &[u8] = &[
        0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01,
    ];

    let mut outer = Reader::new(der).element(SEQUENCE)?;
    // Nothing may follow the outer SEQUENCE. Trailing bytes are how one file
    // gets to mean two things.
    ensure!(
        outer.trailing.is_empty(),
        "\x1eBytes hinter dem Schl\u{fc}ssel\x1ftrailing bytes after the key\x1d"
    );

    let mut numbers = match outer.body.peek() {
        Some(SEQUENCE) => {
            let algorithm = outer.body.element(SEQUENCE)?.body;
            ensure!(
                algorithm.rest.starts_with(RSA_ENCRYPTION),
                "\x1enicht RSA\x1fnot RSA\x1d"
            );
            let bits = outer.body.element(BIT_STRING)?.body;
            // The first byte of a BIT STRING counts the unused bits of the
            // last one. For a wrapped structure that is always zero.
            let (unused, inner) = bits
                .rest
                .split_first()
                .ok_or_else(|| anyhow::anyhow!("\x1eleeres BIT STRING\x1fempty BIT STRING\x1d"))?;
            ensure!(
                *unused == 0,
                "\x1eangebrochenes BIT STRING\x1fBIT STRING is not whole bytes\x1d"
            );
            Reader::new(inner).element(SEQUENCE)?.body
        }
        Some(INTEGER) => outer.body,
        _ => bail!("\x1eunbekannte Schl\u{fc}sselform\x1funknown key shape\x1d"),
    };

    let modulus = unsigned(numbers.element(INTEGER)?.body.rest)?;
    let exponent = unsigned(numbers.element(INTEGER)?.body.rest)?;

    let key = PublicKey { modulus, exponent };
    ensure!(
        key.bits() >= MINIMUM_BITS,
        "\x1eSchl\u{fc}ssel zu kurz\x1fkey too short\x1d: {} bit",
        key.bits()
    );
    ensure!(
        !key.exponent.is_empty(),
        "\x1ekein Exponent\x1fno exponent\x1d"
    );
    Ok(key)
}

/// Strips the sign byte DER puts in front of a positive number whose top bit
/// is set, and refuses a negative one.
fn unsigned(bytes: &[u8]) -> Result<Vec<u8>> {
    let trimmed = bytes.iter().position(|b| *b != 0).map(|at| &bytes[at..]);
    let Some(trimmed) = trimmed else {
        bail!("\x1eZahl ist null\x1fnumber is zero\x1d");
    };
    // After stripping leading zeroes a top bit that is still set means the
    // original had no sign byte, so it encoded a negative number.
    ensure!(
        bytes.len() > trimmed.len() || trimmed[0] < 0x80,
        "\x1enegative Zahl im Schl\u{fc}ssel\x1fnegative number in the key\x1d"
    );
    Ok(trimmed.to_vec())
}

/// Walks DER one element at a time.
struct Reader<'a> {
    rest: &'a [u8],
}

/// One DER element, plus whatever followed it.
struct Element<'a> {
    body: Reader<'a>,
    trailing: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { rest: bytes }
    }

    fn peek(&self) -> Option<u8> {
        self.rest.first().copied()
    }

    /// Reads the next element, which must carry `tag`, and advances past it.
    fn element(&mut self, tag: u8) -> Result<Element<'a>> {
        let (actual, after_tag) = self
            .rest
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("\x1eDER zu Ende\x1fDER ended\x1d"))?;
        ensure!(
            *actual == tag,
            "\x1eDER: {tag:#04x} erwartet, {actual:#04x} gelesen\x1fDER: expected {tag:#04x}, read {actual:#04x}\x1d"
        );

        let (length, after_length) = length_of(after_tag)?;
        ensure!(
            length <= after_length.len(),
            "\x1eDER: L\u{e4}nge {length} \u{fc}ber das Ende hinaus\x1fDER: length {length} runs past the end\x1d"
        );
        let (body, trailing) = after_length.split_at(length);
        self.rest = trailing;
        Ok(Element {
            body: Reader::new(body),
            trailing,
        })
    }
}

/// A DER length: one byte below 0x80, otherwise a count of bytes and then that
/// many, big-endian.
fn length_of(bytes: &[u8]) -> Result<(usize, &[u8])> {
    let (first, rest) = bytes
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("\x1eDER: keine L\u{e4}nge\x1fDER: no length\x1d"))?;
    if *first < 0x80 {
        return Ok((*first as usize, rest));
    }

    let count = (*first & 0x7f) as usize;
    // 0x80 is the indefinite length, which DER forbids; more than four bytes
    // of length is a file this program has no business reading.
    ensure!(
        (1..=4).contains(&count),
        "\x1eDER: unm\u{f6}gliche L\u{e4}nge\x1fDER: impossible length\x1d"
    );
    ensure!(
        rest.len() >= count,
        "\x1eDER: L\u{e4}nge abgeschnitten\x1fDER: length is cut off\x1d"
    );
    let (digits, rest) = rest.split_at(count);
    let length = digits
        .iter()
        .fold(0usize, |total, digit| (total << 8) | *digit as usize);
    Ok((length, rest))
}

/// Base64 to bytes, strictly.
///
/// Whitespace anywhere is ignored, because PEM and a signature file both wrap
/// at seventy-six characters. Everything else is refused: a character outside
/// the alphabet, padding in the middle, a length that cannot be right, or
/// leftover bits that are not zero. Leniency in a decoder that feeds a
/// signature check is how two readers come to disagree about what a file says.
pub fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits = 0u32;
    let mut padded = false;

    for byte in text.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            padded = true;
            continue;
        }
        ensure!(
            !padded,
            "\x1eBase64: Zeichen hinter der F\u{fc}llung\x1fbase64: character after the padding\x1d"
        );

        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => bail!(
                "\x1eBase64: unerlaubtes Zeichen {:?}\x1fbase64: character {:?} is not allowed\x1d",
                byte as char,
                byte as char
            ),
        };

        accumulator = (accumulator << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }

    // Six left over is one stray character: four base64 digits are three
    // bytes, and no valid ending leaves that much behind.
    ensure!(
        bits != 6,
        "\x1eBase64: L\u{e4}nge geht nicht auf\x1fbase64: length does not work out\x1d"
    );
    ensure!(
        accumulator == 0,
        "\x1eBase64: Restbits sind nicht null\x1fbase64: leftover bits are not zero\x1d"
    );
    Ok(out)
}

/// Is this signature the author's, over exactly these bytes?
///
/// `signature` is the content of `checksums.txt.sig`: base64, wrapped or not.
/// Returns `Ok(())` only when CNG says the signature verifies; every other
/// outcome — a malformed file, a key CNG will not import, a digest that does
/// not match — is an error, and the caller installs nothing.
pub fn verify(message: &[u8], signature: &str, key: &PublicKey) -> Result<()> {
    let signature = base64_decode(signature)?;
    // PKCS#1 v1.5 signatures are exactly as long as the modulus. Checked here
    // so that a truncated download says so, rather than arriving at CNG as an
    // opaque STATUS_INVALID_SIGNATURE.
    ensure!(
        signature.len() == key.modulus.len(),
        "\x1eSignatur ist {} Bytes lang, erwartet {}\x1fsignature is {} bytes, expected {}\x1d",
        signature.len(),
        key.modulus.len(),
        signature.len(),
        key.modulus.len()
    );

    let handle = ImportedKey::import(key)?;
    let hash = super::sha256_bytes(message);
    let padding = BCRYPT_PKCS1_PADDING_INFO {
        pszAlgId: BCRYPT_SHA256_ALGORITHM,
    };

    let status = unsafe {
        BCryptVerifySignature(
            handle.0,
            Some(std::ptr::from_ref(&padding).cast()),
            &hash,
            &signature,
            BCRYPT_PAD_PKCS1,
        )
    };
    ensure!(
        status.is_ok(),
        "\x1eSignatur passt nicht ({status:?})\x1fsignature does not match ({status:?})\x1d"
    );
    Ok(())
}

/// A key CNG holds for us, destroyed when it goes out of scope.
struct ImportedKey(BCRYPT_KEY_HANDLE);

impl ImportedKey {
    /// Hands the two numbers to CNG in the layout it asks for: the header,
    /// then the exponent, then the modulus, both big-endian and both without
    /// leading zeroes.
    fn import(key: &PublicKey) -> Result<Self> {
        let header = BCRYPT_RSAKEY_BLOB {
            Magic: BCRYPT_RSAPUBLIC_MAGIC,
            BitLength: key.bits() as u32,
            cbPublicExp: key.exponent.len() as u32,
            cbModulus: key.modulus.len() as u32,
            // A public key has neither prime.
            cbPrime1: 0,
            cbPrime2: 0,
        };

        let mut blob = Vec::with_capacity(size_of::<BCRYPT_RSAKEY_BLOB>() + key.modulus.len() + 8);
        // `repr(C)` and six `u32` fields: the struct is its own byte layout,
        // and reading it as bytes is what the API expects to be handed.
        blob.extend_from_slice(unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(&header).cast::<u8>(),
                size_of::<BCRYPT_RSAKEY_BLOB>(),
            )
        });
        blob.extend_from_slice(&key.exponent);
        blob.extend_from_slice(&key.modulus);

        let mut handle = BCRYPT_KEY_HANDLE::default();
        let status = unsafe {
            BCryptImportKeyPair(
                // The provider handle that is always there, rather than
                // opening and closing one around a single verification.
                BCRYPT_RSA_ALG_HANDLE,
                None,
                BCRYPT_RSAPUBLIC_BLOB,
                &mut handle,
                &blob,
                0,
            )
        };
        ensure!(
            status.is_ok(),
            "\x1eCNG nimmt den Schl\u{fc}ssel nicht ({status:?})\x1fCNG will not take the key ({status:?})\x1d"
        );
        Ok(ImportedKey(handle))
    }
}

impl Drop for ImportedKey {
    fn drop(&mut self) {
        // Nothing to do about a failure here, and nothing that depends on it.
        let _ = unsafe { BCryptDestroyKey(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key pair made for these tests alone and thrown away afterwards, so
    /// that rotating the release key cannot make this file fail.
    const TEST_KEY: &str = "\
-----BEGIN PUBLIC KEY-----
MIIBojANBgkqhkiG9w0BAQEFAAOCAY8AMIIBigKCAYEAuqq1RZIA+HO50lxoFfcw
DXN/Cb3tLG1dm6pGzfMZn9fyLfx+WQbieDUDVqLYZcYH/dUajLHuvjdgyYgcIZMM
xReDVQF2xH0RcPECwJacSOcOAj3CCQNFVJEYWgCib/mzeyJsEIG2+ioLRvGDB4qC
2PH/f489MGymSkZz2IfDCfi39lNisNO33VDokbN7n6XsCZkyXj6ByCLarGtWwvXS
RVgoNKFCCfw6D/Gw9Pj8EdU/lRnXimlc+2zPZK3gnexrAroFxwHrCFrXpdrhjLXx
WHV+c0RhOjHd2HVCsluNaZ5N095rFsWbZzWduqA4gToi8AkAqso2cj4tmPP2OQBG
p/rt8Ngt5MYSnUS6vwTY+QRVC/HEbOOBfEXnDyz02hf9t6PCJFEx8rV7uvMJwN7K
5EbCqP6febALeIxXpAxa6Km4kG7+ix37LrOd1XFm0zHsOEVCdzZL9vKuonj+k0gP
ekljGXuc8zGjPVb1bGbsUZLHHWkPbOSpnjsBqzKMHwehAgMBAAE=
-----END PUBLIC KEY-----
";

    /// The same key in the other shape, to prove both are read alike.
    const TEST_KEY_PKCS1: &str = "\
-----BEGIN RSA PUBLIC KEY-----
MIIBigKCAYEAuqq1RZIA+HO50lxoFfcwDXN/Cb3tLG1dm6pGzfMZn9fyLfx+WQbi
eDUDVqLYZcYH/dUajLHuvjdgyYgcIZMMxReDVQF2xH0RcPECwJacSOcOAj3CCQNF
VJEYWgCib/mzeyJsEIG2+ioLRvGDB4qC2PH/f489MGymSkZz2IfDCfi39lNisNO3
3VDokbN7n6XsCZkyXj6ByCLarGtWwvXSRVgoNKFCCfw6D/Gw9Pj8EdU/lRnXimlc
+2zPZK3gnexrAroFxwHrCFrXpdrhjLXxWHV+c0RhOjHd2HVCsluNaZ5N095rFsWb
ZzWduqA4gToi8AkAqso2cj4tmPP2OQBGp/rt8Ngt5MYSnUS6vwTY+QRVC/HEbOOB
fEXnDyz02hf9t6PCJFEx8rV7uvMJwN7K5EbCqP6febALeIxXpAxa6Km4kG7+ix37
LrOd1XFm0zHsOEVCdzZL9vKuonj+k0gPekljGXuc8zGjPVb1bGbsUZLHHWkPbOSp
njsBqzKMHwehAgMBAAE=
-----END RSA PUBLIC KEY-----
";

    /// What was signed. Exactly these bytes, trailing newline included.
    const TEST_MESSAGE: &[u8] = b"ctxmenu signature test vector\n";

    /// The signature over [`TEST_MESSAGE`], made with the private half of
    /// [`TEST_KEY`] by `RSA.SignData(..., SHA256, Pkcs1)`.
    const TEST_SIGNATURE: &str = "\
lovxBKjYMPgUZyN4kUTZ3eWjpypF5eBu4vdhp+yqgS96UEIRJxdSO32HhHbZnCa80FmihRiqkReL
O9ToaTlVmr+uVYuHYeRNfcum0iEuR+tamr5Slji0SYOQi3IjGF++X6alNfQiHolbkCPxHFMh8tO1
C8DagMnUvfMKJGs99BYqFnOOJzHhJIBiSr49VKmfpJTW3p/EGtgckytvfjYjHQ5DzyPOK1ro1DXZ
PrEMjOODIXtk5rJlIJNlAl73jlqgHxD37AeZSprizJrI4X2h1ZwsErwCiVyMsEfOEJIoEynyikSy
CDa5H9hA6V81975HYQT5eB+R2veIf47uTYyc5zeVtGNEykJVBjrzx5KAJiBL7Q2fTs7F5jh+nybu
F9rsr8AvcVA6dwWGRN9A6REkKJUbD+5/dn/TQHoahda+4Iz1IaeoqDjq9Jzr9AypGYct2ylL4BI9
soIljPRcn4HMrz8A7CurfexB2ffxw1Qfbmn1/FZEtJ/nGUOeuiGNaB4Z";

    fn test_key() -> PublicKey {
        parse_pem(TEST_KEY).expect("the test key parses")
    }

    /// A PEM armour that says "private key", assembled rather than written.
    ///
    /// Spelled out in one piece it would carry the armour line every PKCS#8
    /// key file opens with, and both guards in front of a commit here — gitleaks and
    /// `detect-private-key` — match exactly that on sight, without caring that
    /// what follows is four characters of nothing. A repository that cries wolf
    /// over its own test data is a repository where somebody eventually adds an
    /// exception, and the next real key walks through it.
    fn armour_saying_private() -> String {
        format!(
            "-----BEGIN {kind}-----\nMIIB\n-----END {kind}-----\n",
            kind = "PRIVATE ".to_owned() + "KEY"
        )
    }

    #[test]
    fn base64_reads_the_published_vectors() {
        // RFC 4648, section 10. The one place base64 is written down.
        for (text, bytes) in [
            ("", &b""[..]),
            ("Zg==", b"f"),
            ("Zm8=", b"fo"),
            ("Zm9v", b"foo"),
            ("Zm9vYg==", b"foob"),
            ("Zm9vYmE=", b"fooba"),
            ("Zm9vYmFy", b"foobar"),
        ] {
            assert_eq!(base64_decode(text).expect(text), bytes, "{text}");
        }
    }

    #[test]
    fn base64_ignores_the_line_breaks_pem_puts_in() {
        assert_eq!(base64_decode("Zm9v\nYmFy\r\n").expect("wrapped"), b"foobar");
        assert_eq!(base64_decode("  Zm9v YmFy  ").expect("spaced"), b"foobar");
    }

    #[test]
    fn base64_refuses_what_it_cannot_read_exactly() {
        // Each of these has exactly one reading in a lenient decoder and none
        // in a strict one. A signature check is the wrong place to guess.
        for bad in [
            "Zm9vYg=x", // a character behind the padding
            "Zm9v!mFy", // outside the alphabet
            "Zm9vY",    // one digit too many
            "Zm9vYmF-", // the URL-safe alphabet, which this is not
            "Zh==",     // leftover bits that are not zero
        ] {
            assert!(base64_decode(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn both_pem_shapes_yield_the_same_key() {
        let spki = parse_pem(TEST_KEY).expect("SubjectPublicKeyInfo");
        let pkcs1 = parse_pem(TEST_KEY_PKCS1).expect("PKCS#1");
        assert_eq!(spki, pkcs1, "same key, two wrappers");
        assert_eq!(spki.bits(), 3072);
        assert_eq!(spki.exponent, vec![0x01, 0x00, 0x01], "65537");
    }

    #[test]
    fn a_private_key_is_refused_by_name() {
        // Not because it would otherwise work, but so that whoever points at
        // the wrong file reads a sentence instead of a tag number.
        let message = parse_pem(&armour_saying_private())
            .expect_err("a private key is not a public key")
            .to_string();
        assert!(message.contains("private key"), "got {message}");
    }

    #[test]
    fn nonsense_in_place_of_a_key_is_an_error_and_not_a_panic() {
        for bad in [
            "",
            "hello",
            "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n",
            "-----BEGIN PUBLIC KEY-----\nZm9vYmFy\n-----END PUBLIC KEY-----\n",
            // A well-formed SEQUENCE whose length runs past the end.
            "-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----\n",
            "-----END PUBLIC KEY-----\n-----BEGIN PUBLIC KEY-----\n",
        ] {
            assert!(parse_pem(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_key_shorter_than_the_floor_is_refused() {
        // A 512-bit key, generated once and pasted here: syntactically a key,
        // and not one anything should be trusted to.
        const SHORT: &str = "\
-----BEGIN PUBLIC KEY-----
MFwwDQYJKoZIhvcNAQEBBQADSwAwSAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf
9Cnzj4p4WGeKLs1Pt8QuKUpRKfFLfRYC9AIKjbJTWit+CqvjWYzvQwECAwEAAQ==
-----END PUBLIC KEY-----
";
        let message = parse_pem(SHORT).expect_err("512 bit").to_string();
        assert!(message.contains("too short"), "got {message}");
    }

    #[test]
    fn the_signature_of_the_test_vector_verifies() {
        // The whole chain in one assertion: PEM, base64, DER, the CNG blob and
        // BCryptVerifySignature. If any one of them is wrong, this fails.
        verify(TEST_MESSAGE, TEST_SIGNATURE, &test_key()).expect("the vector verifies");
    }

    #[test]
    fn a_changed_message_no_longer_verifies() {
        // One byte, at the end, where a truncated file would differ.
        let mut tampered = TEST_MESSAGE.to_vec();
        tampered.pop();
        assert!(verify(&tampered, TEST_SIGNATURE, &test_key()).is_err());

        // And one in the middle, which is what an edited checksums.txt is.
        let mut edited = TEST_MESSAGE.to_vec();
        edited[3] = b'M';
        assert!(verify(&edited, TEST_SIGNATURE, &test_key()).is_err());
    }

    #[test]
    fn a_changed_signature_no_longer_verifies() {
        let mut flipped: Vec<char> = TEST_SIGNATURE.chars().collect();
        flipped[0] = if flipped[0] == 'a' { 'b' } else { 'a' };
        let flipped: String = flipped.into_iter().collect();
        assert!(verify(TEST_MESSAGE, &flipped, &test_key()).is_err());

        // An empty signature is the shape a missing file arrives in.
        assert!(verify(TEST_MESSAGE, "", &test_key()).is_err());
        // And one of the right alphabet but the wrong length.
        assert!(verify(TEST_MESSAGE, "Zm9vYmFy", &test_key()).is_err());
    }

    #[test]
    fn a_signature_from_another_key_does_not_pass() {
        // The release key is a different key, and the test vector was not
        // signed with it. This is the case the whole file exists for: a
        // release signed by somebody else.
        let other = release_key().expect("the shipped key parses");
        assert!(verify(TEST_MESSAGE, TEST_SIGNATURE, &other).is_err());
    }

    #[test]
    fn the_shipped_release_key_is_a_usable_key() {
        // Not a signature check -- rotating the key must not break this file.
        // Only: what release.yml signs with has a public half in this
        // repository, it parses, and it is long enough to be worth having.
        let key = release_key().expect("release-signing.pub.pem parses");
        assert!(
            key.bits() >= 4096,
            "the released key should be 4096 bit, is {}",
            key.bits()
        );
        assert_eq!(key.exponent, vec![0x01, 0x00, 0x01], "65537");
        // Importable by CNG, which is the only thing a verification needs of
        // it. A key that parses but that CNG refuses would fail on the user's
        // machine and pass here.
        ImportedKey::import(&key).expect("CNG takes the shipped key");
    }
}
