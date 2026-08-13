# Handover: Windows Context Menu Manager (Rust + egui)

**Entwicklungsplattform:** Windows 10 (1809+). Alles muss dort laufen und getestet werden.
**Zielplattform v1:** Windows 10 und das klassische Kontextmenü unter Windows 11 (Shift+F10 / "Weitere Optionen anzeigen").
**Ergebnis:** eine einzelne `.exe`, kein Installer, keine Runtime-Abhängigkeit.

Das neue Windows-11-Hauptmenü ist **nicht Teil von v1**. Abschnitt 14 beschreibt, was später nötig wird, damit die Architektur sich das nicht verbaut.

---

## 1. Anforderungen

**Muss:**

1. Vorhandene Kontextmenü-Einträge anzeigen, getrennt nach Kategorie: Dateien allgemein, Ordner, Ordner-Hintergrund, Desktop-Hintergrund, Laufwerke.
2. **Dateityp-Ansicht:** Einträge für die wichtigsten Dateitypen einzeln anzeigen (PDF, JPG, PNG, MP4, ZIP, …), inklusive aller Vererbungsebenen. Siehe Abschnitt 10.
3. **Programm-Ansicht:** Einträge nach dem aufgerufenen Programm gruppieren und alle Vorkommen gemeinsam löschen oder deaktivieren, auch wenn sie über 15 Dateitypen verteilt sind. Siehe Abschnitt 11.
4. Einträge löschen, immer mit vorherigem Backup.
5. Reihenfolge beeinflussen, soweit Windows das zulässt. Siehe Abschnitt 6.
6. Eigene Einträge anlegen: Anzeigename, Kommandozeile, Icon, optional Untermenü.
7. Icons aus `.ico`, `.exe`, `.dll` (mit Index) laden und in der GUI vorschauen.
8. GUI-Sprache Deutsch und Englisch, zur Laufzeit umschaltbar.
9. Dark Mode, Light Mode und "System folgen", inklusive Titelleiste.

**Nicht im Scope für v1:**

- Bearbeiten von Einträgen fremder COM-Handler. Deren Text wird zur Laufzeit generiert, in der Registry steht er nicht.
- Das neue Win11-Hauptmenü.

---

## 2. Entwicklungsumgebung einrichten

Reihenfolge einhalten. Punkt 2 vor Punkt 1 zu installieren spart einen Neustart des Terminals.

### 2.1 MSVC Build Tools

Rust braucht unter Windows den Microsoft-Linker. Ohne das schlägt der erste Build mit `error: linker 'link.exe' not found` fehl.

Installiere **Visual Studio Build Tools 2022** (der Standalone-Installer, kein volles Visual Studio nötig). Im Installer:

- Workload: **Desktopentwicklung mit C++**
- In der Einzelkomponenten-Liste sicherstellen, dass angehakt ist:
  - MSVC v143 Buildtools für x64/x86
  - **Windows 11 SDK** (Version 10.0.22621 oder neuer). Läuft problemlos auf Windows 10 und bringt die aktuelleren Header. Das Windows 10 SDK ab 10.0.19041 geht auch.
  - C++-ATL ist **nicht** nötig für v1, wird aber für den späteren COM-Handler aus Abschnitt 14 gebraucht. Wenn du sowieso installierst, nimm es gleich mit.

Platzbedarf grob 5 bis 7 GB.

**Warum das SDK zwingend ist:** `winresource` ruft `rc.exe` auf, um Icon und Manifest in die `.exe` zu kompilieren. `rc.exe` kommt aus dem Windows SDK, nicht aus den Build Tools. Fehlt es, bricht der Build mit einer wenig hilfreichen Meldung ab.

### 2.2 Rust-Toolchain

Über `rustup` von `rustup.rs`. Der Installer erkennt die Build Tools und wählt das richtige Target.

```powershell
rustup default stable-x86_64-pc-windows-msvc
rustup component add rustfmt clippy rust-src
rustc -vV        # host muss x86_64-pc-windows-msvc zeigen
```

**Nicht** das `-gnu`-Target verwenden. `windows-rs` funktioniert damit zwar grundsätzlich, aber die COM- und Ressourcen-Anbindung ist auf MSVC ausgelegt, und du bekommst eine MinGW-Runtime-Abhängigkeit ins Binary.

`rust-src` braucht rust-analyzer, um in die Standardbibliothek springen zu können.

### 2.3 Runtime-Abhängigkeiten prüfen

Rust linkt die MSVC-Runtime auf `windows-msvc` standardmäßig statisch. Die fertige `.exe` sollte also kein `vcruntime140.dll` brauchen. Nach dem ersten Release-Build gegenprüfen:

```powershell
dumpbin /dependents target\release\ctxmenu.exe
```

`dumpbin` liegt in der "x64 Native Tools Command Prompt for VS 2022". In der Ausgabe dürfen nur Windows-eigene DLLs stehen (`KERNEL32.dll`, `USER32.dll`, `SHELL32.dll`, `ADVAPI32.dll`, `GDI32.dll`, `dwmapi.dll`, `ole32.dll`, `opengl32.dll`). Taucht dort `VCRUNTIME140.dll` oder `MSVCP140.dll` auf, stimmt etwas mit der CRT-Konfiguration nicht.

Das ist der Test für die Anforderung "keine Runtime-Abhängigkeit". Mach ihn früh, nicht erst kurz vor der Veröffentlichung.

### 2.4 Git

Git for Windows installieren. Für ein Rust-Repo die Zeilenenden festnageln, sonst produziert `rustfmt` bei jedem Commit Rauschen:

```powershell
git config --global core.autocrlf false
```

Dazu eine `.gitattributes` im Projektwurzelverzeichnis:

```
* text=auto eol=lf
*.ico binary
*.png binary
```

Lange Pfade freischalten, weil `target/` tief verschachtelt:

```powershell
git config --system core.longpaths true
```

Zusätzlich in der Registry oder per Gruppenrichtlinie:

```
HKLM\SYSTEM\CurrentControlSet\Control\FileSystem
    LongPathsEnabled = 1   (REG_DWORD)
```

### 2.5 Editor

**VS Code** ist der pragmatische Weg. Erweiterungen:

| Erweiterung | Zweck |
|---|---|
| `rust-lang.rust-analyzer` | Sprachserver, unverzichtbar |
| `ms-vscode.cpptools` | Debugger `cppvsdbg` für MSVC-Builds |
| `tamasfe.even-better-toml` | `Cargo.toml`-Unterstützung |
| `usernamehw.errorlens` | Fehler inline statt nur in der Problemliste |
| `fill-labs.dependi` | Versionshinweise für Crates |

Debug-Konfiguration in `.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "name": "Debug ctxmenu",
      "type": "cppvsdbg",
      "request": "launch",
      "program": "${workspaceFolder}/target/debug/ctxmenu.exe",
      "args": [],
      "cwd": "${workspaceFolder}",
      "preLaunchTask": "cargo build"
    }
  ]
}
```

`cppvsdbg` ist unter MSVC dem `CodeLLDB` vorzuziehen, weil es die PDB-Symbole nativ versteht. **RustRover** (JetBrains, für nichtkommerzielle Nutzung kostenlos) ist die Alternative, wenn dir eine vollständige IDE lieber ist.

### 2.6 Cargo-Werkzeuge

```powershell
cargo install bacon          # kontinuierliches cargo check im Hintergrund
cargo install cargo-bloat    # wer frisst das Binary auf
cargo install cargo-deny     # Lizenz- und Advisory-Prüfung
```

`cargo-bloat` ist für das Ziel "unter 15 MB" aus Abschnitt 15 die entscheidende Diagnose:

```powershell
cargo bloat --release --crates
```

`bacon` ersetzt das ständige manuelle `cargo check` und ist auf Windows deutlich flotter als `cargo-watch`.

### 2.7 Analysewerkzeuge für dieses Projekt

Das ist der Teil, der den Unterschied macht. Ohne diese Tools rätst du bei der Registry-Semantik.

