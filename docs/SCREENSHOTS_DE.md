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

Ein Lauf über die zehn englischen Bilder dauert 56 Sekunden, gemessen am
2026-08-19, weil jedes Bild das Programm neu startet. Beide Sprachen kosten
das Doppelte.

## Was aufgenommen wird

Neun Ansichten, in der Reihenfolge, in der ein Leser das Programm kennenlernen
sollte.

| Name | Gestartet mit | Wofür |
|---|---|---|
| `01-overview` | `--tab categories` | Das eine Bild, das sagen muss, worum es geht |
| `02-entry-detail` | `--tab categories --search 7-Zip` | Registry-Pfad, Bereich, Programm, Merkmale |
| `03-search` | `--tab categories --search git` | Den einen Eintrag finden, der stört |
| `04-new-entry` | `--new directory` | Das Formular, das ein eigenes Programm ins Menü bringt |
| `05-file-types` | `--tab filetypes --ext .png` | Die Auflösungskette, die sonst kein Werkzeug zeigt |
| `06-programs` | `--tab programs` | Zwanzig Schlüssel eines Programms als eine Zeile |
| `07-favourites` | `--tab favourites` | Programme und Webtools, die der Benutzer hineingelegt hat |
| `08-services` | `--service snapotter` | Aus einer OpenAPI-Beschreibung werden Einträge, mit den Werkzeugen im Bild |
| `09-backups` | `--tab backups` | Das Versprechen, das den Rest gefahrlos macht |
| `10-many-entries` | `--tab categories --synthetic 2000` | Die Aussage zur Geschwindigkeit, mit sichtbarer Zeilenzahl |

Die Liste steht als Datenstruktur oben im Skript, mit einer Zeile `Use` je
Eintrag. Eine Ansicht dazuzunehmen heißt: vier Zeilen dort ergänzen, sonst
nichts.

## Was die Bilder wiederholbar macht

Am 2026-08-19 gemessen, mit allem Folgenden an Ort und Stelle: **zehn von
zehn auf den Bildpunkt gleich**, `08-services` eingeschlossen, das seine
Werkzeugliste über HTTP holt.

Eins musste dafür erst repariert werden, sonst hätte die Zahl nichts bedeutet.
Diese Fassung von ImageMagick ist ein Q16-HDRI-Bau und meldet `-metric AE` als
Bruchteil statt als Anzahl: zwei Läufe derselben Aufnahme kamen mit
`0.294118 (8.4501e-08)` zurück, Statuszeile und Bildlaufleiste schon
abgeschnitten. Die alte Prüfung nannte alles über null eine Änderung und
schrieb sie dann als „0 Bildpunkte anders" hin: ein Phantom bei jedem Lauf,
und ein Vergleich, dem bald niemand mehr glaubt. Die Schwelle ist jetzt ein
ganzer Bildpunkt.

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
  melden `startup_to_first_list_ms`; die fünf ohne (der Dialog für einen neuen
  Eintrag, Programme, Favoriten, Dienste, Sicherungen) melden das nie, sondern
  nur `window_placed`. Jeder
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

1. **Deutsch ist abgeschaltet, der Satz ist einsprachig.** `-Languages` steht
   auf `en`, solange an der Oberfläche noch gearbeitet wird; das halbiert die
   Laufzeit. Jeder Titel in `$shots` steht weiter in beiden Sprachen da, ein
   `-Languages de,en` holt den vollen Satz zurück. Für den Release-Satz wieder
   anschalten — und die deutschen Bilder dann ansehen: deutsche Wörter sind
   länger, und eine Spalte, die auf Englisch passt, kann trotzdem umbrechen.
2. **`08-services` hängt an etwas außerhalb dieser Maschine.** Als einzige
   Aufnahme. `--service snapotter` holt die Beschreibung über HTTP; antwortet
   der Dienst nicht, steht im Bild die rote Fehlerzeile statt der Werkzeuge,
   und die Kennung muss es in der eigenen `services.json` auch geben. Wer
   diesen Satz nachstellt, braucht den Dienst erreichbar oder tauscht den
   Eintrag gegen einen eigenen.
3. **`--new` nimmt eine Kategorie, keine Dateiendung.** `--new ext:.png` wird
   abgelehnt, weil `Category::from_slug` auch `create --category` speist, und
   das schreibt: den Weg zu erweitern hieße, einen Schreibpfad für ein Bild zu
   erweitern. Die Aufnahme kann deshalb nur eine der sieben Basiskategorien
   zeigen.
4. **Zwei Dialoge fehlen weiter.** Die Rückfrage vor dem Schreiben und das
   Über-Fenster sind nach wie vor nur per Klick erreichbar. Gleiche Form der
   Lösung wie bei `--new`: ein Argument, das eines öffnet.
5. **Der Sicherungen-Reiter ist voller Testreste.** 1274 der 1289 Verzeichnisse
   unter `%LOCALAPPDATA%\ctxmenu\backups` stammen aus Testläufen.
   `09-backups` zeigt sie über den echten Sicherungen. Vor dem Release-Satz
   `tools\backups_aufraeumen.ps1 -Apply` laufen lassen.
6. **Nichts ist beschriftet.** Für die README wollen einige Bilder eine
   Hervorhebung oder einen vergrößerten Ausschnitt. ImageMagick ist installiert
   und wird vom Skript schon für den Vergleich benutzt, `magick ... -annotate`
   ist von hier aus ein kleiner Schritt.
7. **Das Video ist nicht angefangen.** ffmpeg ist installiert. Die Bausteine
   liegen bereit (feste Zustände, feste Fenstergröße, beide Sprachen), aber
   nichts fährt bisher eine Abfolge ab und zeichnet sie auf. Die naheliegende
   Form ist eine Liste von Schritten wie die `$shots`-Liste, mit einer Dauer je
   Schritt.
8. **Nur diese Maschine.** Alles oben ist auf vier Bildschirmen mit 3840x2160
   bei 150 % gemessen. Ein Lauf auf einem einzelnen 1920x1080-Bildschirm bei
   100 % ist nicht versucht worden, und `--window 2400x1500` passt dort nicht.
