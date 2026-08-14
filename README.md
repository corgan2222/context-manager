# ctxmenu — Kontextmenü-Manager

Ein Werkzeug für das Windows-Rechtsklickmenü: es zeigt, was darin steckt, wo es
in der Registry sitzt und zu welchem Programm es gehört — und lässt Einträge
ausblenden, nur mit Umschalttaste zeigen, sortieren, löschen und neu anlegen.
**Vor jeder Änderung wird gesichert**, und zwar nicht als Vorsatz, sondern weil
die Löschfunktion ohne Sicherungsnachweis gar nicht aufrufbar ist.

Windows 10 und 11, 64 Bit. Eine einzelne `.exe` ohne Installation und ohne
Laufzeitbibliothek.

*A manager for the Windows context menu. German and English interface,
switchable at runtime; this README is German only.*

---

## Was es kann

- **Alles sehen.** Die sieben Basis-Kategorien (Dateien, Ordner,
  Ordner-Hintergrund, Desktop, Laufwerke …) über drei Registry-Bereiche:
  `HKCU`, `HKLM` und die 32-Bit-Sicht `WOW6432Node`. Statische Verben und
  COM-Handler getrennt ausgewiesen. Dazu Windows' eigener **Verbvorrat**
  (`CommandStore`) — 229 Verben auf dieser Maschine, die in keinem Menü
  stehen, bis ein anderer Eintrag sie in seiner `SubCommands`-Liste nennt.
  Nur lesbar, mit Schloss gekennzeichnet.
- **Dateitypen auflösen.** Für eine Erweiterung wie `.jpg` die vollständige
  Kette aus Benutzerwahl, ProgID, `PerceivedType` und `SystemFileAssociations`
  — also das, was der Rechtsklick tatsächlich anzeigt, nicht das, was an einer
  Stelle eingetragen ist.
- **Nach Programm gruppieren.** Ein Programm, das sich in zwanzig Dateitypen
  einträgt, erscheint als **eine** Gruppe mit allen Vorkommen. Der Name kommt
  aus der Versionsressource der `.exe`, nicht aus dem Schlüsselnamen.
- **Ändern, in vier Stufen von sanft nach hart:** ausblenden
  (`LegacyDisable`), nur mit Umschalttaste zeigen (`Extended`), Position auf
  oben oder unten setzen, COM-Handler systemweit blockieren, löschen.
- **Eigene Einträge anlegen** mit Anzeigename, Befehl, Symbol, Position und
  Umschalt-Sichtbarkeit. Immer in `HKCU`, also ohne Administratorrechte und
  ohne Risiko für andere Konten.
- **Favoriten: der Werkzeugkasten.** Ein Programm oder ein Webtool einmal
  eintragen, und es bleibt. Von dort aus landet es mit einem Klick in jeder
  Kategorie oder bei einem bestimmten Dateityp. Aus dem Reiter *Programme*
  wandert ein Programm, das ohnehin ständig auftaucht, mit einem Klick in die
  Liste.
- **Webtools als Favorit.** Ein Favorit muss keine `.exe` sein, eine Adresse
  genügt. Weil eine Webseite keine lokale Datei lesen darf, wird sie
  *geschickt* — dazu unten mehr.
- **Sichern und zurückholen.** Jede Aktion legt vorher ein Backup an, eine
  Gruppenaktion genau eines für die ganze Gruppe. Der Reiter *Sicherungen*
  zeigt den Verlauf und spielt zurück.
- **Untermenüs** werden mit ihren Kindern angezeigt, eingerückt unter dem
  Eintrag, in dem sie hängen.
- Deutsch und Englisch, hell und dunkel oder „System folgen" — beides ohne
  Neustart, die Titelleiste zieht mit.

## Was es bewusst nicht kann

- **Den Text eines COM-Handlers ändern.** Der entsteht zur Laufzeit in
  `IContextMenu::QueryContextMenu` und steht nirgends in der Registry. Gezeigt
  werden Schlüsselname, Klarname der CLSID und die DLL dahinter.