**Process Monitor (Sysinternals), das wichtigste Werkzeug hier.** Filter setzen auf `Process Name is explorer.exe` und `Operation begins with Reg`, dann im Explorer einen Rechtsklick auf eine `.jpg` machen. Du siehst exakt, welche Keys Windows in welcher Reihenfolge liest. **Damit verifizierst du die Auflösungskette aus Abschnitt 10.1**, statt dich auf die Dokumentation zu verlassen.

**Process Explorer (Sysinternals).** Spalte "GDI Objects" einblenden. Das ist der Leak-Test für die Icon-Extraktion aus Abschnitt 7.3: nach fünf Vollscans darf der Wert nicht monoton steigen. Der Task-Manager kann das auch (Details-Tab, Spalte "GDI-Objekte"), Process Explorer aktualisiert feiner.

**Autoruns (Sysinternals).** Der Tab "Explorer" listet die registrierten Shell-Erweiterungen. Gute Gegenprobe für deinen `shellex`-Scanner: findet dein Tool dieselben Handler?

**ShellExView und ShellMenuView (NirSoft).** Die direkte Referenzimplementierung. Vergleiche deine Ausgabe damit, besonders bei den Ebenen 3 und 4 aus Abschnitt 10.1, die ShellMenuView teilweise anders zuordnet.

**Resource Hacker.** Zum Nachschlagen von Icon-Ressourcen-IDs in `shell32.dll` und `imageres.dll`. Damit prüfst du, ob dein Parser aus Abschnitt 7.1 negative Indizes richtig behandelt.

**OleViewDotNet** (optional). Für die CLSID-Auflösung, wenn ein Handler sich nicht erklären lässt.

Alle Sysinternals-Tools laufen ohne Installation. Lade die Suite als ZIP und leg sie irgendwo hin.

### 2.8 Test-VM

**Nicht auf der Arbeitsmaschine schreiben.** Das Tool ändert `HKLM\SOFTWARE\Classes`, und ein Fehler dort macht den Explorer unbenutzbar.

- Hyper-V (in Windows 10 Pro enthalten) oder VirtualBox
- Windows 10 22H2 als Evaluierungs-ISO vom Microsoft Evaluation Center, 90 Tage gültig und für diesen Zweck ausreichend
- **Checkpoint vor der ersten Schreiboperation**, danach vor jeder neuen Testrunde zurücksetzen
- In der VM ein paar Programme installieren, die viele Kontextmenü-Einträge anlegen: 7-Zip, VLC, IrfanView, Notepad++, Git for Windows. Das gibt dir echtes Testmaterial für die Programm-Gruppierung aus Abschnitt 11

Als leichtgewichtige Ergänzung für reine HKCU-Tests taugt ein zweites lokales Benutzerkonto auf der Entwicklungsmaschine. Für HKLM-Tests reicht das nicht.

### 2.9 Defender-Ausnahmen

Windows Defender scannt jede Datei in `target/`, und Rust erzeugt davon tausende. Das kostet bei einem Vollbuild leicht die Hälfte der Zeit.

```powershell
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.cargo"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.rustup"
Add-MpPreference -ExclusionPath "D:\Coding\ctxmenu"    # Projektpfad anpassen
```

Braucht eine Administrator-PowerShell.

Rechne außerdem damit, dass Defender die frisch gebaute, unsignierte `.exe` beim ersten Start anmeckert. Das ist normal bei einem Tool, das in die Registry schreibt, und verschwindet erst mit einem Code-Signing-Zertifikat.

### 2.10 Projekt anlegen

```powershell
cargo new ctxmenu
cd ctxmenu
mkdir assets, src\ui, src\registry, src\program, src\icons
```

`rust-toolchain.toml` im Wurzelverzeichnis, damit die Version reproduzierbar ist:

```toml
[toolchain]
channel = "1.85"
components = ["rustfmt", "clippy", "rust-src"]
targets = ["x86_64-pc-windows-msvc"]
```

Versionsnummer an die tatsächlich installierte anpassen.

`.gitignore`:

```
/target
/backups
*.pdb
```

### 2.11 Smoke-Test vor dem eigentlichen Start

Bevor du Registry-Code schreibst, bau ein minimales eframe-Fenster und prüfe fünf Dinge. Jedes davon deckt ein Setup-Problem auf, das später schwer zuzuordnen wäre.

1. `cargo run --release` öffnet ein Fenster. Scheitert es am Linker, fehlen die Build Tools.
2. Kein Konsolenfenster im Release-Build. Sonst fehlt `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
3. Die `.exe` zeigt im Explorer dein Icon. Sonst hat `winresource` `rc.exe` nicht gefunden.
4. Systemtheme auf Dunkel stellen, App neu starten: die Oberfläche ist dunkel. Prüft `ThemePreference::System` aus Abschnitt 9.1 auf deinem konkreten Windows-Build.
5. `dumpbin /dependents` zeigt kein `VCRUNTIME140.dll`.

Erst danach mit Meilenstein 1 anfangen.

### 2.12 Häufige Setup-Fehler

| Symptom | Ursache |
|---|---|
| `error: linker 'link.exe' not found` | Build Tools fehlen oder falsches Target (`-gnu` statt `-msvc`) |
| `rc.exe not found` beim Build | Windows-SDK-Komponente nicht mitinstalliert |
| Build bricht mit Pfadlängenfehler ab | `LongPathsEnabled` nicht gesetzt, oder Projekt liegt zu tief |
| Fenster bleibt schwarz, kein Rendering | glow findet keinen OpenGL-Kontext. Tritt über RDP und in manchen VMs auf. Lokal testen, oder auf `wgpu` ausweichen |
| Vollbuild dauert mehrere Minuten | Defender-Ausnahmen fehlen |
| `cargo` kennt nach der rustup-Installation kein `cargo` | Terminal neu öffnen, PATH wurde erst danach gesetzt |

---

## 3. Tech-Stack

```toml
[package]
name = "ctxmenu"
version = "0.1.0"
edition = "2021"

[dependencies]
eframe = { version = "0.31", default-features = false, features = [
    "default_fonts",
    "glow",
    "wayland",          # schadet nicht, wird unter Windows ignoriert
] }
egui = "0.31"
egui_extras = { version = "0.31", features = ["image"] }
windows-registry = "0.5"
windows-result = "0.3"
raw-window-handle = "0.6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
chrono = "0.4"
dirs = "5"
rustc-hash = "2"

[dependencies.windows]
version = "0.59"
features = [
    "Win32_Foundation",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Graphics_Gdi",
    "Win32_Graphics_Dwm",
    "Win32_System_Com",
    "Win32_System_Registry",
    "Win32_System_Threading",
    "Win32_System_Environment",
    "Win32_Storage_FileSystem",
    "Win32_Security",
]

[build-dependencies]
winresource = "0.1"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

Versionsnummern vor dem ersten Build gegen crates.io prüfen. `windows` und `egui` ziehen schnell weiter, und `egui` bricht zwischen Minor-Versionen regelmäßig die API. Pinne die Version im Lockfile und aktualisiere bewusst, nicht nebenbei.

**Renderer:** `glow` (OpenGL) ist der Standard und liefert das kleinste Binary. `wgpu` ist die Alternative, wenn glow auf einer Zielmaschine zickt, kostet aber mehrere MB und deutlich längere Buildzeit. Fang mit glow an.

**Lizenz:** egui ist MIT oder Apache-2.0. Keine Auflagen für die Veröffentlichung.

### build.rs

```rust
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set_manifest_file("assets/app.manifest");
        res.compile().expect("Ressourcen-Kompilierung fehlgeschlagen");
    }
}
```

---

## 4. egui: Aufbau und Architektur

### 4.1 Grundgerüst

```rust
// src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    // Elevated-Job-Modus vor dem GUI-Start abfangen (siehe 13.2)
    if let Some(job) = elevation::parse_job_arg() {
        return elevation::run_job(job);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(load_app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Context Menu Manager",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
```

### 4.2 Modulschnitt

