# ctxmenu — Kontextmenü-Manager

Ein Werkzeug für das Windows-Rechtsklickmenü: es zeigt, was darin steckt, wo es
in der Registry sitzt und zu welchem Programm es gehört — und lässt Einträge
ausblenden, nur mit Umschalttaste zeigen, sortieren, löschen und neu anlegen.
**Vor jeder Änderung wird gesichert**, und zwar nicht als Vorsatz, sondern weil
die Löschfunktion ohne Sicherungsnachweis gar nicht aufrufbar ist.

Und es geht in die andere Richtung: eigene Einträge, Untermenüs, ein
Werkzeugkasten aus Programmen und Web-Diensten — bis hin zu **zweihundert
Menüeinträgen aus einer einzigen Adresse**, wenn eine Webanwendung sich selbst
über OpenAPI beschreibt.

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
- **Eigene Dateiendungen und der Vollscan.** Der Reiter *Dateitypen* zeigt
  eine kuratierte Auswahl von 98 Typen; ein Feld darüber nimmt jede weitere
  Endung auf, die dann gespeichert bleibt. Wer alles sehen will, drückt
  *Alle installierten* — auf einem gewachsenen Rechner sind das rund 1700
  Typen statt 98, entsprechend länger dauert das Einlesen.
- **Nach Programm gruppieren.** Ein Programm, das sich in zwanzig Dateitypen
  einträgt, erscheint als **eine** Gruppe mit allen Vorkommen — mit seinem
  Symbol davor. Der Name kommt aus der Versionsressource der `.exe`, nicht aus
  dem Schlüsselnamen. **Zeigt ein Eintrag auf ein Programm, das es nicht mehr
  gibt, steht die Zeile in Rot**; das passiert vor allem nach Updates von
  Store-Apps, deren Ordner die Versionsnummer im Namen trägt.
- **Ändern, in vier Stufen von sanft nach hart:** ausblenden
  (`LegacyDisable`), nur mit Umschalttaste zeigen (`Extended`), Position auf
  oben oder unten setzen, COM-Handler systemweit blockieren, löschen.
- **Eigene Einträge anlegen** mit Anzeigename, Befehl, Symbol, Position und
  Umschalt-Sichtbarkeit. Immer in `HKCU`, also ohne Administratorrechte und
  ohne Risiko für andere Konten.
- **Auch als Untermenü.** Statt eines Befehls bekommt der Eintrag eine Liste
  von Untereinträgen, die im Menü aufklappt. Die Reihenfolge im Formular ist
  die im Menü — Windows sortiert Registry-Schlüssel alphabetisch, deshalb
  nummeriert das Werkzeug sie beim Schreiben durch.
- **Favoriten: der Werkzeugkasten.** Ein Programm oder ein Webtool einmal
  eintragen, und es bleibt. Von dort aus landet es mit einem Klick in jeder
  Kategorie oder bei einem bestimmten Dateityp. Aus dem Reiter *Programme*
  wandert ein Programm, das ohnehin ständig auftaucht, mit einem Klick in die
  Liste.
- **Webtools als Favorit.** Ein Favorit muss keine `.exe` sein, eine Adresse
  genügt. Weil eine Webseite keine lokale Datei lesen darf, wird sie
  *geschickt* — dazu unten mehr.
- **Dienste: hundert Werkzeuge aus einer Adresse.** Beschreibt eine
  Webanwendung sich selbst über OpenAPI, genügt die Adresse ihrer
  Dokumentationsseite. Das Programm sucht das maschinenlesbare Dokument dahinter,
  liest heraus, welche Endpunkte eine Datei annehmen, gruppiert sie so, wie der
  Dienst sie selbst gruppiert, und macht aus jedem angekreuzten einen Favoriten.
  Nimmt ein Werkzeug Einstellungen an, entsteht ein Formular dafür — auch dann,
  wenn der Dienst seine Optionen nur als Fließtext beschreibt.
- **Ziehen und Ablegen.** Eine `.exe` ins Fenster ziehen legt einen Eintrag mit
  ihr an; über welcher Kategorie sie fällt, entscheidet, wo er landet. Im Editor
  nehmen auch die Felder für Befehl und Symbol eine abgelegte Datei entgegen.
- **Sichern und zurückholen.** Jede Aktion legt vorher ein Backup an, eine
  Gruppenaktion genau eines für die ganze Gruppe. Der Reiter *Sicherungen*
  zeigt den Verlauf und spielt zurück — und hat einen Knopf **Alles sichern**,
  der jeden Ort mitnimmt, den dieses Werkzeug überhaupt anfasst (auf dieser
  Maschine 1,2 MB in unter einer Sekunde).
- **Untermenüs** werden mit ihren Kindern angezeigt, eingerückt unter dem
  Eintrag, in dem sie hängen.
