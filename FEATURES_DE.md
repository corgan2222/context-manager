# ctxmenu

*[English version](FEATURES.md)*

Ein Verwalter für das Windows-Rechtsklickmenü. Er zeigt, was darin steckt und wo
es in der Registry sitzt, nimmt Einträge heraus, die stören, und setzt eigene
hinein — bis hin zu zweihundert Menüpunkten aus der Selbstbeschreibung eines
Webdienstes.

Windows 10 und 11, 64 Bit. Eine `.exe` von 6,5 MB, kein Installer, keine
Laufzeitbibliothek, kein Dienst im Hintergrund.

---

## Lesen

**Sieben Kategorien, drei Registry-Bereiche.** Dateien, Ordner,
Ordner-Hintergrund, Desktop-Hintergrund, Laufwerke, Dateisystemobjekte und der
Shell-Namensraum — gelesen aus `HKCU`, `HKLM` und der 32-Bit-Sicht
`WOW6432Node`. Auf einer gewachsenen Maschine sind das rund 930 Einträge.

**Statische Verben und COM-Handler getrennt.** Ein Verb ist ein Schlüssel mit
einem Befehl; ein COM-Handler ist eine CLSID, hinter der eine DLL steckt. Das
Programm zeigt für den Handler den Klarnamen der CLSID und die DLL dahinter.

**Windows' eigener Verbvorrat.** Der `CommandStore` enthält auf dieser Maschine
229 Verben, die in keinem Menü auftauchen, bis ein anderer Eintrag sie in seiner
`SubCommands`-Liste nennt. Sie stehen mit Schloss in der Liste, nur lesbar.

**Die Auflösungskette eines Dateityps.** Für `.jpg` sind es sieben Ebenen:
Benutzerwahl, ProgID, `PerceivedType`, `SystemFileAssociations` und die
allgemeinen Einträge. Das Programm zeigt, was der Rechtsklick wirklich anzeigt,
nicht was an einer Stelle eingetragen ist. Für `.jpg` sind das 58 Einträge, 39
davon gelten für jede Datei.

**Nach Programm gruppiert.** Ein Programm, das sich in zwanzig Dateitypen
einträgt, erscheint als eine Gruppe mit allen Vorkommen und seinem Symbol davor.
Der Name kommt aus der Versionsressource der `.exe`, nicht aus dem
Schlüsselnamen. Zeigt ein Eintrag auf ein Programm, das es nicht mehr gibt,
steht die Zeile rot — das passiert nach Updates von Store-Apps, deren Ordner die
Versionsnummer im Namen trägt.

**Suchen.** Ein Feld über allen Reitern, es durchsucht Anzeigename, Befehl und
Registry-Pfad. Auch dann, wenn links nichts ausgewählt ist.

---

## Ändern

Vier Stufen, von sanft nach hart:

| Stufe | Was passiert | Umkehrbar |
|---|---|---|
| **Ausblenden** | `LegacyDisable` an einem Ort | ja |
| **Nur mit Umschalttaste** | `Extended`, der Eintrag erscheint bei ⇧+Rechtsklick | ja |
| **Position** | `Position=Top` oder `Bottom` | ja |
| **Systemweit sperren** | Die CLSID kommt in die Sperrliste, für alle Konten | ja |
| **Löschen** | Der Schlüssel verschwindet | nur aus der Sicherung |

**Vor jeder Änderung wird gesichert.** Das ist keine Absichtserklärung: `delete_tree`
verlangt ein Token, und ein Token entsteht nur als Rückgabewert einer geglückten
Sicherung. Ohne Sicherung ist die Löschfunktion nicht aufrufbar.

**Eine Sicherung je Gruppenaktion**, nicht eine je Eintrag. Zwanzig Einträge auf
einmal ausblenden legt ein Verzeichnis an, nicht zwanzig.

**Erhöhte Rechte nur, wenn nötig.** Einträge unter `HKLM` verlangen sie; das
Programm fragt für genau diesen Schritt und startet sich dafür einmal neu. Wer
ablehnt, behält die Änderungen an den eigenen Einträgen.

