# Bildschirmaufnahmen: wie sie entstehen, und was noch fehlt

*[English version](SCREENSHOTS.md)*

`tools\screenshots.ps1` nimmt bei jedem Lauf dieselben Bilder auf, deutsch und
englisch. Nach einer Änderung zeigt ein Vergleich damit, was sich an der
Oberfläche geändert hat, und nicht, wo das Fenster gerade stand.

Diese Datei ist die Arbeitsnotiz dazu: was das Skript tut, warum es so
entschieden wurde, und was vor einem Release noch zu tun ist.

## Aufrufen

```powershell
pwsh tools\screenshots.ps1                          # alle neun Ansichten, beide Sprachen
pwsh tools\screenshots.ps1 -Only '05-*'             # eine Ansicht
pwsh tools\screenshots.ps1 -Languages de            # eine Sprache
pwsh tools\screenshots.ps1 -Compare                 # neu aufnehmen und mit dem letzten Satz vergleichen
pwsh tools\screenshots.ps1 -WindowSize 3000x1900    # andere Größe
```

Die Bilder landen in `tmp\screenshots\`. Der Ordner steht in `.gitignore`: die
Aufnahmen sind Artefakte, und was ein Release wirklich braucht, wird von Hand
nach `docs\images\` kopiert.

Ein voller Lauf dauert rund vier Minuten, weil jedes Bild das Programm neu
startet.

## Was aufgenommen wird

Neun Ansichten, in der Reihenfolge, in der ein Leser das Programm kennenlernen
sollte.

| Name | Gestartet mit | Wofür |
|---|---|---|
| `01-uebersicht` | `--tab categories` | Das eine Bild, das sagen muss, worum es geht |
| `02-eintrag-im-detail` | `--tab categories --search 7-Zip` | Registry-Pfad, Bereich, Programm, Merkmale |
| `03-suche` | `--tab categories --search git` | Den einen Eintrag finden, der stört |
| `04-dateitypen` | `--tab filetypes --ext .png` | Die Auflösungskette, die sonst kein Werkzeug zeigt |
| `05-programme` | `--tab programs` | Zwanzig Schlüssel eines Programms als eine Zeile |
| `06-favoriten` | `--tab favourites` | Programme und Webtools, die der Benutzer hineingelegt hat |
| `07-dienste` | `--tab services` | Aus einer OpenAPI-Beschreibung werden Einträge |
| `08-sicherungen` | `--tab backups` | Das Versprechen, das den Rest gefahrlos macht |
| `09-viele-eintraege` | `--synthetic 2000` | Die Aussage zur Geschwindigkeit, mit sichtbarer Zeilenzahl |

Die Liste steht als Datenstruktur oben im Skript, mit einer Zeile `Use` je
Eintrag. Eine Ansicht dazuzunehmen heißt: vier Zeilen dort ergänzen, sonst
nichts.

## Was die Bilder wiederholbar macht

Über zwei Läufe gemessen, mit allem Folgenden an Ort und Stelle: **jedes Bild
auf den Bildpunkt gleich.**

* **`--window 2400x1500`** legt die Größe fest, damit nichts zwischen zwei
  Läufen umbricht. Physische Bildpunkte: bei 150 % macht diese Maschine daraus
  1600x1000 logische Punkte, und dafür ist die Oberfläche gebaut. Bei 1600
  physischen (1067 logischen) gemessen: die Statuszeile überlappt sich selbst
  und die Werkzeugleiste beschneidet ihre eigenen Knöpfe, dieselbe Klasse
  Problem, die `CLAUDE.md` bei 1267 logischen Punkten festhält.
* **`--lang de|en`** legt die Sprache für den Lauf fest, **ohne
  `%LOCALAPPDATA%\ctxmenu\settings.json` anzufassen**. Das Skript bildet vorher
  und nachher die Prüfsumme dieser Datei und meldet keinen Erfolg, wenn sie sich
  geändert hat. Beide Argumente sind für dieses Skript entstanden; vorher hieß
  Sprache umschalten, die Einstellungen des Benutzers zu überschreiben.
* **`--synthetic <n>`** füllt die Tabelle mit erzeugten Zeilen, wo der Inhalt
  nicht zur Sache tut. Der Erzeuger ist deterministisch: gleiche Zahl, gleiche
  Zeilen, auf jeder Maschine.
* **Auf die richtige Zeile in der Fehlerausgabe warten.** Ansichten mit Tabelle
  melden `startup_to_first_list_ms`; die vier ohne (Programme, Favoriten,
  Dienste, Sicherungen) melden das nie, sondern nur `window_placed`. Jeder
  Eintrag sagt, worauf zu warten ist, danach wird noch einmal auf die Symbole
  gewartet.
* **Zwei Streifen werden vor dem Vergleich abgeschnitten**, weil beide sich von
  allein ändern und sonst bei jedem Bild einen Unterschied melden würden:
  * die unteren 40 Bildpunkte, die Statuszeile mit Bildern je Sekunde,
    Bildzeit und Startdauer;
  * die rechten 16 Bildpunkte, die Bildlaufleiste, die egui je nach Zeit seit
    dem letzten Ereignis ein- und ausblendet. Gemessen: allein sie machte jeden
    Unterschied zwischen zwei Läufen aus, ein Streifen von 10x1390 bei x=2390,
    dieselben 3065 Bildpunkte in allen vier geprüften Bildern.

## Drei Dinge, die auf dieser Maschine stimmen mussten

Alle drei stehen in `CLAUDE.md` und haben Zeit gekostet, bevor sie verstanden
waren.

* **DPI zuerst anmelden.** Vier Bildschirme mit 3840x2160 bei 150 %. Ein Skript
  ohne `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` sieht sie als
  2560x1440 und bekommt von `GetWindowRect` zwei Drittel der echten
  Koordinaten. `tools\capture_window.ps1` meldet sich bis heute nicht an, seine
  Zahlen sind hier um den Faktor 1,5 daneben.
* **`PrintWindow` liefert bei diesem OpenGL-Fenster Schwarz**: gemessen, kein
  einziger heller Bildpunkt. Es bleibt die Bildschirmkopie, und damit muss das
  Fenster vorher nach oben und hinterher zurück.
* **Nur das eigene Fensterrechteck wird kopiert**, nie der ganze Bildschirm.
  Auf den anderen Bildschirmen liegt Privates.

## Warum es sich unter Windows PowerShell neu startet

PowerShell 7 bringt `System.Drawing` nicht mehr mit; es steckt jetzt im Paket
`System.Drawing.Common`, und `Bitmap` scheitert dort mit CS1069. Windows
PowerShell 5.1 hat es noch und liegt jedem Windows bei, also läuft der
Aufnahmeteil dort. Das Skript erkennt `$PSVersionTable.PSEdition -eq 'Core'` und
startet sich einmal neu, über `-Command` und nicht über `-File`: mit `-File`
kommt jedes Argument als eine Zeichenkette an, `-Languages de,en` wird also zum
wörtlichen `"de,en"` und scheitert an seinem `ValidateSet`.

## Was vor einem Release noch zu tun ist

1. **Der Dienste-Reiter zeigt eine leere Fläche.** `--tab services` öffnet die
   Liste, ohne dass ein Dienst gewählt ist; das Bild zeigt links einen Namen und
   rechts „Links einen Dienst wählen". Ausgerechnet der Reiter mit dem
   auffälligsten Merkmal hat das schwächste Bild. Es fehlt ein Argument in der
   Art `--service <id>`, das einen auswählt und lädt, so wie `--ext` schon eine
   Endung vorwählt.
2. **Keine Dialoge.** Editor, die Rückfrage vor dem Schreiben und das
   Über-Fenster sind nur per Klick erreichbar. Jedes davon gäbe ein besseres
   Bild als ein Reiter. Gleiche Form der Lösung: ein Argument, das eines öffnet.
3. **Der Sicherungen-Reiter ist voller Testreste.** 1274 der 1289 Verzeichnisse
   unter `%LOCALAPPDATA%\ctxmenu\backups` stammen aus Testläufen.
   `08-sicherungen` zeigt sie über den echten Sicherungen. Vor dem Release-Satz
   `tools\backups_aufraeumen.ps1 -Apply` laufen lassen.
4. **Nichts ist beschriftet.** Für die README wollen einige Bilder eine
   Hervorhebung oder einen vergrößerten Ausschnitt. ImageMagick ist installiert
   und wird vom Skript schon für den Vergleich benutzt, `magick ... -annotate`
   ist von hier aus ein kleiner Schritt.
5. **Das Video ist nicht angefangen.** ffmpeg ist installiert. Die Bausteine
   liegen bereit (feste Zustände, feste Fenstergröße, beide Sprachen), aber
   nichts fährt bisher eine Abfolge ab und zeichnet sie auf. Die naheliegende
   Form ist eine Liste von Schritten wie die `$shots`-Liste, mit einer Dauer je
   Schritt.
6. **Nur diese Maschine.** Alles oben ist auf vier Bildschirmen mit 3840x2160
   bei 150 % gemessen. Ein Lauf auf einem einzelnen 1920x1080-Bildschirm bei
   100 % ist nicht versucht worden, und `--window 2400x1500` passt dort nicht.