- **Das neue Windows-11-Hauptmenü umbauen.** Das Werkzeug arbeitet am
  klassischen Menü („Weitere Optionen anzeigen"), das Windows 11 weiterhin
  vollständig führt.
- **Die Reihenfolge frei bestimmen.** Windows sortiert die Unterschlüssel
  alphabetisch und kennt nur die groben Blöcke `Position=Top` und
  `Position=Bottom`. Beides ist nachgemessen; mehr gibt das System nicht her.

---

## Loslegen

Es gibt keinen Installer. Die `.exe` starten reicht.

```
ctxmenu.exe
```

Ohne Argumente öffnet sich das Fenster. Ganz ohne Administratorrechte —
angefragt werden sie erst, wenn eine Änderung sie wirklich braucht, und dann
nur für diesen einen Schritt.

### Die vier Reiter

| Reiter | Wofür |
|---|---|
| **Kategorien** | Der Einstieg: was bei Rechtsklick auf Ordner, Dateien, Desktop erscheint |
| **Dateitypen** | Eine Erweiterung wählen und die vollständige Auflösungskette sehen |
| **Programme** | Nach Programm gruppiert — der schnellste Weg, ein Programm ganz aus dem Menü zu nehmen |
| **Favoriten** | Der eigene Werkzeugkasten: einmal eintragen, immer da |
| **Sicherungen** | Verlauf aller Sicherungen, mit Knopf zum Zurückspielen |

Das Suchfeld greift auf jedem Reiter und durchsucht Anzeigename, Befehl und
Registry-Pfad; auch dann, wenn links noch nichts ausgewählt ist.

**In der Liste:** Pfeiltasten bewegen die Auswahl, Pos1 und Ende springen an
Anfang und Ende, mit gedrückter Umschalttaste wächst die Auswahl. Ein Klick auf
eine Spaltenüberschrift sortiert danach, ein zweiter dreht die Richtung um. Die
Spalte **Erscheint bei** sagt in Worten, wo ein Eintrag auftaucht — „Alle
Dateien" statt `*`, „.zip" statt eines Pfads mit `SystemFileAssociations` in der
Mitte; der echte Registry-Pfad steht im Tooltip. Jeder Knopf hat einen, der
erklärt, was er anfasst und ob es sich rückgängig machen lässt.

### Eine typische Runde

1. Reiter **Programme**, das Programm anklicken, das stört.
2. Rechts prüfen, was daran hängt — Pfad, Befehl, Bereich.
3. **Ausblenden** statt Löschen. Das ist umkehrbar und reicht fast immer.
4. Wenn Windows nach Administratorrechten fragt: das sind die Einträge unter
   `HKLM`, also die für alle Konten. Wer ablehnt, behält die Änderungen an den
   eigenen Einträgen; die anderen bleiben, wie sie waren.

---

## Favoriten und Webtools

Der Reiter **Favoriten** ist eine Liste, die bleibt. Was einmal darin steht,
lässt sich jederzeit an einer weiteren Stelle im Kontextmenü eintragen, ohne
es noch einmal einzurichten. „Ins Kontextmenü" fragt nur noch nach dem Wo:
eine der Basis-Kategorien, eine Dateiendung (`.png`), oder eine ganze Art von
Datei (`image` erfasst jedes Bildformat, das Windows kennt).

Ein Favorit muss kein Programm sein. Wenn das Werkzeug im Browser lebt, gibt
es ein Problem, das keine Registry löst: **eine Webseite darf keine lokale
Datei lesen.** Eine Adresse wie `https://tool.example/?f=C:\bild.png` öffnet
zwar die Seite, aber die Datei kommt dort nie an — kein Browser erlaubt das,
und das ist auch gut so. Die Datei muss also verschickt werden, und dafür
braucht es einen Absender. Das ist dieses Programm: der Menüeintrag ruft
`ctxmenu --favourite <kennung> "%1"` auf und macht dann, je nach Betriebsart,
eines von drei Dingen.

**Zwischenablage** — die Datei landet in der Zwischenablage, die Seite geht
auf, ein Strg+V im Browser genügt. Das ist der Weg für alles, was gar keine
Schnittstelle anbietet: Squoosh, die TinyPNG-Seite, remove.bg. Kein Schlüssel,
kein Endpunkt, funktioniert auch bei Werkzeugen, die nie damit gerechnet
haben. Bei einer PNG liegt zusätzlich das Bild selbst in der Ablage, damit
auch Seiten zufrieden sind, die ein Bild statt einer Datei erwarten.

**Hochladen** — für Werkzeuge mit echtem Endpunkt. Die Datei geht per
`multipart/form-data` (Feldname einstellbar) oder pur als Rumpf hinaus,
Kopfzeilen für einen Schlüssel lassen sich mitgeben. Was zurückkommt, wird
wahlweise neben der Originaldatei gespeichert (`bild.png` → `bild.min.png`;
das Original wird **nie** überschrieben), im Browser geöffnet, oder nur
gemeldet. Die Ergebnisadresse darf im `Location`-Kopf oder in einem JSON-Feld
wie `output.url` stehen.

**Adresse öffnen** — baut die Adresse aus Platzhaltern und öffnet sie, ohne
etwas zu übertragen. `{name}`, `{stem}`, `{ext}`, `{path}`, `{dir}` und
`{fileurl}`, alle korrekt kodiert. Für Suche, Wiki, Ticketformular.

**Vor dem ersten Hochladen wird gefragt.** Einmal je Werkzeug, mit Angabe von
Ziel und Dateigröße; die Antwort wird gemerkt. Unverschlüsseltes `http://`
lehnt das Programm ab, solange es nicht für diesen Favoriten ausdrücklich
erlaubt wurde — eine Datei im Klartext durchs Netz zu schicken soll eine
Entscheidung sein, keine Voreinstellung. Für die Übertragung ist WinHTTP
zuständig, also der Client von Windows selbst: mit dem Systemzertifikatspeicher
und den Proxy-Einstellungen, die ohnehin gelten.

---

## Über die Kommandozeile

Dieselbe Anwendung ist auch ein Diagnosewerkzeug. Ausgaben landen in der
Konsole, aus der sie gestartet wurde.

```
ctxmenu scan --category directory        Einträge einer Kategorie
ctxmenu scan --all-types --json          Vollscan inklusive Dateitypen, als JSON
ctxmenu filetype .jpg                    Auflösungskette einer Erweiterung
ctxmenu programs                         nach Programm gruppiert
ctxmenu hide "<schlüssel>" --yes         ausblenden, mit Sicherung
ctxmenu delete "<schlüssel>" --yes       löschen, mit Sicherung
ctxmenu backups                          Sicherungen auflisten
ctxmenu restore "<verzeichnis>"          Sicherung zurückspielen
ctxmenu create --category directory --name "Mit Editor öffnen"
               --command "\"C:\Windows\notepad.exe\" \"%1\""
ctxmenu created                          eigene Einträge auflisten
ctxmenu favourites                       Favoriten auflisten
ctxmenu favourite add --name "PNG verkleinern"
        --url https://squoosh.app --mode clipboard
ctxmenu favourite place <kennung> --ext .png
ctxmenu favourite run <kennung> <datei>  ausführen wie ein Klick
ctxmenu --help                           die vollständige Liste
```

Ein Hinweis zum Anlegen: In den Hintergrund-Kategorien (Ordner-Hintergrund,
Desktop) bleibt `%1` **leer**. Dort gehört `%V` hin. Das Werkzeug warnt davor,
denn ein Eintrag, der nichts tut, sieht aus wie ein Eintrag, der geht.

---

## Wo die Sicherungen liegen

```
%LOCALAPPDATA%\ctxmenu\backups\<zeitstempel>_<aktion>\
    manifest.json      was gesichert wurde, wann, und was fehlte
    01_….reg           eine Datei je Schlüssel, von reg.exe geschrieben
%LOCALAPPDATA%\ctxmenu\entries.json     selbst angelegte Einträge
%LOCALAPPDATA%\ctxmenu\favourites.json  der Werkzeugkasten
%LOCALAPPDATA%\ctxmenu\settings.json    Sprache und Darstellung
```

Die `.reg`-Dateien sind gewöhnliche Registrierungsdateien: sie lassen sich auch
ohne dieses Werkzeug per Doppelklick zurückspielen. Eine Grenze hat das —
`reg import` fügt hinzu und überschreibt, es **entfernt nichts**. Nach einem
Löschen stellt es exakt den alten Zustand her; über einen inzwischen
veränderten Schlüssel gespielt, bleiben dessen neue Werte stehen.

---

## Selbst bauen

Vorausgesetzt sind Rust 1.95 und die Visual-Studio-Build-Tools mit
C++-Werkzeugkette.

```powershell
cargo build --release
cargo test
```

Das Ergebnis liegt unter `target\x86_64-pc-windows-msvc\release\ctxmenu.exe` —
nicht unter `target\release\`, weil `.cargo\config.toml` das Ziel ausdrücklich
nennt. Das ist Absicht: nur so gilt die statisch gebundene C-Laufzeit für die
Anwendung und nicht auch für die Makro-Bibliotheken des Übersetzers. Die
fertige Datei braucht deshalb kein „Visual C++ Redistributable" — nachgeprüft
auf einem frisch installierten Windows 10 ohne jede Zusatzsoftware.

Weiterführend: `HANDOVER.md` hält den Entwicklungsstand fest, die Messwerte und
die Stellen, an denen Windows sich anders verhält als dokumentiert.
