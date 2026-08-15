# Sicherheitsrichtlinie

`ctxmenu` ist ein Desktop-Programm. Es läuft als der angemeldete Benutzer auf
einem Windows-PC, liest und schreibt die Registry-Schlüssel des Kontextmenüs und
kann Dateien an Webdienste schicken, die der Benutzer selbst eingetragen hat. Es
ist kein Dienst, hat keine Konten und lauscht auf keinem Anschluss.

Drei Dinge machen es trotzdem sicherheitsrelevant, und darum steht der Rahmen
hier ausdrücklich statt zum Erraten:

- Es **schreibt in die Registry**, teils nach `HKLM`, also für alle Konten.
- Es fordert dafür **erhöhte Rechte** an und startet sich selbst neu.
- Es **verschickt Dateien** an Adressen und speichert die Schlüssel dazu.

## Unterstützte Fassungen

Nur die jeweils neueste Veröffentlichung.

| Fassung | Korrekturen |
|---|---|
| Neueste Veröffentlichung | Ja |
| Alles ältere | Nein — erst aktualisieren |

Das Projekt hat einen Betreuer. Es gibt keinen Zweig, auf dem eine ältere
Fassung weiter gepflegt wird, und eine Tabelle, die etwas anderes verspricht,
wäre ein Versprechen, das niemand halten kann.

## Eine Schwachstelle melden

**Bitte nicht als öffentliches Issue.** Issues sind in dem Moment öffentlich, in
dem sie angelegt werden, und jeder Leser ist jemand, der handeln kann, bevor es
eine Korrektur gibt.

Zwei private Wege, beide sind recht:

- **Private Meldung über GitHub** — im Reiter *Security* dieses Repositoriums,
  *Report a vulnerability*. Bevorzugt: Meldung, Diskussion und Korrektur bleiben
  an einem Ort, und Sie sehen den Patch, bevor er öffentlich wird.
- **E-Mail an `stefan@knaak.org`** mit `ctxmenu security` im Betreff. Auf der
  Empfangsseite liegt nichts verschlüsselt; wenn ein Detail für Klartextpost zu
  heikel ist, bitten Sie kurz um einen anderen Weg.

Was eine Meldung schnell bearbeitbar macht: die Fassung aus dem
Über-Fenster, der Windows-Build, die Schritte zum Auslösen, und — falls
vorhanden — der Auszug aus `%LOCALAPPDATA%\ctxmenu\ctxmenu.log`. **Sehen Sie
das Protokoll vorher durch:** es nennt Registry-Pfade und Dateinamen von Ihrem
Rechner.

## Was als Schwachstelle zählt

Alles, was einer dieser Sätze beschreibt:

- Ein Weg, über den das Programm **etwas anderes schreibt, als der Benutzer
  bestätigt hat** — insbesondere außerhalb der Registry-Bereiche, die es
  verwaltet, oder ohne die Sicherung, die es zusagt.
- Ein Weg, über den die **erhöhten Rechte** für etwas anderes benutzt werden als
  für den einen bestätigten Schritt; jede Möglichkeit, den erhöhten Vorgang von
  außen zu beeinflussen.
- Ein Weg, über den **eine Datei verschickt wird**, ohne dass der Benutzer dem
  für dieses Werkzeug zugestimmt hat, oder an eine andere Adresse als die
  eingetragene.
- Ein Weg, über den ein **gespeicherter Schlüssel** an jemanden gelangt, der ihn
  nicht ohnehin lesen dürfte.
- Ein **bösartiges OpenAPI-Dokument** oder eine bösartige Antwort eines Dienstes,
  die das Programm dazu bringt, außerhalb des Zielordners zu schreiben,
  auszuführen, was es nicht soll, oder abzustürzen.
- Eine **Sicherung, die nicht zurückspielt**, was sie zu enthalten behauptet.

## Was ausdrücklich keine Schwachstelle ist

- **Die Schlüssel liegen im Klartext** in `%LOCALAPPDATA%\ctxmenu\`. Das ist
  gewollt und dokumentiert: geschützt sind sie durch die Rechte auf dem
  Benutzerprofil, wie in einer `.npmrc` oder `.gitconfig` auch. Wer das nicht
  will, benutzt einen Schlüssel mit eingeschränkten Rechten. Ein Angreifer, der
  in Ihrem Profil lesen kann, hat ohnehin schon gewonnen.
- **Das Programm kann das Kontextmenü kaputt machen.** Das ist sein Zweck. Es
  sichert vorher und sagt vorher, was es tut.
- **Ein Eintrag kann jeden Befehl ausführen**, den der Benutzer hineinschreibt.
  Das ist die Funktion des Kontextmenüs, nicht ein Fehler darin.
- **Unverschlüsseltes `http://`**, wenn es für einen Favoriten ausdrücklich
  erlaubt wurde. Das Programm lehnt es ab, bis jemand den Haken setzt.
- **SmartScreen warnt vor der `.exe`.** Sie ist nicht signiert; ein
  Zertifikat, dem Windows von sich aus traut, kostet mehrere hundert Euro im
  Jahr. Die Prüfsumme jeder Veröffentlichung steht bei der Veröffentlichung.
- Meldungen aus einem Schwachstellen-Scanner **ohne einen Weg, wie sich das
  hier auswirkt**. Eine Abhängigkeit mit einer CVE in einem Codepfad, den dieses
  Programm nicht benutzt, ist keine Schwachstelle dieses Programms.

## Was Sie erwarten können

- **Eingangsbestätigung innerhalb von drei Tagen.** Wenn nach einer Woche nichts
  kommt, ist die Mail untergegangen — dann bitte über den anderen Weg nachhaken.
- **Eine Einschätzung innerhalb von zwei Wochen**: bestätigt, kein Fehler, oder
  eine Rückfrage.
- **Nennung, wenn Sie das möchten**, in der Veröffentlichung, die es behebt.
- Kein Geld. Das ist ein Freizeitprojekt ohne Einnahmen.
