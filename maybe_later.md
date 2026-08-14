# Zurückgestellt

Was bewusst nicht gebaut wurde, mit dem Grund und dem Stand der Vorarbeit.
Alles hier ist eine Entscheidung, kein Versäumnis — offene Punkte am gebauten
Werkzeug stehen in der `HANDOVER.md`.

---

## Das neue Windows-11-Hauptmenü

Windows 11 zeigt beim Rechtsklick ein eigenes, kurzes Menü; das klassische
liegt darunter unter „Weitere Optionen anzeigen". Dieses Werkzeug arbeitet am
klassischen Menü, das Windows 11 vollständig weiterführt. Ins neue Menü kommt
man **nicht** über die Registry.

**Was es bräuchte:**

- Eine zweite Crate im selben Workspace, `crate-type = ["cdylib"]`, die
  `IExplorerCommand` über `windows-rs` und `#[implement]` bereitstellt. Der
  Workspace ist deshalb von Anfang an ein Workspace: die zweite Crate ist ein
  Einzeiler in der `Cargo.toml`.
- Registrierung ausschließlich über ein **Sparse MSIX Package** mit der
  `desktop4:FileExplorerContextMenus`-Erweiterung. Registry-Einträge greifen
  dort nicht.
- Das Paket muss **signiert** sein. Für den Eigengebrauch: selbstsigniertes
  Zertifikat nach `LocalMachine\Root` und `LocalMachine\TrustedPeople`, dann
  `Add-AppxPackage -ExternalLocation`.

**Was dafür schon steht:** jeder selbst angelegte Eintrag wird nicht nur in die
Registry geschrieben, sondern parallel nach
`%LOCALAPPDATA%\ctxmenu\entries.json`, mit demselben Datenmodell. Der Handler
soll diese Datei lesen und seine Einträge daraus bauen — dann muss die DLL
genau einmal gebaut und signiert werden, und die Oberfläche schreibt weiterhin
nur JSON. Gelöschte Einträge verschwinden seit dem 2026-08-14 auch aus dieser
Datei, damit der Handler nichts wiederbelebt, was jemand entfernt hat.

**Warum nicht jetzt:** eine signierte DLL im Explorer-Prozess ist eine andere
Risikoklasse als ein Programm, das Registry-Schlüssel schreibt. Ein Fehler dort
trifft jeden Rechtsklick auf dem System, und zwar ohne Rückfallweg.

---

## Drag & Drop für die Reihenfolge

**Verworfen, nicht verschoben.** Die Registry sortiert Unterschlüssel
unbedingt alphabetisch; eine frei gewählte Reihenfolge ließe sich nur über
Schlüsselnamen wie `01_`, `02_` erzwingen — sichtbar im Menü, sobald ein
Eintrag keinen Anzeigenamen hat, und zerstört bei jedem Programm-Update, das
seinen Schlüssel neu anlegt. Was Windows wirklich hergibt, ist `Position` mit
`Top` und `Bottom`, beides nachgemessen. Das Experiment und sein Ergebnis
stehen in der `HANDOVER.md`.

---

## CommandStore lesend anzeigen

`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell`
enthält Windows-eigene Verben, meist im Besitz von TrustedInstaller. Sie zu
zeigen wäre Auskunft; sie zu ändern ist nicht vorgesehen. `paths::COMMAND_STORE`
steht bereit, `RegTarget::parse` lehnt den Pfad ausdrücklich ab (Test
`paths_outside_the_classes_roots_are_refused`), der Scanner fasst ihn nicht an.

Daran hängt die zweite Form von Untermenüs: ein `SubCommands`-Wert, der
semikolongetrennt Verben aus dem CommandStore aufzählt. Gezeigt werden heute
nur Untermenüs aus echten Unterschlüsseln — auf dieser Maschine sind das alle
fünf vorhandenen.