```
src/
  main.rs              // eframe-Start, Elevated-Job-Modus
  app.rs               // App-Struct, update(), Tab-Routing
  i18n.rs              // DE/EN Strings
  theme.rs             // ThemePreference, DWM-Titelleiste
  settings.rs          // Sprache, Theme, eigene Dateitypen persistieren
  model.rs
  ui/
    mod.rs
    style.rs           // Badge-Palette, Spaltenbreiten, Icon-Größen
    tab_categories.rs  // Kategoriebaum + Eintragstabelle
    tab_filetypes.rs   // Dateityp-Baum + Eintragstabelle
    tab_programs.rs    // Programm-Gruppen + Detailliste
    tab_backups.rs     // Verlauf und Restore
    entry_table.rs     // gemeinsame Tabelle, von drei Tabs genutzt
    editor.rs          // Formular neu/bearbeiten
    dialogs.rs         // Bestätigung, Ergebnis, Fehler
  registry/
    mod.rs
    paths.rs           // Kategorie -> Registry-Pfade
    scan.rs            // Lesen, Merge über Scopes
    filetypes.rs       // ProgID-Auflösung, Dateitypliste, Kette aus 10.1
    write.rs           // Anlegen, Ändern, Löschen
    blocked.rs         // Blocked-Liste verwalten
    backup.rs          // .reg-Export und Import
  program/
    mod.rs
    cmdline.rs         // argv[0]-Extraktion, Interpreter-Sonderfälle
    identity.rs        // ProgramKey, Anzeigename aus Versionsressource
    group.rs           // Gruppierung, Plan-Erstellung
  icons/
    mod.rs
    parse.rs           // Icon-Referenz parsen
    extract.rs         // SHDefExtractIconW, HICON -> RGBA, RAII-Wrapper
    cache.rs           // Lazy-Loading, TextureHandle-Cache
  elevation.rs         // Token prüfen, runas-Relaunch, Job-Datei
assets/
  app.ico
  app.manifest
```

### 4.3 App-Zustand

Immediate mode bedeutet: `update()` läuft 60-mal pro Sekunde. **Nichts Teures darf im Frame-Pfad stehen.** Kein Registry-Zugriff, keine Icon-Extraktion, keine Versionsressourcen-Abfrage. Alles vorberechnen und im Zustand halten.

```rust
pub struct App {
    // Daten
    scan: Option<ScanResult>,
    groups: Vec<ProgramGroup>,
    filetypes: Vec<FileTypeInfo>,

    // vorberechnete Sicht, wird nur bei Filteränderung neu gebaut
    visible_rows: Vec<usize>,       // Indizes in scan.entries
    filter_dirty: bool,

    // UI-Zustand
    tab: Tab,
    selected_category: Option<Category>,
    selected_ext: Option<String>,
    selected_group: Option<usize>,
    selection: FxHashSet<String>,   // Entry-IDs
    search: String,
    expanded: FxHashSet<String>,    // aufgeklappte Baumknoten

    // Hintergrundarbeit
    scan_rx: Option<Receiver<ScanProgress>>,
    scanning: bool,
    progress: (usize, usize),

    // Dienste
    icons: IconCache,
    tr: &'static i18n::Strings,
    settings: Settings,
    pending_dialog: Option<Dialog>,
}
```

`visible_rows` ist der wichtigste Teil. Filter, Suche und Sortierung werden **nicht** pro Frame ausgewertet, sondern einmal beim Setzen von `filter_dirty`.

### 4.4 Hintergrund-Scan

```rust
// beim Start und bei "Neu scannen"
let (tx, rx) = std::sync::mpsc::channel();
let ctx = ctx.clone();
std::thread::spawn(move || {
    registry::scan::scan_all(|progress| {
        let _ = tx.send(progress);
        ctx.request_repaint();      // weckt den Event-Loop
    });
});
self.scan_rx = Some(rx);
```

In `update()` nur `try_recv()` in einer Schleife leeren, nie blockieren. `ctx.request_repaint()` ist nötig, weil egui sonst schläft, bis eine Eingabe kommt.

Melde Fortschritt pro Kategorie, damit die Liste sich sichtbar füllt statt nach acht Sekunden auf einmal zu erscheinen.

### 4.5 Tabelle

Für die Eintragsliste `egui_extras::TableBuilder` verwenden, **immer** mit `body.rows(row_height, total_rows, |mut row| { … })`. Das ist die virtualisierte Variante: nur sichtbare Zeilen werden gezeichnet. Bei 2.000 Einträgen ist der Unterschied zwischen 60 fps und einer Diashow.

```rust
TableBuilder::new(ui)
    .striped(true)
    .resizable(true)
    .column(Column::exact(28.0))                    // Auswahl-Checkbox
    .column(Column::exact(24.0))                    // Icon
    .column(Column::initial(260.0).at_least(120.0)) // Name
    .column(Column::initial(180.0))                 // Ort
    .column(Column::initial(90.0))                  // Scope
    .column(Column::remainder())                    // Befehl
    .header(24.0, |mut h| { /* ... */ })
    .body(|body| {
        body.rows(26.0, self.visible_rows.len(), |mut row| {
            let e = &self.scan.entries[self.visible_rows[row.index()]];
            // ...
        });
    });
```

**Icons in der Zeile:** die Textur wird im Cache angefordert, aber die Extraktion passiert nicht hier. `IconCache::get()` liefert entweder eine fertige Textur oder einen Platzhalter und stellt die Referenz in eine Warteschlange, die ein Worker-Thread abarbeitet. Siehe 7.4.

### 4.6 Baum links

Kein fertiges Widget nötig, `egui::CollapsingHeader` reicht:

```rust
egui::CollapsingHeader::new(group.label)
    .default_open(true)
    .show(ui, |ui| {
        for ft in &group.types {
            let label = format!("{}  ({})", ft.ext, ft.entry_count);
            if ui.selectable_label(selected == Some(&ft.ext), label).clicked() {
                self.selected_ext = Some(ft.ext.clone());
                self.filter_dirty = true;
            }
        }
    });
```

In einem `SidePanel` mit `ScrollArea::vertical()`.

### 4.7 Drag & Drop für die Sortierung

egui hat das seit 0.27 eingebaut:

```rust
let item_id = egui::Id::new(("row", entry.id.as_str()));
let resp = ui.dnd_drag_source(item_id, entry.id.clone(), |ui| {
    ui.label(&entry.display_name);
}).response;

if let Some(payload) = resp.dnd_hover_payload::<String>() {
    // Einfügemarke zeichnen
}
if let Some(payload) = resp.dnd_release_payload::<String>() {
    self.reorder(&payload, target_index);
}
```

Lies vorher Abschnitt 6, bevor du zu viel Aufwand hier investierst. Windows nimmt die Reihenfolge nur begrenzt an.

---

## 5. Registry-Landkarte

Das klassische Kontextmenü lebt vollständig in der Registry.

### 5.1 Basis-Kategorien

| Kategorie | Pfad |
|---|---|
| Alle Dateien | `HKCR\*\shell` |
| Alle Dateien, COM-Handler | `HKCR\*\shellex\ContextMenuHandlers` |
| Alle Dateisystemobjekte | `HKCR\AllFilesystemObjects\shell` |
| Ordner (Rechtsklick auf Ordner) | `HKCR\Directory\shell` |
| Ordner, COM-Handler | `HKCR\Directory\shellex\ContextMenuHandlers` |
| Leerer Bereich im Ordner | `HKCR\Directory\Background\shell` |
| Leerer Bereich, COM-Handler | `HKCR\Directory\Background\shellex\ContextMenuHandlers` |
| Ordner + Shell-Namespace (ZIP, Bibliotheken) | `HKCR\Folder\shell` |
| Desktop-Hintergrund | `HKCR\DesktopBackground\Shell` |
| Desktop, COM-Handler | `HKCR\DesktopBackground\ShellEx\ContextMenuHandlers` |
| Laufwerke | `HKCR\Drive\shell` |

Dateityp-spezifische Pfade stehen in Abschnitt 10.

### 5.2 HKCR ist eine Sicht, kein Hive

`HKCR` merged `HKLM\SOFTWARE\Classes` (systemweit) und `HKCU\SOFTWARE\Classes` (nutzerspezifisch). HKCU gewinnt bei Konflikten.

**Für den Scanner:** lies beide Hives getrennt und merge selbst. Sonst kannst du weder anzeigen, woher ein Eintrag stammt, noch ob er ohne Elevation löschbar ist.

