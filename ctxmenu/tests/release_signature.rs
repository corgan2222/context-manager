//! The seam between the release workflow and the updater.
//!
//! `release.yml` signs `checksums.txt` with .NET's `RSA.SignData`, driven from
//! PowerShell; `ctxmenu::update::signature` verifies it with Windows CNG. Two
//! different libraries, two different languages, one file format between them —
//! and nothing else in the test suite would notice if they drifted apart. What
//! *would* notice is a user, at the point where their update stops installing.
//!
//! So this test does what the workflow does, on this machine, and then reads the
//! result the way the program does: generate a pair, write a checksums.txt,
//! sign it, and verify the signature against the public half.
//!
//! It needs PowerShell 7 — `RSA.ImportFromPem` and `ExportSubjectPublicKeyInfoPem`
//! arrived in .NET 7, and Windows PowerShell 5.1 runs on the Framework, which
//! has neither. The GitHub runner has `pwsh`; a machine without it is told so
//! and the test passes, because a missing tool is not a failing program.

use std::path::Path;
use std::process::Command;

use ctxmenu::update::signature;

/// Exactly what `release.yml` does, in one script, into `directory`.
const SIGN: &str = r#"
param([string] $Directory)
$ErrorActionPreference = 'Stop'

# A checksums.txt of the shape the workflow writes: upper case digest, two
# spaces, the name, and the CRLF line endings Set-Content puts there.
$lines = @(
  'E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855  ctxmenu.exe'
  '0000111122223333444455556666777788889999AAAABBBBCCCCDDDDEEEEFFFF  ctxmenu_9.9.9_windows_amd64.zip'
)
$checksums = Join-Path $Directory 'checksums.txt'
$lines | Set-Content -Encoding ascii $checksums

$sha = [System.Security.Cryptography.HashAlgorithmName]::SHA256
$pkcs1 = [System.Security.Cryptography.RSASignaturePadding]::Pkcs1
$rsa = [System.Security.Cryptography.RSA]::Create(3072)
try {
  $bytes = [System.IO.File]::ReadAllBytes($checksums)
  $signature = $rsa.SignData($bytes, $sha, $pkcs1)
  $armoured = [Convert]::ToBase64String($signature, 'InsertLineBreaks')
  [System.IO.File]::WriteAllText(
    (Join-Path $Directory 'checksums.txt.sig'),
    $armoured + "`n",
    [System.Text.UTF8Encoding]::new($false))
  [System.IO.File]::WriteAllText(
    (Join-Path $Directory 'public.pem'),
    $rsa.ExportSubjectPublicKeyInfoPem(),
    [System.Text.UTF8Encoding]::new($false))
} finally {
  $rsa.Dispose()
}
"#;

#[test]
fn what_the_release_workflow_signs_is_what_this_program_verifies() {
    let directory = std::env::temp_dir().join(format!("ctxmenu-signature-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a directory in TEMP");

    let script = directory.join("sign.ps1");
    std::fs::write(&script, SIGN).expect("writing the script");

    let run = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(&script)
        .arg("-Directory")
        .arg(&directory)
        .output();

    let outcome = match run {
        Ok(outcome) => outcome,
        // Not a failure of this program. Said out loud rather than swallowed,
        // so that a green run on a machine without pwsh is not mistaken for a
        // green run of this test.
        Err(error) => {
            eprintln!(
                "pwsh is not on this machine ({error}) -- the signature seam was not checked"
            );
            clean_up(&directory);
            return;
        }
    };
    assert!(
        outcome.status.success(),
        "the signing script failed:\n{}",
        String::from_utf8_lossy(&outcome.stderr)
    );

    let checksums = std::fs::read(directory.join("checksums.txt")).expect("checksums.txt");
    let armoured =
        std::fs::read_to_string(directory.join("checksums.txt.sig")).expect("checksums.txt.sig");
    let pem = std::fs::read_to_string(directory.join("public.pem")).expect("public.pem");
    clean_up(&directory);

    let key = signature::parse_pem(&pem).expect("the exported public half parses");
    assert_eq!(key.bits(), 3072);

    signature::verify(&checksums, &armoured, &key).expect(
        "what PowerShell signed, CNG must verify -- if this fails, updates stop installing",
    );

    // And the other direction, because a verifier that says yes to everything
    // would pass the assertion above.
    let mut tampered = checksums.clone();
    tampered[0] = b'F';
    assert!(
        signature::verify(&tampered, &armoured, &key).is_err(),
        "an edited checksums.txt must not verify"
    );
}

fn clean_up(directory: &Path) {
    let _ = std::fs::remove_dir_all(directory);
}