- **Einen Eintrag ansehen:** Doppelklick auf eine Zeile — oder Rechtsklick,
  *Eintrag ansehen* — öffnet das Formular mit allem, was wirklich in der
  Registry steht. Ändern lässt sich dort noch nichts.
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

### Die sechs Reiter

| Reiter | Wofür |
|---|---|
| **Kategorien** | Der Einstieg: was bei Rechtsklick auf Ordner, Dateien, Desktop erscheint |
| **Dateitypen** | Eine Erweiterung wählen und die vollständige Auflösungskette sehen |
| **Programme** | Nach Programm gruppiert — der schnellste Weg, ein Programm ganz aus dem Menü zu nehmen |
| **Favoriten** | Der eigene Werkzeugkasten: einmal eintragen, immer da |
| **Dienste** | Werkzeuge aus der Selbstbeschreibung einer Webanwendung übernehmen |
| **Sicherungen** | Verlauf aller Sicherungen, mit Knopf zum Zurückspielen |

Das Suchfeld greift auf jedem Reiter und durchsucht Anzeigename, Befehl und
Registry-Pfad; auch dann, wenn links noch nichts ausgewählt ist.

**In der Liste:** Pfeiltasten bewegen die Auswahl, Pos1 und Ende springen an
Anfang und Ende, mit gedrückter Umschalttaste wächst die Auswahl, Strg+A nimmt
alles. Ein Klick auf eine Spaltenüberschrift sortiert danach, ein zweiter dreht
die Richtung um. Die Spalte **Erscheint bei** sagt in Worten, wo ein Eintrag
auftaucht — „Alle Dateien" statt `*`, „.zip" statt eines Pfads mit
`SystemFileAssociations` in der Mitte; der echte Registry-Pfad steht im Tooltip.
Ein **Rechtsklick** bietet überall genau die Aktionen an, die für das Angeklickte
etwas ändern würden — und im leeren Bereich *Neu*.

**Die Aktionsleiste** über der Tabelle ist kein Satz Knöpfe, sondern vier
Schalter: *Im Menü* (sichtbar ↔ versteckt), *Umschalttaste* (immer ↔ nur mit ⇧),
*Systemweit* (frei ↔ gesperrt) und *Position*. Hervorgehoben ist, wo die Auswahl
gerade steht; ein Klick auf die andere Seite führt dorthin. Aus „welchen Knopf
drücke ich?" wird „wohin soll es?". Was gerade nicht geht, ist grau und sagt im
Tooltip warum — etwa, dass keiner der ausgewählten Einträge ein COM-Handler ist
und es deshalb nichts zu sperren gibt.

**Explorer neu starten** sitzt oben in der Leiste. Windows liest die
Kontextmenü-Schlüssel beim Start des Explorers; ein Eintrag, der partout nicht
auftauchen will, braucht das.

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

## Dienste: hundert Werkzeuge, eine Adresse

Einen Favoriten von Hand einzurichten heißt sechs Felder auszufüllen. Bei einem
selbst betriebenen Dienst mit zweihundert Werkzeugen ist das keine Arbeit, die
jemand macht.

Der Reiter **Dienste** nimmt deshalb die Adresse, die man ohnehin im Browser
offen hat — die API-Dokumentation, mitsamt Sprungmarke:

```
http://192.168.x.y:1349/api/docs/#tag/tools
```

Das Programm schneidet die Sprungmarke ab und sucht das maschinenlesbare
Dokument dahinter: die Seite selbst, dann `openapi.json`, `swagger.json` und die
übrigen üblichen Orte, von diesem Pfad aus und von der Wurzel. Der Statuscode
entscheidet dabei nichts — eine Dokumentationsseite antwortet ebenso mit 200 wie
das Dokument. Ob sich die Antwort als JSON lesen lässt, ist das Kriterium.

Aus der Beschreibung wird dann alles gelesen, was sich lesen lässt:

- **Welche Endpunkte überhaupt in Frage kommen** — die, die eine Datei als
  `multipart/form-data` annehmen. Alles andere kann ein Rechtsklick nicht
  bedienen.
- **Wie sie zusammengehören.** Nicht nach dem OpenAPI-`tag`: der lautet bei
  vielen Diensten für alles gleich. Stattdessen treten alle möglichen
  Gliederungen gegeneinander an — der Tag und jede Stelle des Pfades — und die
  gewinnt, die das brauchbarste Menü ergibt. Bei einem Dienst mit 232 Werkzeugen
  kommen so *Image, Video, PDF, Audio, Files* heraus statt einer Schublade
  „Tools" mit 225 Einträgen darin.