**Für den Writer:** schreibe standardmäßig nach `HKCU\SOFTWARE\Classes\…`. Keine Elevation nötig, reversibel ohne Systemschaden. Schreiben nach HKCR landet in HKLM und braucht Admin.

`HKLM\SOFTWARE\Classes` hat unter `HKLM\SOFTWARE\WOW6432Node\Classes` einen 32-Bit-Zwilling, den ein 64-Bit-Prozess nur mit `KEY_WOW64_32KEY` sieht. Dort registrieren 32-Bit-Programme ihre Einträge, und genau die fehlen in Konkurrenzprodukten oft. Scanne beide Views, markiere die Herkunft als `Scope::Machine32`.

### 5.3 Aufbau eines Verb-Eintrags

```
HKCU\SOFTWARE\Classes\Directory\shell\MeinTool
    (Default)              = "Mit MeinTool öffnen"
    MUIVerb                = "@C:\tool\t.dll,-101"     ; lokalisiert, hat Vorrang
    Icon                   = "C:\tool\t.exe,0"
    Position               = "Top" | "Bottom"
    Extended               = ""                        ; nur mit Shift sichtbar
    NoWorkingDirectory     = ""
    AppliesTo              = "System.ItemType:.txt"
    SubCommands            = ""                        ; leer = Untermenü aus Subkeys
    LegacyDisable          = ""                        ; blendet Eintrag aus
    ProgrammaticAccessOnly = ""                        ; dito

HKCU\SOFTWARE\Classes\Directory\shell\MeinTool\command
    (Default)              = "\"C:\tool\t.exe\" \"%V\""
```

**Platzhalter:**

- `%1` — Pfad des angeklickten Objekts
- `%V` — funktioniert auch bei Background-Kategorien, wo `%1` leer bleibt
- `%W` — Arbeitsverzeichnis

Für `Directory\Background\shell` und `DesktopBackground\Shell` **immer `%V`**. Das ist der häufigste Fehler bei handgebauten Einträgen. Der Editor soll beim Speichern warnen, wenn in einer Background-Kategorie `%1` steht.

**Untermenüs:** `MUIVerb` plus leerer `SubCommands`-Wert am Elternkey, Kinder unter `<Elternkey>\shell\<Kind>`. Alternativ listet `SubCommands` semikolongetrennt Verben aus dem CommandStore.

### 5.4 COM-Handler (`shellex`)

```
HKCR\Directory\shellex\ContextMenuHandlers\7-Zip
    (Default) = "{23170F69-40C1-278A-1000-000100020000}"
```

Der Anzeigetext entsteht zur Laufzeit über `IContextMenu::QueryContextMenu` und steht nicht in der Registry. Für v1 zeigst du:

- den Subkey-Namen,
- den Klarnamen der CLSID aus `HKCR\CLSID\{…}\(Default)`,
- den DLL-Pfad aus `HKCR\CLSID\{…}\InprocServer32`.

Der DLL-Pfad ist gleichzeitig der Gruppierungsschlüssel für die Programm-Ansicht.

**Blockieren statt löschen:**

```
HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Blocked
    {CLSID} = ""    (REG_SZ, Wert beliebig)
```

Braucht Admin, ist aber reversibel und übersteht Programm-Updates, die den `shellex`-Key sonst wieder anlegen. **Ein einziger Blocked-Eintrag ersetzt das Löschen desselben Handlers unter zwanzig Klassen.** Bevorzugte Aktion in der Programm-Ansicht, siehe Abschnitt 11.

### 5.5 CommandStore

```
HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell
```

Windows-eigene Verben, meist im Besitz von TrustedInstaller. **Nur lesen.** In der GUI mit Schloss-Symbol markieren, Löschbutton deaktivieren.

### 5.6 MUI-Strings auflösen

Werte wie `@%SystemRoot%\system32\shell32.dll,-8506` sind indirekte Ressourcen-Referenzen:

```rust
use windows::Win32::UI::Shell::SHLoadIndirectString;
use windows::core::{PCWSTR, HSTRING};

fn resolve_mui(raw: &str) -> Option<String> {
    if !raw.starts_with('@') {
        return Some(raw.to_string());
    }
    let src = HSTRING::from(raw);
    let mut buf = vec![0u16; 512];
    unsafe { SHLoadIndirectString(PCWSTR(src.as_ptr()), &mut buf, None).ok()? };
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}
```

Schlägt das fehl, zeige den Rohstring. Nicht panicken. Ergebnisse cachen, dieselbe Referenz taucht dutzendfach auf.

---

## 6. Reihenfolge: was tatsächlich geht

Hier versprechen viele Tools mehr, als Windows hergibt. Sei in der GUI ehrlich darüber.

**Steuerbar:**

1. `Position = "Top"` oder `"Bottom"` am Verb-Key. Grobe Einteilung in drei Blöcke, funktioniert zuverlässig.
2. Der Default-Wert von `HKCR\<Klasse>\shell` nimmt eine Liste von Verb-Namen entgegen. Das erste Element ist das Standard-Verb, also die Doppelklick-Aktion. Ob die restliche Listenreihenfolge die Anzeige beeinflusst, ist **nicht dokumentiert und versionsabhängig**. Als Trennzeichen findet man in freier Wildbahn Leerzeichen und Komma.
3. Reihenfolge innerhalb eines `SubCommands`-Untermenüs: die semikolongetrennte Liste greift zuverlässig.

**Nicht steuerbar:**

- Position von COM-Handler-Einträgen. Die vergeben ihre Menü-IDs selbst.
- Absolute Sortierung aller Einträge. Einen `SortOrder`-Wert gibt es nicht.

**Auftrag:** Punkt 2 als experimentelles Feature mit sichtbarem Hinweis bauen und auf Windows 10 empirisch verifizieren. Greift es nicht, das Drag & Drop aus 4.7 auf `Position` reduzieren. Das Ergebnis in dieser Datei nachtragen.

---

## 7. Icons

### 7.1 Referenz parsen

Formate: `"C:\pfad\datei.exe,3"`, `"C:\pfad\icon.ico"`, `"%SystemRoot%\system32\shell32.dll,-244"`.

- Umgebungsvariablen mit `ExpandEnvironmentStringsW` expandieren.
- Kein Index → 0.
- **Negativer Index ist eine Ressourcen-ID, kein Index.** `ExtractIconExW` und `SHDefExtractIconW` erwarten den negativen Wert unverändert.
- Anführungszeichen im Pfad tolerieren, auch wenn danach ein Komma folgt.

### 7.2 Extraktion

`SHDefExtractIconW` ist `ExtractIconExW` vorzuziehen, weil du die Zielgröße angibst und DPI-korrekte Ergebnisse bekommst:

```rust
use windows::Win32::UI::Shell::SHDefExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{HICON, DestroyIcon};

unsafe {
    let mut icon = HICON::default();
    SHDefExtractIconW(PCWSTR(path.as_ptr()), index, 0, Some(&mut icon), None, 32).ok()?;
    // -> RGBA konvertieren
    DestroyIcon(icon)?;
}
```

### 7.3 HICON nach RGBA

Weg: `GetIconInfo` → `hbmColor`, `hbmMask` → `GetObjectW` für die Dimensionen → `CreateDIBSection` mit 32bpp top-down → `DrawIconEx` in den DIB → Bytes lesen.

Fallstricke:

- Windows liefert BGRA, egui erwartet RGBA. Kanäle tauschen.
- Bei alten 4bpp- und 8bpp-Icons ist jedes Alpha-Byte 0. Heuristik: sind alle Alpha-Werte 0, baue Alpha aus `hbmMask` (Maskenbit gesetzt bedeutet transparent).
- `hbmColor` und `hbmMask` aus `GetIconInfo` müssen **beide** per `DeleteObject` freigegeben werden. Ohne das leckt jeder Scan GDI-Objekte, und bei mehreren tausend Einträgen läufst du in das Prozesslimit von 10.000.

Bau einen RAII-Wrapper mit `Drop` für `HICON`, `HBITMAP` und `HDC`. Dann vergisst du es nicht.

### 7.4 Cache und Lazy-Loading

