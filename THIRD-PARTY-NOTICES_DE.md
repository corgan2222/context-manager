# Fremde Bestandteile

*[English version](THIRD-PARTY-NOTICES.md)*

`ctxmenu` steht unter der MIT-Lizenz (siehe `LICENSE`). Die fertige
`ctxmenu.exe` enthält darüber hinaus Code und Daten Dritter, die unter eigenen
Bedingungen stehen. Diese Datei nennt sie vollständig.

Nichts davon verlangt, dass abgeleitete Arbeiten offengelegt werden — es sind
durchweg permissive Lizenzen. Alle verlangen aber, dass Urhebervermerk und
Lizenztext mitgeliefert werden, und genau dafür ist diese Datei da.

## Eingebettet in die ausgelieferte Datei

| Bestandteil | Lizenz | Wofür |
|---|---|---|
| [Feather Icons](https://feathericons.com/) | MIT (© Cole Bemis) | Die Symbole der Oberfläche. Als Schriftdatei über die Crate `iconflow` eingebettet — rund 58 KB der `.exe`. |
| [`iconflow`](https://crates.io/crates/iconflow) | MIT | Verpackt Feather als Schrift und löst Namen zu Zeichen auf. |
| [`egui` / `eframe` / `egui_extras`](https://github.com/emilk/egui) | MIT oder Apache-2.0 | Die Oberfläche und ihr Fenster. |
| [`windows` / `windows-registry`](https://github.com/microsoft/windows-rs) | MIT oder Apache-2.0 | Die Windows-Schnittstellen: Registry, GDI, WinHTTP, Shell. |
| [`serde` / `serde_json`](https://serde.rs/) | MIT oder Apache-2.0 | Lesen und Schreiben der JSON-Dateien. |
| [`anyhow`](https://github.com/dtolnay/anyhow) · [`thiserror`](https://github.com/dtolnay/thiserror) | MIT oder Apache-2.0 | Fehlerbehandlung. |
| [`chrono`](https://github.com/chronotope/chrono) | MIT oder Apache-2.0 | Zeitstempel der Sicherungen. |
| [`dirs`](https://github.com/dirs-dev/dirs-rs) | MIT oder Apache-2.0 | Findet `%LOCALAPPDATA%`. |
| [`rustc-hash`](https://github.com/rust-lang/rustc-hash) | Apache-2.0 oder MIT | Die schnellen Mengen und Tabellen im Bildpfad. |
| [`raw-window-handle`](https://github.com/rust-windowing/raw-window-handle) | MIT, Apache-2.0 oder Zlib | Fenstergriff für die dunkle Titelleiste. |
| [`read-fonts`](https://github.com/googlefonts/fontations) | MIT oder Apache-2.0 | Prüft im Test, ob ein Zeichen in einer Schrift wirklich vorhanden ist. |
| [`winresource`](https://github.com/BenjaminRi/winresource) | MIT | Schreibt Version und Symbol in die Dateiressource. |

Die vollständigen Lizenztexte liegen in den Quellpaketen der jeweiligen Crates;
`cargo vendor` oder ein Blick in `~/.cargo/registry` fördert sie zutage.

## Benutzt, aber nicht mitgeliefert

- **Segoe UI** und **Segoe UI Symbol** gehören zu Windows und werden von dort
  geladen. Sie sind nicht Teil dieser Anwendung.
- **`reg.exe`** ist das Sicherungswerkzeug von Windows und wird aufgerufen, nicht
  eingebettet.

## Das Logo

Die Bildmarke oben rechts im Fenster stammt aus dem Projekt des Autors
([corgan2222/Dashboard](https://github.com/corgan2222/Dashboard)) und gehört
ihm; sie steht nicht unter der MIT-Lizenz dieses Programms. Wer `ctxmenu` forkt
und weitergibt, ersetzt sie durch eine eigene — die Datei liegt unter
`ctxmenu/assets/logo.rgba`.
