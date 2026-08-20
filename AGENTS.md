# Notes for coding agents

Orientation for an agent working in this repository. The rules for everyone
who contributes are in [CONTRIBUTING.md](CONTRIBUTING.md) and are not repeated
here. Read that file first. Its one rule, "Measured, not assumed", covers agent
work too.

## What this program is

ctxmenu manages the classic Windows context menu. It reads the registry, shows
which entry belongs to which program, and hides, sorts, deletes or creates
entries. It builds new ones as well: submenus, favourites, and services read
from an OpenAPI description.

Target platform: Windows 10 1809 or newer, 64-bit, plus the classic menu on
Windows 11. Rust throughout, `egui` and `eframe` for the interface,
`windows-rs` for the registry. The result is a single `.exe`, with no installer
and no runtime to put beside it.

## Code map

| Path | What lives there |
|---|---|
| `ctxmenu/src/app.rs` | the interface, including `run_native` |
| `ctxmenu/src/cli.rs` | the subcommands. With no argument the window opens, not the help |
| `ctxmenu/src/registry/` | `scan`, `backup`, `write`, `plan`, `create`, `filetypes`, `clsid`, `mui`, `paths`, `win11` |
| `ctxmenu/src/program/`, `ctxmenu/src/icons/` | grouping entries by program, icon extraction |
| `ctxmenu/src/webtool/`, `ctxmenu/src/service/` | favourites with web tools, services from an OpenAPI description |
| `ctxmenu/src/update/` | self-update: ask GitHub, verify the signature, replace the running `.exe` |
| `ctxmenu/src/i18n.rs` | both languages, one `Strings` value each |
| `ctxmenu/src/bilingual.rs` | the markers a bilingual message is split at |
| `ctxmenu/tests/` | four integration targets: `registry_roundtrip`, `plan_transaction`, `release_signature`, `snapotter_abnahme` |

## Do not touch

- `target/`. Build artefacts.
- `%LOCALAPPDATA%\ctxmenu\`. The user's settings, favourites and backups. Real
  backups of real registry keys live there.
- `ctxmenu/release-signing.pub.pem`. `include_str!` compiles it into every
  `.exe`, so changing it takes the self-update away from every shipped version.
- The private half of the release signing key. It lives outside the repository
  and never enters it. Without it nobody can build a release that a shipped
  version would install.

## Read before you change

Read a file before you edit it. Search for every caller before you change a
function.

## Where a new finding goes

Walk this ladder from the top. Stop at the first step that fits.

1. A regression test can catch it. Write the test.
2. A lint or CI rule can catch it. Add the rule, and word the error message so
   that it names the way out.
3. Only prose explains it. Put a comment at the code it applies to.

## Commands

[CONTRIBUTING.md](CONTRIBUTING.md) covers the setup, the git hooks and what
each check is for. This table is the short reference.

| Purpose | Command |
|---|---|
| Format | `cargo fmt --all` |
| Lint | `cargo clippy --all-targets -- -D warnings` |
| Test | `cargo test` |
| Release build | `cargo build --release` |

Run `cargo test` without `--test-threads=1`. CI calls it that way, and only
then do the faults between concurrent tests show up.

End every finished item with `cargo build --release`. A debug build that passed
the tests is not a shipped one, because `lto` and `codegen-units = 1` apply to
the release profile alone.

Before a release, `git log --oneline v<version>..HEAD` must be empty. A tag
that stays put while the work goes on ships the old state.