Das ist bei egui wichtiger als bei einem retained-mode Framework, weil `update()` pro Frame läuft.

```rust
pub struct IconCache {
    textures: FxHashMap<String, TextureHandle>,
    pending: FxHashSet<String>,
    tx: Sender<String>,              // Anforderungen an den Worker
    rx: Receiver<(String, ColorImage)>, // fertige Bilder
    placeholder: TextureHandle,
}

impl IconCache {
    /// Wird pro sichtbarer Zeile aufgerufen. Muss billig sein.
    pub fn get(&mut self, icon_ref: &str) -> &TextureHandle {
        if let Some(t) = self.textures.get(icon_ref) { return t; }
        if self.pending.insert(icon_ref.to_string()) {
            let _ = self.tx.send(icon_ref.to_string());
        }
        &self.placeholder
    }

    /// Einmal pro Frame am Anfang von update() aufrufen.
    pub fn poll(&mut self, ctx: &egui::Context) {
        for (key, img) in self.rx.try_iter().take(16) {
            let t = ctx.load_texture(&key, img, TextureOptions::default());
            self.pending.remove(&key);
            self.textures.insert(key, t);
        }
    }
}
```

Zwei Punkte:

- **`take(16)` pro Frame.** Ohne Deckel lädt der erste Frame nach einem Scan 800 Texturen auf einmal hoch und die App friert kurz ein.
- **Der Worker-Thread ruft COM-Funktionen auf.** `CoInitializeEx` mit `COINIT_APARTMENTTHREADED` einmal pro Thread, sonst schlägt `SHDefExtractIconW` bei manchen Icon-Quellen fehl.

Key ist die normalisierte Icon-Referenz nach Expansion. Ein Vollscan mit Dateitypen trifft leicht 2.000 Einträge, von denen die Hälfte auf `shell32.dll` oder `imageres.dll` zeigt, also greift der Cache stark.

---

## 8. Mehrsprachigkeit (DE/EN)

Bei egui ist das eine normale Rust-Struct, kein Framework-Thema.

```rust
// src/i18n.rs
pub struct Strings {
    pub app_title: &'static str,
    pub tab_categories: &'static str,
    pub tab_filetypes: &'static str,
    pub tab_programs: &'static str,
    pub tab_backups: &'static str,
    pub btn_delete: &'static str,
    pub btn_disable: &'static str,
    pub btn_block: &'static str,
    pub btn_restore: &'static str,
    pub col_name: &'static str,
    pub col_location: &'static str,
    pub col_command: &'static str,
    pub msg_needs_admin: &'static str,
    pub msg_confirm_delete: &'static str,
    // ...
}

pub static DE: Strings = Strings {
    app_title: "Kontextmenü-Manager",
    tab_categories: "Kategorien",
    btn_delete: "Löschen",
    /* ... */
};

pub static EN: Strings = Strings {
    app_title: "Context Menu Manager",
    tab_categories: "Categories",
    btn_delete: "Delete",
    /* ... */
};
```

Im App-Zustand liegt `tr: &'static Strings`. Verwendung: `ui.button(self.tr.btn_delete)`. Sprachwechsel ist eine Zuweisung, wirkt beim nächsten Frame. Kein Setter, keine Bindings, kein Neustart.

**Startsprache** aus `GetUserDefaultUILanguage` ableiten: Primary Language ID `0x07` bedeutet Deutsch, alles andere fällt auf Englisch zurück. Die Wahl in der Settings-Datei persistieren.

**Formatierte Texte** (etwa "78 Einträge betroffen") als `&'static str` mit Platzhalter halten und mit `format!` füllen, nicht als fertigen Satz. Deutsch und Englisch haben unterschiedliche Wortstellung.

**Nicht übersetzt** werden Registry-Pfade, Verb-Namen, Kommandozeilen und die von `SHLoadIndirectString` gelieferten Anzeigenamen. Die kommen bereits in der Systemsprache.

---

## 9. Theming: Dark, Light, System

### 9.1 egui-Seite

Seit egui 0.31 gibt es `ThemePreference`, das genau deine Dreier-Auswahl abbildet:

```rust
ctx.set_theme(egui::ThemePreference::System);  // oder Dark | Light
```

`System` fragt eframe nach dem Betriebssystem-Theme; der winit-Backend liest das unter Windows selbst aus und meldet Änderungen zur Laufzeit weiter. Ein eigener Registry-Watcher ist damit normalerweise überflüssig.

**Verifiziere das auf der Entwicklungsmaschine.** Reagiert `ThemePreference::System` auf deinem Win10-Build nicht auf einen Themenwechsel, ist der Fallback ein `RegNotifyChangeKeyValue`-Watcher auf:

```
HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize
    AppsUseLightTheme    (REG_DWORD)   0 = dunkel, 1 = hell
```

Eigener Thread, `REG_NOTIFY_CHANGE_LAST_SET`, bei Signal `ctx.set_theme()` plus `ctx.request_repaint()`. Rund zwanzig Zeilen.

### 9.2 Eigene Farben

Nie feste Farben schreiben, sondern aus `ui.visuals()` ableiten:

```rust
let fg = ui.visuals().text_color();
let warn = ui.visuals().warn_fg_color;
let err = ui.visuals().error_fg_color;
let weak = ui.visuals().weak_text_color();
```

Für die Badges (Admin-Schild, "blockiert", "nur mit Shift", "schreibgeschützt") reicht das nicht, weil dieselbe RGB-Farbe in beiden Themes unterschiedlich lesbar ist. Bau eine kleine Palette in `ui/style.rs`:

```rust
pub struct Badges { pub admin: Color32, pub blocked: Color32, pub shift: Color32 }

pub fn badges(visuals: &egui::Visuals) -> Badges {
    if visuals.dark_mode {
        Badges { admin: Color32::from_rgb(255, 196, 0), /* ... */ }
    } else {
        Badges { admin: Color32::from_rgb(150, 100, 0), /* ... */ }
    }
}
```

### 9.3 Schrift

egui lädt standardmäßig eine eigene Schrift, nicht Segoe UI. Auf einem Windows-Systemwerkzeug fällt das auf. Lade Segoe UI beim Start:

```rust
let mut fonts = egui::FontDefinitions::default();
if let Ok(data) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
    fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(data).into());
    fonts.families.get_mut(&egui::FontFamily::Proportional)
        .unwrap().insert(0, "segoe".into());
}
ctx.set_fonts(fonts);
```

Schlägt das Lesen fehl, bleibt die Standardschrift. Nicht panicken.

Segoe UI Variable (`SegUIVar.ttf`, ab Win11) ist die modernere Datei, existiert auf Win10 aber nicht. Erst `segoeui.ttf` versuchen, das gibt es überall.

### 9.4 Titelleiste

eframe lässt Windows den Fensterrahmen zeichnen, also bleibt die Titelleiste hell, bis du DWM fragst:

```rust
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Foundation::{HWND, BOOL};

const DWMWA_USE_IMMERSIVE_DARK_MODE: i32 = 20;
const DWMWA_USE_IMMERSIVE_DARK_MODE_OLD: i32 = 19;

unsafe fn set_titlebar_dark(hwnd: HWND, dark: bool) {
    let value = BOOL::from(dark);
    let ptr = &value as *const _ as *const core::ffi::c_void;
    let size = std::mem::size_of::<BOOL>() as u32;
    if DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE), ptr, size).is_err() {
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE_OLD), ptr, size);
    }
}
```

**Wichtig für die Entwicklungsmaschine:** Attribut 20 gilt ab Windows-10-Build 18985. Auf 1809 und 1903 ist es 19, daher der Fallback. Vor Build 17763 funktioniert beides nicht, dort bleibt die Titelleiste hell. Das ist akzeptabel.

HWND aus dem eframe-Fenster über `raw-window-handle`:

```rust
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

// in App::new(cc) oder beim ersten update()
if let Ok(h) = cc.window_handle() {
    if let RawWindowHandle::Win32(w) = h.as_raw() {
        unsafe { set_titlebar_dark(HWND(w.hwnd.get() as *mut _), dark) };
    }
}
```

