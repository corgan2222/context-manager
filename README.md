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
  COM-Handler getrennt ausgewiesen.
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
| **Sicherungen** | Verlauf aller Sicherungen, mit Knopf zum Zurückspielen |

Das Suchfeld greift auf jedem Reiter und durchsucht Anzeigename, Befehl und
Registry-Pfad; auch dann, wenn links noch nichts ausgewählt ist.

### Eine typische Runde

1. Reiter **Programme**, das Programm anklicken, das stört.
2. Rechts prüfen, was daran hängt — Pfad, Befehl, Bereich.
3. **Ausblenden** statt Löschen. Das ist umkehrbar und reicht fast immer.
4. Wenn Windows nach Administratorrechten fragt: das sind die Einträge unter
   `HKLM`, also die für alle Konten. Wer ablehnt, behält die Änderungen an den
   eigenen Einträgen; die anderen bleiben, wie sie waren.

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
