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
Datei, damit der Handler nichts wiederbelebt, was jemand entfernt hat — seit
dem 2026-08-15 auch dann, wenn das Löschen über die Kommandozeile lief.

Seit dem 2026-08-15 steht in derselben Datei auch, was ein Eintrag an
**Untereinträgen** hat (Feld `children`, bei älteren Einträgen schlicht
abwesend). Der Handler kann daraus ein kaskadierendes Menü bauen, ohne das
Format noch einmal anzufassen.

**Das ist die einzige offene Verwendung von `entries.json`** und der Grund,
warum die Datei sonst wenig tut: gelesen wird sie heute vom Editor-Dialog
(„bereits angelegt"), von `ctxmenu created` und intern beim Hinzufügen und
Entfernen.

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

**Eine Ausnahme gibt es seit dem 2026-08-15:** innerhalb eines Untermenüs, das
dieses Werkzeug selbst anlegt, ordnen zwei Pfeilknöpfe die Untereinträge — und
dort ist das Zahlenpräfix genau richtig, weil alle Kinder aus derselben Hand
stammen und keine fremde Installation sie neu schreibt. Für fremde Einträge
bleibt es beim Nein. Die Reihenfolge in der *Tabelle* ist davon unberührt: die
ist seit dem 2026-08-14 über die Spaltenköpfe sortierbar.

---

## Einsprachige Fehlermeldungen bis in den Kern

**Die Formulare sind fertig, der Rest bleibt liegen.** Seit dem 2026-08-15
liefern `create::check` und `Favourite::problems` eine *Ursache* statt eines
Satzes; die Oberfläche formuliert in ihrer Sprache, die Konsole nimmt
`bilingual()`, und der Test `nothing_the_window_says_is_said_twice` besteht auf
beidem.

Was bleibt, sind die Fehler, die erst beim *Schreiben* auftreten: „Schlüssel
existiert bereits / key already exists", Backup- und Registry-Fehler. Sie
reisen als `anyhow`-Kette und landen wörtlich in `Dialog::Error`.

**Warum nicht:** dafür müsste jedes `bail!` und jedes `context` im Kern durch
einen Ursachentyp ersetzt werden — rund sechzig Stellen, quer durch `backup`,
`write`, `plan`, `paths` und `webtool`. Der Ertrag wären Meldungen, die im
Alltag kaum jemand zu sehen bekommt, weil ihnen ein tatsächlicher Fehlschlag
vorausgehen muss; und die Konsole braucht dort weiterhin beide Sprachen, also
verschwände der doppelte Text nicht, er zöge nur um. Der Weg ist vorgezeichnet,
falls es doch jemand will: `Fault` in `create.rs` ist die Vorlage.

---

## Symbole für DLLs, die keine haben

Manche Shell-Erweiterungs-DLLs tragen schlicht keine Symbolressource. Dagegen
hilft kein weiteres Nachschlagen — es gibt nichts zu finden, und die Tabelle
zeigt dort das Ersatzbild.

Der zweite Grund für fehlende Symbole ist entfallen: bloße Programmnamen
werden seit dem 2026-08-14 über `PATH` aufgelöst. Übrig bleibt genau ein
unaufgelöster Name, `mscoree.dll`, und der kommt nicht aus der Kommandozeile,
sondern aus dem `InprocServer32` einer .NET-Shell-Erweiterung.

**Warum nicht:** dort ebenfalls über `PATH` zu suchen wäre möglich, würde aber
32- und 64-Bit-Handler auf dieselbe Datei in `System32` zeigen lassen. Eine
falsche Zuordnung für ein generisches Symbol ist den Tausch nicht wert.

Die Zahl der Einträge ohne Symbol ist seit dem Verbvorrat nicht neu erhoben —
die alten 37 stammen aus einem Lauf mit 700 statt 927 Einträgen. Wer sie
braucht: die Statuszeile zeigt sie im laufenden Fenster als
`Icons geladen/wartend/gescheitert`.

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