Der Aufruf muss nach dem ersten Anzeigen des Fensters erfolgen. Auf manchen Builds schlägt die Titelleiste erst nach einem Redraw um, also danach einmal `SetWindowPos` mit `SWP_FRAMECHANGED` hinterherschicken. Bei jedem Themenwechsel erneut aufrufen.

---

## 10. Dateityp-Ansicht

Das ist die anspruchsvollste Lesefunktion. Beim Rechtsklick auf eine `.jpg` sieht der Nutzer Einträge aus mindestens sieben verschiedenen Registry-Ästen.

### 10.1 Auflösungskette für eine Erweiterung

Für `.jpg` in dieser Reihenfolge sammeln:

| # | Quelle | Pfad |
|---|---|---|
| 1 | Alle Dateien | `HKCR\*\shell` und `HKCR\*\shellex\ContextMenuHandlers` |
| 2 | Alle Dateisystemobjekte | `HKCR\AllFilesystemObjects\shell` |
| 3 | Wahrgenommener Typ | `HKCR\SystemFileAssociations\image\shell` |
| 4 | Erweiterung direkt | `HKCR\SystemFileAssociations\.jpg\shell` |
| 5 | ProgID | `HKCR\<ProgID>\shell` und `HKCR\<ProgID>\shellex\ContextMenuHandlers` |
| 6 | Erweiterungs-Key | `HKCR\.jpg\shell` (selten, existiert aber) |
| 7 | Weitere ProgIDs | jede ProgID aus `HKCR\.jpg\OpenWithProgids` |

Ebene 3 und 4 werden von den meisten Tools übersehen. Genau dort registrieren sich Bildbetrachter, Konverter und viele Fotoprogramme.

Jede Ebene bekommt in der Anzeige eine eigene Herkunftsbezeichnung, siehe 10.4.

### 10.2 ProgID ermitteln

Zwei Quellen, in dieser Priorität:

```
1. HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.jpg\UserChoice
       ProgId = "PhotoViewer.FileAssoc.Jpeg"       ; Nutzerwahl, hat Vorrang
2. HKCR\.jpg\(Default) = "jpegfile"                 ; Systemvorgabe
```

Zusätzlich alle Werte-Namen unter `HKCR\.jpg\OpenWithProgids` einsammeln. Das sind alternative ProgIDs, deren `shell`-Zweige ebenfalls Einträge beisteuern.

`PerceivedType` steht in `HKCR\.jpg\PerceivedType`. Mögliche Werte: `text`, `image`, `audio`, `video`, `compressed`, `document`, `system`, `application`. Fehlt der Wert, entfällt Ebene 3.

### 10.3 Kuratierte Dateitypliste

Als `const`-Tabelle im Code, gruppiert für die GUI. Startpunkt, nicht Grenze.

```rust
pub struct FileTypeDef { pub ext: &'static str, pub group: TypeGroup }

// Dokumente
".pdf" ".doc" ".docx" ".xls" ".xlsx" ".ppt" ".pptx" ".txt" ".rtf"
".odt" ".ods" ".csv" ".md" ".epub"

// Bilder
".jpg" ".jpeg" ".png" ".gif" ".bmp" ".tif" ".tiff" ".webp" ".heic"
".svg" ".ico" ".psd" ".xcf"

// RAW
".cr2" ".cr3" ".nef" ".arw" ".dng" ".raf" ".orf" ".rw2"

// Audio
".mp3" ".flac" ".wav" ".m4a" ".ogg" ".opus" ".aac" ".wma"

// Video
".mp4" ".mkv" ".avi" ".mov" ".webm" ".wmv" ".flv" ".m2ts" ".mpg"

// Archive
".zip" ".rar" ".7z" ".tar" ".gz" ".bz2" ".xz" ".iso" ".cab"

// Code und Konfiguration
".py" ".rs" ".go" ".js" ".ts" ".jsx" ".tsx" ".c" ".cpp" ".h" ".cs"
".java" ".rb" ".php" ".sql" ".json" ".yaml" ".yml" ".toml" ".xml"
".html" ".css" ".sh" ".ps1" ".bat" ".cmd" ".ini" ".conf" ".log"

// System
".exe" ".dll" ".msi" ".lnk" ".reg" ".sys" ".vhd" ".vhdx"
```

Dazu zwei UI-Funktionen:

- **Eigene Erweiterung hinzufügen:** Eingabefeld, prüft ob `HKCR\.<ext>` existiert, nimmt sie in die persistierte Liste auf.
- **Alle installierten Typen scannen:** enumeriert alle `HKCR`-Subkeys, die mit `.` beginnen. Typischerweise 400 bis 900 Stück. Nur auf ausdrückliche Anforderung und im Hintergrund ausführen, sonst dauert der Kaltstart zu lang. Ergebnis nach Anzahl gefundener Einträge sortieren, damit die interessanten Typen oben stehen.

### 10.4 Anzeige

Links ein `SidePanel` mit Gruppenbaum (Dokumente, Bilder, RAW, Audio, …), Erweiterungen darunter, jeweils mit Zähler. Typen ohne eigene Einträge ausgrauen oder per Checkbox ausblenden.

Rechts die Eintragstabelle mit Herkunftsspalte, die zeigt, aus welcher der sieben Ebenen der Eintrag stammt.

**Der Nutzer muss verstehen, dass Löschen auf Ebene 1 (`HKCR\*\shell`) alle Dateitypen betrifft, nicht nur die markierte `.jpg`.** Der Löschdialog soll die Reichweite beziffern, etwa: "Dieser Eintrag gilt für alle Dateien. Löschen entfernt ihn auch bei 78 anderen Dateitypen."

**Caching:** Ebene 1 und 2 sind für alle Dateitypen identisch. Einmal scannen, wiederverwenden.

---

## 11. Programm-Ansicht und gruppiertes Löschen

Die Kernfunktion. Ein Programm wie 7-Zip, VLC oder IrfanView legt Einträge unter zehn bis zwanzig Klassen ab. Das einzeln zu löschen ist genau die Arbeit, die dieses Tool abnehmen soll.

### 11.1 Identität eines Programms bestimmen

Aus jedem gescannten Eintrag einen `ProgramKey` ableiten.

**Für `Verb`-Einträge:** Zielprogramm aus dem `command`-Wert extrahieren.

```
1. Umgebungsvariablen expandieren (ExpandEnvironmentStringsW)
2. argv[0] extrahieren, Windows-Quoting beachten:
   - beginnt der String mit ", geht argv[0] bis zum nächsten "
   - sonst bis zum ersten Leerzeichen
3. Sonderfälle: zeigt argv[0] auf einen Interpreter oder Loader,
   ist das eigentliche Ziel argv[1]:
       rundll32.exe   "shell32.dll,Control_RunDLL foo.cpl" -> DLL-Name
       regsvr32.exe
       mshta.exe
       cmd.exe /c
       powershell.exe / pwsh.exe
       wscript.exe / cscript.exe
       explorer.exe
4. Normalisieren: Kleinschreibung, / durch \ ersetzen,
   doppelte Backslashes zusammenfassen
5. ProgramKey = normalisierter Vollpfad
```

**Für `ShellEx`-Einträge:** `ProgramKey` ist der normalisierte Pfad aus `HKCR\CLSID\{…}\InprocServer32`. Die CLSID zusätzlich am Eintrag behalten, sie wird für die Blocked-Liste gebraucht.

**Anzeigename der Gruppe**, in dieser Reihenfolge versuchen:

1. `FileDescription` aus der Versionsressource der EXE oder DLL (`GetFileVersionInfoW` plus `VerQueryValueW` auf `\StringFileInfo\<lang>\FileDescription`)
2. `ProductName` aus derselben Ressource
3. Klarname der CLSID aus `HKCR\CLSID\{…}\(Default)`
4. Dateiname ohne Erweiterung

Damit steht in der Liste "7-Zip Shell Extension" statt `c:\program files\7-zip\7-zip.dll`.

Das Auslesen der Versionsressource ist teuer. **Einmal nach dem Scan im Worker-Thread erledigen**, Ergebnis im `ProgramGroup` speichern, nie im Frame-Pfad.