**Die Aktionsleiste besteht aus vier Schaltern**, nicht aus neun Knöpfen. *Im
Menü* (sichtbar ↔ versteckt), *Umschalttaste* (immer ↔ nur mit ⇧), *Systemweit*
(frei ↔ gesperrt) und *Position*. Hervorgehoben ist, wo die Auswahl steht; ein
Klick auf die andere Seite führt dorthin. Bei gemischter Auswahl leuchtet nichts,
und der Tooltip nennt die Zahlen. Was gerade nicht geht, ist grau und sagt warum
— etwa, dass keiner der ausgewählten Einträge eine CLSID hat, die sich sperren
ließe.

---

## Anlegen

**Eigene Einträge** mit Anzeigename, Befehl, Symbol, Position und
Umschalt-Sichtbarkeit. Immer in `HKCU`, also ohne Administratorrechte und ohne
Wirkung auf andere Konten.

**Untermenüs.** Statt eines Befehls bekommt der Eintrag eine Liste von
Untereinträgen. Die Reihenfolge im Formular ist die im Menü — Windows sortiert
Registry-Schlüssel alphabetisch, deshalb nummeriert das Programm sie beim
Schreiben durch.

**Ziehen und Ablegen.** Eine `.exe` ins Fenster gezogen legt einen Eintrag mit
ihr an; über welcher Kategorie sie fällt, entscheidet, wo er landet. Im Editor
nehmen auch die Felder für Befehl und Symbol eine abgelegte Datei entgegen.

**Eine Prüfung vor dem Schreiben.** In den Hintergrund-Kategorien bleibt `%1`
leer — dort gehört `%V` hin. Das Programm sagt das vorher, denn ein Eintrag, der
nichts tut, sieht aus wie einer, der geht.

---

## Favoriten

Eine Liste, die bleibt. Was einmal darin steht, lässt sich mit einem Klick an
einer weiteren Stelle im Kontextmenü eintragen: in einer Basis-Kategorie, bei
einer Dateiendung (`.png`), oder bei einer ganzen Art von Datei (`image` erfasst
jedes Bildformat, das Windows kennt).

Ein Favorit muss kein Programm sein. Wenn das Werkzeug im Browser lebt, steht
eine Hürde im Weg, die keine Registry beseitigt: **eine Webseite darf keine
lokale Datei lesen.** `https://tool.example/?f=C:\bild.png` öffnet die Seite, die
Datei kommt nie an. Sie muss also verschickt werden, und dafür braucht es einen
Absender. Der Menüeintrag ruft `ctxmenu --favourite <kennung> "%1"` und macht
dann eines von drei Dingen:

**Zwischenablage.** Die Datei landet in der Ablage, die Seite geht auf, Strg+V im
Browser genügt. Der Weg für alles ohne Schnittstelle: Squoosh, remove.bg. Bei
einer PNG liegt zusätzlich das Bild selbst in der Ablage, damit auch Seiten
zufrieden sind, die ein Bild statt einer Datei erwarten.

**Hochladen.** Die Datei geht als `multipart/form-data` oder pur als Rumpf
hinaus, Kopfzeilen für einen Schlüssel lassen sich mitgeben. Was zurückkommt,
wird neben der Originaldatei gespeichert (`bild.png` → `bild.min.png`; das
Original wird nie überschrieben), im Browser geöffnet, oder nur gemeldet. Die
Ergebnisadresse darf im `Location`-Kopf oder in einem JSON-Feld wie `output.url`
stehen.

**Adresse öffnen.** Baut eine Adresse aus `{name}`, `{stem}`, `{ext}`, `{path}`,
`{dir}` und `{fileurl}` und öffnet sie, ohne etwas zu übertragen. Für Suche,
Wiki, Ticketformular.

Vor dem ersten Hochladen fragt das Programm, einmal je Werkzeug, mit Ziel und
Dateigröße. Unverschlüsseltes `http://` lehnt es ab, bis es für diesen Favoriten
ausdrücklich erlaubt wurde. Für die Übertragung ist WinHTTP zuständig, also der
Client von Windows selbst, mit dem Systemzertifikatspeicher und den geltenden
Proxy-Einstellungen.

---

## Dienste

Einen Favoriten von Hand einzurichten heißt, sechs Felder auszufüllen. Bei einem
Dienst mit zweihundert Werkzeugen macht das niemand.