- **Was ein Werkzeug außer der Datei annimmt.** Liefert die Beschreibung ein
  Schema, entsteht daraus ein Formular mit getippten Feldern: Zahl mit dem
  erlaubten Bereich, Ankreuzfeld, Auswahlliste. Liefert sie keins, sondern nur
  Prosa — der häufigere Fall —, wird auch die gelesen, solange sie ihre Felder
  auflistet:

  ```
  JSON string with options:
  - `left` (number, required) - Left offset in pixels (min 0)
  - `unit` (string, optional) - One of: px, percent
  ```

  Daraus wird dasselbe Formular. Wo die Prosa nicht eindeutig ist, bleibt es
  beim Textfeld mit der Beschreibung darüber — lieber kein Feld als ein falsches,
  denn ein falsches schickt Unsinn an einen echten Dienst.
- **Was nicht funktionieren würde.** Endpunkte, die nur mit einer
  Auftragsnummer antworten und im Hintergrund weiterarbeiten, stehen nicht in
  der Liste: ein Eintrag daraus würde melden, es habe geklappt, und nichts
  speichern. Ihre Zahl steht trotzdem da, mit einem Knopf, der sie einblendet.

Angekreuzt wird einzeln oder kategorienweise, angelegt auf einen Schlag. Was ein
Dienst über sich selbst sagt, steht danach in jedem Favoriten; **Adresse und
Schlüssel bleiben lokal** in `%LOCALAPPDATA%\ctxmenu\services.json` und gehen
nirgendwohin.

Zwei Felder kann keine Beschreibung liefern, weil sie von der Installation
abhängen: wo in der Antwort die fertige Datei genannt wird, und ob
unverschlüsseltes `http://` erlaubt sein soll. Dafür gibt es Vorlagen — ein
Klick füllt sie aus, die Adresse und der Schlüssel bleiben Ihre.

---

## Über die Kommandozeile

Dieselbe Anwendung ist auch ein Diagnosewerkzeug. Ausgaben landen in der
Konsole, aus der sie gestartet wurde.

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
ctxmenu create --category directory --name "Werkzeuge"
               --sub "Öffnen|\"C:\Windows\notepad.exe\" \"%1\""
               --sub "Anzeigen|cmd /c dir \"%1\" & pause"
                                         Untermenü statt einem Befehl
ctxmenu created                          eigene Einträge auflisten
ctxmenu favourites                       Favoriten auflisten
ctxmenu favourite add --name "PNG verkleinern"
        --url https://squoosh.app --mode clipboard
ctxmenu favourite place <kennung> --ext .png
ctxmenu favourite run <kennung> <datei>  ausführen wie ein Klick
ctxmenu --tab dienste                    Fenster auf einem bestimmten Reiter öffnen
ctxmenu --version                        welche Fassung das ist
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
%LOCALAPPDATA%\ctxmenu\services.json    eingetragene Dienste samt Schlüssel
%LOCALAPPDATA%\ctxmenu\settings.json    Sprache und Darstellung
```

Die Schlüssel in `favourites.json` und `services.json` liegen dort im Klartext,
geschützt nur durch die Rechte auf Ihrem Benutzerprofil — wie in einer
`.npmrc` oder `.gitconfig` auch. Wer das nicht möchte, benutzt für dieses
Programm einen eigenen Schlüssel mit eingeschränkten Rechten.

Die `.reg`-Dateien sind gewöhnliche Registrierungsdateien: sie lassen sich auch
ohne dieses Werkzeug per Doppelklick zurückspielen. Eine Grenze hat das —
`reg import` fügt hinzu und überschreibt, es **entfernt nichts**. Nach einem
Löschen stellt es exakt den alten Zustand her; über einen inzwischen
veränderten Schlüssel gespielt, bleiben dessen neue Werte stehen.

Das Programm selbst geht einen Schritt weiter. Schlüssel, die es beim Sichern
noch gar nicht gab, stehen im `manifest.json` unter `absent` und werden beim
Zurückspielen wieder **entfernt** — anders ließe sich ein Blockieren nicht
rückgängig machen, denn die Blocked-Liste liefert Windows nicht mit, sie
entsteht erst mit dem ersten blockierten Handler. Für die Gesamtsicherung gilt
das ausdrücklich nicht: sie umfasst ganze Zweige wie `Directory\shell`, in die
auch jedes andere Programm schreibt, und nimmt beim Zurückspielen nichts weg.

Ein Zurückspielen bricht nicht mehr beim ersten fehlenden Schlüssel ab: jeder
Eintrag wird versucht, und am Ende steht, wie viele zurück sind und welche
nicht. Eine geteilte Aktion — ein Teil hier, ein Teil mit Administratorrechten —
legt zwei Sicherungen an; das Ergebnisfenster nennt beide und der Knopf
*Wiederherstellen* spielt beide ein.

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

Zurückgestellte Vorhaben, den Entwicklungsstand, die Messwerte und die Stellen,
an denen Windows sich anders verhält als dokumentiert, führt der Autor in
Notizen, die nicht Teil dieses Repositoriums sind.