Zeigt der Pfad ins Windows-Verzeichnis, markiere die Gruppe als Systemkomponente und blende eine Warnung ein.

### 11.2 Gruppierung

```rust
pub struct ProgramGroup {
    pub key: String,                 // normalisierter Pfad
    pub display_name: String,
    pub icon_ref: Option<String>,
    pub entry_indices: Vec<usize>,   // Indizes in ScanResult.entries
    pub clsids: Vec<String>,         // eindeutige CLSIDs, für Blocked-Liste
    pub scopes: Vec<Scope>,
    pub locations: Vec<String>,      // menschenlesbar, für die Zusammenfassung
    pub is_system: bool,
}
```

Indizes statt geklonter Einträge, dann bleibt die Wahrheit an einer Stelle und die Auswahl in der Tabelle bleibt konsistent.

Nach dem Vollscan über alle Einträge iterieren und in eine `FxHashMap<String, ProgramGroup>` einsortieren, danach in einen `Vec` sortiert nach Anzahl absteigend, dann alphabetisch.

Die Detailansicht listet jedes Vorkommen mit Ort und Scope, jedes einzeln abwählbar. Voreinstellung: alles ausgewählt außer schreibgeschützten Einträgen.

### 11.3 Aktionen auf einer Gruppe

Vier Stufen, von sanft nach hart. Die GUI soll sie in dieser Reihenfolge anbieten, nicht mit "Löschen" beginnen.

| Aktion | Wirkung | Reversibel | Admin |
|---|---|---|---|
| **Ausblenden** | `LegacyDisable=""` an jedem Verb-Key | ja, trivial | je nach Scope |
| **Nur mit Shift** | `Extended=""` an jedem Verb-Key | ja, trivial | je nach Scope |
| **Blockieren** | CLSID in die Blocked-Liste, ein Eintrag pro Handler | ja | immer |
| **Löschen** | Verb-Keys rekursiv entfernen | nur über Backup | je nach Scope |

Für COM-Handler ist "Blockieren" fast immer die richtige Wahl: es ersetzt zwanzig Löschungen durch einen Registry-Wert, überlebt Programm-Updates und lässt sich mit einem Klick zurücknehmen. Der Löschbutton für `ShellEx`-Einträge soll darauf hinweisen.

Für `Verb`-Einträge gibt es kein Äquivalent zur Blocked-Liste, dort ist `LegacyDisable` die sanfte Variante.

### 11.4 Transaktionale Ausführung

Eine Gruppenaktion trifft bis zu dreißig Registry-Keys über zwei oder drei Hives.

```
1. Plan aufstellen: Liste von Operationen (Pfad, Scope, Art)
2. Nach Scope partitionieren
3. EIN Backup über alle betroffenen Pfade, vor der ersten Änderung
4. Reihenfolge: erst HKCU (kein Admin), dann HKLM
5. Braucht der HKLM-Teil Elevation: als Job-Datei serialisieren
   und den elevated Prozess starten (Abschnitt 13.2)
6. Jede Einzeloperation protokollieren, Fehler sammeln statt abbrechen
7. Ergebnisdialog mit Erfolgen und Fehlern,
   bei Teilfehlern Restore anbieten
8. SHChangeNotify senden, danach Rescan der betroffenen Kategorien
```

Ein Rollback mitten in der Operation ist unnötig kompliziert. Backup plus deutlicher Restore-Button im Ergebnisdialog reicht und ist robuster.

**egui-spezifisch:** die Ausführung läuft im Worker-Thread. Der Bestätigungsdialog setzt `self.pending_dialog`, der Klick auf "Ausführen" startet den Thread und setzt einen Busy-Zustand. Solange der läuft, Aktionsbuttons deaktivieren, sonst startet ein zweiter Klick eine parallele Transaktion.

### 11.5 Suche

Ein Suchfeld in der Top-Leiste, wirkt auf alle drei Tabs. Sucht gleichzeitig in Anzeigename, Verb-Name, Kommandozeile und Registry-Pfad. Case-insensitiv, Teilstring-Match reicht.

**Wichtig für die Performance:** Der Filter läuft nicht pro Frame. Bei `response.changed()` des Textfelds `filter_dirty = true` setzen, und am Anfang des nächsten `update()` einmal `visible_rows` neu bauen.

---

