# Third-Party Components

*[Deutsche Fassung](THIRD-PARTY-NOTICES_DE.md)*

`ctxmenu` is licensed under the MIT license (see [`LICENSE`](../LICENSE)). The finished
`ctxmenu.exe` additionally contains code and data from third parties, which
are subject to their own terms. This file lists them in full.

None of them require derivative works to be disclosed: they are all
permissive licenses throughout. All of them do require that the copyright
notice and license text be included, and that is exactly what this file is
for.

## Embedded in the shipped file

| Component | License | Purpose |
|---|---|---|
| [Feather Icons](https://feathericons.com/) | MIT (© Cole Bemis) | The interface icons. Embedded as a font file via the `iconflow` crate, about 58 KB of the `.exe`. |
| [`iconflow`](https://crates.io/crates/iconflow) | MIT | Packages Feather as a font and resolves names to glyphs. |
| [`egui` / `eframe` / `egui_extras`](https://github.com/emilk/egui) | MIT or Apache-2.0 | The interface and its window. |
| [`windows` / `windows-registry`](https://github.com/microsoft/windows-rs) | MIT or Apache-2.0 | The Windows interfaces: registry, GDI, WinHTTP, shell. |
| [`serde` / `serde_json`](https://serde.rs/) | MIT or Apache-2.0 | Reading and writing the JSON files. |
| [`anyhow`](https://github.com/dtolnay/anyhow) · [`thiserror`](https://github.com/dtolnay/thiserror) | MIT or Apache-2.0 | Error handling. |
| [`chrono`](https://github.com/chronotope/chrono) | MIT or Apache-2.0 | Timestamps for the backups. |
| [`dirs`](https://github.com/dirs-dev/dirs-rs) | MIT or Apache-2.0 | Finds `%LOCALAPPDATA%`. |
| [`rustc-hash`](https://github.com/rust-lang/rustc-hash) | Apache-2.0 or MIT | The fast sets and maps in the image path. |
| [`raw-window-handle`](https://github.com/rust-windowing/raw-window-handle) | MIT, Apache-2.0, or Zlib | Window handle for the dark title bar. |
| [`read-fonts`](https://github.com/googlefonts/fontations) | MIT or Apache-2.0 | Checks in tests whether a glyph is actually present in a font. |
| [`winresource`](https://github.com/BenjaminRi/winresource) | MIT | Writes version and icon into the file resource. |

The full license texts are included in the source packages of the respective
crates; `cargo vendor` or a look into `~/.cargo/registry` will bring them up.

## Used but not shipped

- **Segoe UI** and **Segoe UI Symbol** belong to Windows and are loaded from
  there. They are not part of this application.
- **`reg.exe`** is Windows's own backup tool and is invoked, not embedded.

## The logo

The logo in the top-right corner of the window comes from the author's own
project ([corgan2222/Dashboard](https://github.com/corgan2222/Dashboard)) and
belongs to him; it is not covered by this program's MIT license. Anyone who
forks `ctxmenu` and redistributes it replaces it with one of their own: the
file is located at `ctxmenu/assets/logo.rgba`.