Der Reiter **Dienste** nimmt deshalb die Adresse, die ohnehin im Browser offen
ist — die API-Dokumentation, mitsamt Sprungmarke. Das Programm schneidet die
Marke ab und sucht das maschinenlesbare Dokument dahinter: die Seite selbst,
dann `openapi.json`, `swagger.json`, `/v3/api-docs` und die übrigen üblichen
Orte. Der Statuscode entscheidet dabei nichts, denn eine Dokumentationsseite
antwortet ebenso mit 200 wie das Dokument. Ob sich die Antwort als JSON lesen
lässt, ist das Kriterium.

**Welche Endpunkte in Frage kommen:** die, die eine Datei als
`multipart/form-data` annehmen. An einem Testdienst mit 351 Pfaden sind das 232.

**Wie sie zusammengehören.** Nicht nach dem OpenAPI-`tag` — der lautet bei vielen
Diensten für fast alles gleich. Stattdessen treten alle möglichen Gliederungen
gegeneinander an, der Tag und jede Stelle des Pfades, gemessen an vier Größen:
wie viel des Dienstes in brauchbaren Gruppen landet, wie gleichmäßig, wie nah die
Gruppenzahl an der Wurzel aus der Gesamtzahl liegt, und ob die Werkzeugnamen das
Gruppenwort wiederholen. Am Testdienst gewinnt so *Image, Video, PDF, Audio,
Files* gegen eine Schublade „Tools" mit 225 Einträgen darin, mit Faktor 17.

**Was ein Werkzeug außer der Datei annimmt.** Liefert die Beschreibung ein
Schema, entsteht ein Formular mit getippten Feldern: Zahl mit dem erlaubten
Bereich im leeren Feld, Ankreuzfeld, Auswahlliste. Liefert sie keins, sondern nur
Prosa, wird auch die gelesen, solange sie ihre Felder auflistet:

```
JSON string with options:
- `left` (number, required) - Left offset in pixels (min 0)
- `unit` (string, optional) - One of: px, percent
```

Daraus wird dasselbe Formular. Am Testdienst ergeben 113 von 227
Options-Beschreibungen ein Formular mit zusammen 431 Feldern. Wo die Prosa nicht
eindeutig ist, bleibt es beim Textfeld: ein falsch erkanntes Feld schickt Unsinn
an einen echten Dienst, ein übersehenes kostet ein Ankreuzfeld.

**Was nicht funktionieren würde, steht nicht in der Liste.** Endpunkte, die nur
mit einer Auftragsnummer antworten und im Hintergrund weiterarbeiten, ergäben
einen Eintrag, der Erfolg meldet und nichts speichert. Am Testdienst sind das 52
von 232. Ihre Zahl steht trotzdem da, mit einem Knopf, der sie einblendet.

Angekreuzt wird einzeln oder kategorienweise, angelegt auf einen Schlag. Jedes
Werkzeug hat außerdem einen Verweis auf seine Stelle in der Dokumentation des
Dienstes.

Adresse und Schlüssel bleiben in `%LOCALAPPDATA%\ctxmenu\services.json` und gehen
nirgendwohin.

---

## Sichern und zurückholen

Jede Aktion legt vorher eine Sicherung an. Der Reiter **Sicherungen** zeigt den
Verlauf, sagt zu jeder, was darin steckt, und spielt sie zurück.

Ein Knopf **Alles sichern** nimmt jeden Ort mit, den dieses Programm überhaupt
anfasst: auf dieser Maschine 26 von 46 Schlüsseln, 1,2 MB, unter einer Sekunde.
Die übrigen 20 gibt es hier nicht, 15 davon in der leeren 32-Bit-Ansicht.

```
%LOCALAPPDATA%\ctxmenu\backups\<zeitstempel>_<aktion>\
    manifest.json      was gesichert wurde, wann, und was fehlte
    01_….reg           eine Datei je Schlüssel, von reg.exe geschrieben
```

Die `.reg`-Dateien sind gewöhnliche Registrierungsdateien und lassen sich per
Doppelklick zurückspielen, auch ohne dieses Programm. Eine Grenze hat das:
`reg import` fügt hinzu und überschreibt, es entfernt nichts. Nach einem Löschen
stellt es den alten Zustand her; über einen inzwischen veränderten Schlüssel
gespielt, bleiben dessen neue Werte stehen.

---

## Bedienung