## 12. Datenmodell

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope { User, Machine, Machine32 }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Category {
    AllFiles,
    AllFilesystemObjects,
    Directory,
    DirectoryBackground,
    Folder,
    DesktopBackground,
    Drive,
    /// SystemFileAssociations\<perceived>
    PerceivedType(String),
    /// SystemFileAssociations\.<ext>
    ExtAssoc(String),
    /// <ProgID>, mit Herkunfts-Erweiterung für die Anzeige
    ProgId { prog_id: String, from_ext: String },
    /// .<ext>\shell
    ExtDirect(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntryKind {
    Verb {
        command: Option<String>,
        sub_commands: Vec<ContextEntry>,
    },
    ShellEx {
        clsid: String,
        server_path: Option<String>,
        blocked: bool,
    },
}

#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub id: String,                  // stabiler Hash aus registry_path + scope
    pub key_name: String,
    pub display_name: String,
    pub raw_display: Option<String>,
    pub icon_ref: Option<String>,
    pub position: Option<String>,
    pub extended: bool,
    pub hidden: bool,
    pub applies_to: Option<String>,
    pub kind: EntryKind,
    pub scope: Scope,
    pub category: Category,
    pub registry_path: String,       // vollständig, für Backup und Delete
    pub read_only: bool,
    pub program_key: Option<String>, // Gruppierungsschlüssel
}

pub struct ScanResult {
    pub entries: Vec<ContextEntry>,
    pub by_category: FxHashMap<Category, Vec<usize>>,
    pub by_program: FxHashMap<String, Vec<usize>>,
    pub scanned_at: chrono::DateTime<chrono::Local>,
}
```

Der Rest der App arbeitet mit `usize`-Indizes in `entries`. Das hält die Auswahl über Filterwechsel hinweg konsistent und spart Klonen im Frame-Pfad.

---

## 13. Backup, Elevation, Aktualisierung

### 13.1 Backup

**Vor jeder Schreiboperation** ein `.reg`-Export der betroffenen Teilbäume nach:

```
%LOCALAPPDATA%\ctxmenu\backups\<ISO8601>_<aktion>\
    01_HKCU_Classes_jpegfile_shell.reg
    02_HKLM_Classes_star_shellex.reg
    manifest.json
```

Umsetzung über `reg.exe export "<pfad>" "<ziel>" /y` per `std::process::Command`, mit `CREATE_NO_WINDOW` als Creation-Flag (`std::os::windows::process::CommandExt::creation_flags(0x0800_0000)`), sonst blitzt ein Konsolenfenster auf. `reg.exe` ist auf jedem Windows vorhanden und behandelt Sonderzeichen und Binärwerte korrekt. Ein eigener rekursiver Exporter lohnt sich für v1 nicht.

`manifest.json` je Backup: Zeitstempel, Aktion, betroffene Keys, Scope, Anzahl Einträge, Programmname bei Gruppenaktionen.

Der Verlaufs-Tab listet Backups und spielt sie per `reg.exe import` zurück. Beachte: `reg import` fügt hinzu und überschreibt, entfernt aber keine später hinzugekommenen Keys. Für einen Restore nach einem Löschvorgang reicht das, weil genau die gelöschten Keys wieder entstehen.

**Ohne funktionierenden Restore keine Löschfunktion freischalten.** Ein zerschossener `HKCR\Directory\shell` fällt erst auf, wenn der Explorer beim nächsten Rechtsklick hängt.

### 13.2 Elevation

Manifest auf `asInvoker`. HKCU-Operationen laufen direkt. Erst wenn eine HKLM-Operation ansteht, startest du dich selbst neu:

```rust
// Plan als JSON nach %TEMP%\ctxmenu_job_<uuid>.json schreiben
ShellExecuteW(None, w!("runas"), exe_path, w!("--elevated-job <pfad>"), None, SW_HIDE)
```

Der elevated Prozess führt den Job aus, schreibt ein Ergebnis-JSON daneben und beendet sich. Der Hauptprozess wartet im Worker-Thread auf den Prozess-Handle, liest das Ergebnis und meldet es per Channel zurück an die UI.

Wichtig: der Job-Modus wird in `main()` **vor** `run_native()` abgefangen, sonst öffnet der elevated Prozess ein zweites Fenster.

Aktuellen Status mit `GetTokenInformation` und `TokenElevation` prüfen und in der Statusleiste anzeigen. Einträge, die Admin brauchen, mit Schildsymbol markieren statt sie zu deaktivieren.

Bricht der Nutzer den UAC-Dialog ab, liefert `ShellExecuteW` `ERROR_CANCELLED` (1223). Das ist kein Fehler, sondern eine Nutzerentscheidung, entsprechend behandeln.

### 13.3 app.manifest

Braucht:

- `requestedExecutionLevel level="asInvoker" uiAccess="false"`
- `supportedOS` mit der GUID für Windows 10 und 11 (`{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}`), sonst meldet Windows eine falsche Version
- `<dpiAwareness>PerMonitorV2</dpiAwareness>` plus das ältere `<dpiAware>true/pm</dpiAware>` für Win10-Builds vor 1703
- `<activeCodePage>UTF-8</activeCodePage>` schadet nicht

`main.rs` mit `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`, damit Release ohne Konsole startet, Debug aber `println!` zeigt.

### 13.4 Änderungen sichtbar machen

```rust
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
```

Das reicht für `shell`-Verben. Bei `shellex`-Änderungen und bei der Blocked-Liste hilft nur ein Neustart von `explorer.exe`, weil der Explorer geladene Handler-DLLs cacht. Als Button anbieten, nie automatisch ausführen.

---

## 14. Später: Windows-11-Hauptmenü

Nicht in v1 bauen. Diese Punkte nur berücksichtigen, damit die Architektur sich nichts verbaut:

- Zweite Crate im selben Workspace, `crate-type = ["cdylib"]`, implementiert `IExplorerCommand` über `windows-rs` und `#[implement]`.
- Registrierung ausschließlich über ein **Sparse MSIX Package** mit `desktop4:FileExplorerContextMenus`-Extension. Registry-Einträge greifen dort nicht.
- Das Package muss signiert sein. Für Eigengebrauch: selbstsigniertes Zertifikat nach `LocalMachine\Root` und `LocalMachine\TrustedPeople`, dann `Add-AppxPackage -ExternalLocation`.
- **Konsequenz für v1:** eigene Einträge nicht nur in die Registry schreiben, sondern parallel in `%LOCALAPPDATA%\ctxmenu\entries.json` mit demselben Datenmodell. Der spätere Handler liest diese Datei und baut daraus dynamisch Einträge. So muss die DLL genau einmal gebaut und signiert werden, und die GUI schreibt nur noch JSON.

---

## 15. Meilensteine

| # | Inhalt | Fertig, wenn |
|---|---|---|
| 1 | Registry-Scanner Basis-Kategorien, CLI-Ausgabe | `cargo run -- scan --category directory` listet alle Verben mit korrektem Scope, inklusive WOW6432Node |
| 2 | MUI-Auflösung, `shellex`-Erkennung, CLSID-Auflösung | Anzeigenamen decken sich mit dem echten Rechtsklickmenü |
| 3 | Backup und Restore über `reg.exe` | Löschen und Wiederherstellen reproduzierbar, auch bei Pfaden mit Leerzeichen |
| 4 | eframe-Grundgerüst, Kategoriebaum, `TableBuilder`, Detailpanel | Alle Basis-Kategorien navigierbar, Scan im Hintergrundthread, 60 fps bei 2.000 Zeilen |
| 5 | i18n DE/EN, `ThemePreference`, Segoe UI, DWM-Titelleiste | Umschalten ohne Neustart, Titelleiste folgt auf Win10 |
| 6 | Icon-Extraktion, Worker-Thread, Texture-Cache | Keine GDI-Leaks nach fünf Vollscans (Task-Manager, Spalte "GDI-Objekte"), kein Ruckeln beim Scrollen |
| 7 | Dateityp-Ansicht mit voller Auflösungskette | Rechtsklickmenü einer `.jpg` und `.pdf` deckt sich mit der Anzeige |
| 8 | Programm-Gruppierung, Kommandozeilen-Parser | 7-Zip oder ein anderes Multi-Typ-Programm erscheint als eine Gruppe mit allen Vorkommen |
| 9 | Gruppenaktionen: Ausblenden, Blockieren, Löschen | Transaktion über HKCU und HKLM inklusive Elevation-Flow funktioniert |
| 10 | Eigene Einträge anlegen, HKCU, plus `entries.json` | Neuer Eintrag erscheint nach `SHChangeNotify` im Explorer |
| 11 | Reihenfolge: `Position` plus Experiment aus Abschnitt 6 | Verhalten auf Win10 verifiziert und hier dokumentiert |
| 12 | Untermenüs, Suche, Release-Build | Binary unter 15 MB, Kaltstart unter 2 s bis zur ersten sichtbaren Liste |

---

## 16. Testumgebung und Testfälle

Arbeite in einer VM oder mit einem Wegwerf-Nutzerprofil. Snapshot vor der ersten Schreiboperation.

**Registry und Shell:**

- Anzeigename mit Umlauten, Leerzeichen und `&`, denn `&` erzeugt im Menü einen Accelerator.
- Kommando mit `%V` unter `Directory\Background`, aufgerufen vom Desktop.
- Icon aus `shell32.dll` mit negativer Ressourcen-ID.
- Eintrag, der nur in `WOW6432Node` existiert.
- Löschversuch auf einem CommandStore-Key ohne Admin: muss sauber fehlschlagen, nicht panicken.
- Key mit leerem `(Default)`: dann ist der Subkey-Name der Anzeigename.
- Kommandozeile ohne Anführungszeichen mit Leerzeichen im Pfad (`C:\Program Files\Tool\t.exe %1`): der Parser muss `argv[0]` trotzdem korrekt raten. Heuristik: Kandidatenpfade schrittweise verlängern und mit `Path::exists()` prüfen.
- `rundll32.exe`-Eintrag: muss der DLL zugeordnet werden, nicht dem rundll32.
- Ein Programm, das denselben Handler unter zwölf Klassen registriert: die Gruppierung muss genau zwölf Vorkommen zeigen, das Blockieren aber nur einen Registry-Wert schreiben.
- UAC-Dialog abbrechen: kein Absturz, keine halbe Transaktion.

**GUI und Performance:**

- Scrollen durch 2.000 Einträge: konstante Framerate, kein Nachladeruckeln.
- Suchfeld tippen bei 2.000 Einträgen: keine spürbare Verzögerung pro Anschlag.
- Themenwechsel im Windows-Einstellungsdialog bei laufender App: Fenster und Titelleiste folgen ohne Neustart.
- Sprachwechsel DE nach EN bei geöffneter Detailansicht: keine leeren Labels, keine abgeschnittenen Buttons (deutsche Wörter sind länger).
- HiDPI: Skalierung 150 % auf Win10, Icons und Text scharf.
- Fenster verkleinern auf Mindestgröße: keine überlappenden Panels.

---

## 17. Referenzen

- Shell-Verben und statische Verben: `learn.microsoft.com/windows/win32/shell/launch`
- Verb-Werte im Detail: `learn.microsoft.com/windows/win32/shell/context-menu`
- Kaskadierende Menüs: `learn.microsoft.com/windows/win32/shell/context-menu-handlers`
- Dateizuordnungen: `learn.microsoft.com/windows/win32/shell/fa-file-types`
- `DWMWA_USE_IMMERSIVE_DARK_MODE`: `learn.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute`
- egui: `docs.rs/egui`, Beispiele unter `github.com/emilk/egui/tree/master/examples`
- egui_extras `TableBuilder`: `docs.rs/egui_extras/latest/egui_extras/struct.TableBuilder.html`
- windows-rs: `github.com/microsoft/windows-rs`

Als Nachschlagewerk für Registry-Semantik lohnen ShellMenuView und Nilesoft Shell (MIT, C++). Kein Code kopieren, nur nachschlagen, wo Windows sich unerwartet verhält.
