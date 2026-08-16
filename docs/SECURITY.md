# Security Policy

*[Deutsche Fassung](SECURITY_DE.md)*

`ctxmenu` is a desktop program. It runs as the logged-in user on a Windows PC,
reads and writes the context menu's registry keys, and can send files to web
services the user has entered themselves. It is not a service, has no
accounts, and does not listen on any port.

Three things make it security-relevant anyway, which is why the scope is
spelled out here rather than left to guessing:

- It **writes to the registry**, some of it under `HKLM`, i.e. for all
  accounts.
- It **requests elevated privileges** for that and restarts itself.
- It **sends files** to addresses and stores the keys for that.

## Supported Versions

Only the latest release.

| Version | Fixes |
|---|---|
| Latest release | Yes |
| Anything older | No, update first |

The project has one maintainer. There is no branch on which an older version
continues to be maintained, and a table that promised otherwise would be a
promise no one could keep.

## Reporting a Vulnerability

**Please not as a public issue.** Issues are public the moment they are
filed, and every reader is someone who can act on it before a fix exists.

Two private channels, both are fine:

- **Private report via GitHub**: in this repository's *Security* tab,
  *Report a vulnerability*. Preferred: report, discussion, and fix stay in
  one place, and you see the patch before it goes public.
- **Email to `stefan@knaak.org`** with `ctxmenu security` in the subject.
  Nothing is encrypted on the receiving end; if a detail is too sensitive
  for plaintext mail, please ask briefly for a different channel.

What makes a report quick to act on: the version from the About window, the
Windows build, the steps to trigger it, and, if available, the excerpt from
`%LOCALAPPDATA%\ctxmenu\ctxmenu.log`. **Review the log beforehand:** it names
registry paths and file names from your machine.

## What Counts as a Vulnerability

Anything one of these sentences describes:

- A way for the program to **write something other than what the user
  confirmed**, in particular outside the registry areas it manages, or
  without the backup it promises.
- A way for the **elevated privileges** to be used for something other than
  the one confirmed step; any way to influence the elevated operation from
  the outside.
- A way for **a file to be sent** without the user having consented to that
  for this tool, or to an address other than the one entered.
- A way for **a stored key** to reach someone who should not otherwise be
  able to read it.
- A **malicious OpenAPI document** or a malicious response from a service
  that makes the program write outside the target folder, execute something
  it should not, or crash.
- **A backup that does not restore** what it claims to contain.

## What Is Explicitly Not a Vulnerability

- **The keys are stored in plaintext** in `%LOCALAPPDATA%\ctxmenu\`. That is
  intentional and documented: they are protected by the permissions on the
  user profile, the same as in an `.npmrc` or `.gitconfig`. Anyone who does
  not want that uses a key with restricted permissions. An attacker who can
  read your profile has already won regardless.
- **The program can break the context menu.** That is its purpose. It backs
  up beforehand and states beforehand what it will do.
- **An entry can execute any command** that the user writes into it. That is
  the function of the context menu, not a flaw in it.
- **Unencrypted `http://`**, when explicitly allowed for a favorite. The
  program refuses it until someone sets the checkbox.
- **SmartScreen warns about the `.exe`.** It is not signed; a certificate
  Windows trusts by default costs several hundred euros a year. The
  checksum for each release is published with the release.
- Reports from a vulnerability scanner **without a path showing how this
  applies here**. A dependency with a CVE in a code path this program does
  not use is not a vulnerability of this program.

## What You Can Expect

- **Acknowledgment of receipt within three days.** If nothing arrives after
  a week, the mail got lost, please follow up via the other channel.
- **An assessment within two weeks**: confirmed, not a bug, or a follow-up
  question.
- **Credit, if you want it**, in the release that fixes it.
- No money. This is a spare-time project with no revenue.