- **Tastatur in der Liste:** Pfeiltasten, Pos1 und Ende, Umschalt für einen
  Bereich, Strg+A für alles.
- **Rechtsklick** bietet überall die Aktionen an, die für das Angeklickte etwas
  ändern würden. Bei Mehrfachauswahl fallen die weg, die nur für einen Eintrag
  Sinn ergeben. Im leeren Bereich steht *Neu*.
- **Sortieren** per Klick auf eine Spaltenüberschrift, ein zweiter dreht um.
- **Die Spalte „Erscheint bei"** sagt in Worten, wo ein Eintrag auftaucht: „Alle
  Dateien" statt `*`, „.zip" statt eines Pfads mit `SystemFileAssociations` in
  der Mitte. Der echte Pfad steht im Tooltip.
- **Explorer neu starten** als Knopf in der oberen Leiste. Windows liest die
  Kontextmenü-Schlüssel beim Start des Explorers.
- **Deutsch und Englisch**, hell und dunkel oder „System folgen". Beides ohne
  Neustart, die Titelleiste zieht mit.
- **Ein Fehlerprotokoll** unter `%LOCALAPPDATA%\ctxmenu\ctxmenu.log`, verlinkt im
  Über-Fenster. Es enthält jeden gezeigten Fehler und jeden Absturz.

---

## Kommandozeile

Dieselbe `.exe` ist auch ein Diagnosewerkzeug. Ausgaben landen in der Konsole,
aus der sie gestartet wurde.

```
ctxmenu scan --category directory        Einträge einer Kategorie
ctxmenu scan --all-types --json          Vollscan inklusive Dateitypen, als JSON
ctxmenu scan --every-type                jede registrierte Endung statt der Auswahl
ctxmenu filetype .jpg                    Auflösungskette einer Erweiterung
ctxmenu programs                         nach Programm gruppiert
ctxmenu hide "<schlüssel>" --yes         ausblenden, mit Sicherung
ctxmenu delete "<schlüssel>" --yes       löschen, mit Sicherung
ctxmenu backups                          Sicherungen auflisten
ctxmenu backup-all                       alles sichern, was das Werkzeug anfasst
ctxmenu restore "<verzeichnis>"          Sicherung zurückspielen
ctxmenu create --category directory --name "Mit Editor öffnen"
               --command "\"C:\Windows\notepad.exe\" \"%1\""
ctxmenu created                          eigene Einträge auflisten
ctxmenu favourites                       Favoriten auflisten
ctxmenu favourite run <kennung> <datei>  ausführen wie ein Klick
ctxmenu --tab dienste                    Fenster auf einem Reiter öffnen
ctxmenu --version
ctxmenu --help
```

---

## Was es nicht kann

**Den Text eines COM-Handlers ändern.** Der entsteht zur Laufzeit in
`IContextMenu::QueryContextMenu` und steht nirgends in der Registry. Gezeigt
werden Schlüsselname, Klarname der CLSID und die DLL dahinter.

**Das Windows-11-Hauptmenü umbauen.** Das Programm arbeitet am klassischen Menü
(„Weitere Optionen anzeigen"), das Windows 11 weiterhin vollständig führt.

**Die Reihenfolge frei bestimmen.** Windows sortiert die Unterschlüssel
alphabetisch und kennt nur die groben Blöcke `Position=Top` und
`Position=Bottom`. Nachgemessen; mehr gibt das System nicht her.

**Gescannte Einträge bearbeiten.** Das Formular zeigt alles, was in der Registry
steht, schreibt aber noch nichts zurück. Eigene Einträge sind davon nicht
betroffen.

---

## Bauen

Rust 1.95, Visual-Studio-Build-Tools mit C++-Werkzeugkette.

```powershell
cargo build --release
cargo test
```

Das Ergebnis liegt unter `target\x86_64-pc-windows-msvc\release\ctxmenu.exe`,
nicht unter `target\release\`: `.cargo\config.toml` nennt das Ziel ausdrücklich,
damit die statisch gebundene C-Laufzeit für die Anwendung gilt und nicht auch für
die Makro-Bibliotheken des Übersetzers. Die fertige Datei braucht deshalb kein
„Visual C++ Redistributable" — nachgeprüft auf einem frisch installierten
Windows 10 ohne Zusatzsoftware.

336 Tests, `cargo clippy -- -D warnings` sauber.
