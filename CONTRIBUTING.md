# Contributing

Thanks for stopping by. This is a small project with strong opinions; the
rules below are what keep it small.

## The one rule

**Measured, not assumed.**

Windows behaves differently from its own documentation in more than one
place. This program therefore claims nothing that hasn't been verified on a
real system, and where a number is missing, no claim is made either. A
change based on "should work in theory" is not one.

Everything else follows from that.

## Setup

Rust 1.95 or newer, `x86_64-pc-windows-msvc`, plus the Visual Studio Build
Tools with the C++ toolchain. Then:

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

All four must be green. `-D warnings` is not negotiable.

The same checks run as git hooks, so that a red build is caught before it
travels rather than after. Once per clone:

```powershell
pip install pre-commit
pre-commit install --install-hooks
pre-commit install --hook-type pre-push
```

Committing then runs gitleaks, the YAML and TOML checks and `cargo fmt`;
pushing runs `cargo clippy` and `cargo test`. Building the gitleaks hook needs
Go on the PATH. The hooks live on your machine and `--no-verify` skips them —
CI runs the same checks again and is the one that decides.

The result ends up at `target\x86_64-pc-windows-msvc\release\ctxmenu.exe`,
not `target\release\`: `.cargo\config.toml` names the target explicitly, so
the statically linked C runtime applies to the application and not also to
the compiler's macro libraries.

## Branches and main

`main` is protected and takes no direct push — not from a contributor, not
from the maintainer. Every change arrives as a pull request, and every change
gets its own branch:

```powershell
git switch -c feature/short-name    # or bugfix/, docs/, chore/
git push -u origin HEAD
gh pr create
```

Those four prefixes are not decoration. `.github/release-drafter.yml` reads
them and sorts the pull request into the right heading of the next release's
notes, so a branch named after what it does labels itself. A branch named
something else still merges; it just arrives in the notes unsorted.

To merge, `check` and `secrets` must be green and the branch must be up to
date with `main`. No approving review is required — this is a one-person
project and nobody can approve their own work.

## What belongs in a pull request

- **One test per new pure function.** Test names are complete sentences that
  state what holds: `fn a_range_whose_ends_are_the_wrong_way_round_is_no_range_at_all`.
  Anyone who can't spell out the name in words hasn't understood the rule
  yet.
- **Comments that explain the *why*.** What the code does is in the code.
  What's valuable is what explains it: which alternative was rejected, which
  measurement is behind it, which Windows quirk forces it.
- **English in the code**, including comments and identifiers. Interface
  text runs through `ctxmenu/src/i18n.rs` and exists twice, in German and
  English.
- **Commit messages in complete sentences** that say what the change does
  and why. No `feat:` prefix.

## About changes to the registry

The most delicate part, so the strictest rules apply:

- **Never delete without a backup.** This isn't a request, it's checked by
  the type system: `write::delete_tree` requires a `BackupToken`, and the
  only way to get one is as the return value of a successful
  `backup::export`.
- **A target is a `RegTarget`**, not a string. Whatever can't be expressed
  as a single entry beneath a Classes root shouldn't be constructible at
  all.
- **Write attempts against `HKLM` belong in a throwaway VM**, not on the
  development machine. A script under `tools\` sets one up.

## What tends to get rejected

- **New dependencies.** Every one has to justify itself; the list is short
  and should stay that way.
- **Rewrites without a bug behind them.** Refactoring that fixes nothing and
  enables nothing costs review time and adds risk.
- **Features Windows doesn't offer.** Free reordering of menu entries, for
  example: measured, and the system only knows `Position=Top` and
  `Position=Bottom`. What fails because of that is listed in the
  [README](README.md) under "What It Deliberately Cannot Do."
- **Machine-generated translations.** Both languages are written by hand and
  are meant to read equally well.

## Reporting bugs

What makes a report quick to act on:

- The version from the About window and the Windows build.
- The shortest path to trigger it.
- The affected registry path, if it's about an entry.
- The excerpt from `%LOCALAPPDATA%\ctxmenu\ctxmenu.log`: **review it first**,
  it names paths and file names from your own machine.

A security vulnerability does **not** belong in an issue: see
[`SECURITY.md`](SECURITY.md).
