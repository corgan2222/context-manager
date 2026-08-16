# Mitmachen

*[English version](CONTRIBUTING.md)*

Danke fürs Reinschauen. Das hier ist ein kleines Projekt mit festen Meinungen;
die Regeln unten sind das, was es klein hält.

## Die eine Regel

**Gemessen, nicht vermutet.**

Windows verhält sich an mehreren Stellen anders, als seine Dokumentation sagt.
Dieses Programm behauptet deshalb nichts, was nicht an einem echten System
nachgeprüft wurde — und wo eine Zahl fehlt, steht auch keine Behauptung. Eine
Änderung, die auf „müsste eigentlich" beruht, ist keine.

Alles Weitere folgt daraus.

## Einrichten

Rust 1.95 oder neuer, `x86_64-pc-windows-msvc`, dazu die Visual-Studio-Build-Tools
mit C++-Werkzeugkette. Dann:

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Alle vier müssen grün sein. `-D warnings` ist nicht verhandelbar.

Das Ergebnis liegt unter `target\x86_64-pc-windows-msvc\release\ctxmenu.exe`,
nicht unter `target\release\`: `.cargo\config.toml` nennt das Ziel ausdrücklich,
damit die statisch gebundene C-Laufzeit für die Anwendung gilt und nicht auch
für die Makro-Bibliotheken des Übersetzers.

## Was in einen Pull Request gehört

- **Ein Test je neuer reiner Funktion.** Testnamen sind ganze Sätze, die sagen,
  was gilt: `fn a_range_whose_ends_are_the_wrong_way_round_is_no_range_at_all`.
  Wer den Namen nicht ausformulieren kann, hat die Regel noch nicht verstanden.
- **Kommentare, die das *Warum* erklären.** Was der Code tut, steht im Code.
  Wertvoll ist, was ihn erklärt: welche Alternative verworfen wurde, welche
  Messung dahintersteht, welche Windows-Eigenheit ihn erzwingt.
- **Englisch im Code**, auch in Kommentaren und Bezeichnern. Oberflächentexte
  laufen über `ctxmenu/src/i18n.rs` und gibt es zweimal, deutsch und englisch.
- **Commit-Nachrichten in ganzen Sätzen**, die sagen, was die Änderung bewirkt
  und warum. Kein `feat:`-Präfix.

## Was Änderungen an der Registry angeht

Der heikelste Teil, also die strengsten Regeln:

- **Nie löschen ohne Sicherung.** Das ist keine Bitte, sondern vom Typsystem
  geprüft: `write::delete_tree` verlangt einen `BackupToken`, und den gibt es
  nur als Rückgabewert eines geglückten `backup::export`.
- **Ein Ziel ist ein `RegTarget`**, keine Zeichenkette. Was sich nicht als
  einzelner Eintrag unterhalb einer Classes-Wurzel ausdrücken lässt, soll gar
  nicht erst konstruierbar sein.
- **Schreibversuche nach `HKLM` gehören in eine Wegwerf-VM**, nicht auf die
  Entwicklungsmaschine. Ein `tools\`-Skript setzt eine auf.

## Was eher abgelehnt wird

- **Neue Abhängigkeiten.** Jede muss sich rechtfertigen; die Liste ist kurz und
  soll es bleiben.
- **Umbauten ohne Fehler dahinter.** Refactoring, das nichts repariert und
  nichts ermöglicht, kostet Prüfzeit und bringt Risiko.
- **Funktionen, die Windows nicht hergibt.** Die freie Sortierung von
  Menüeinträgen zum Beispiel: nachgemessen, das System kennt nur `Position=Top`
  und `Position=Bottom`. Was daran scheitert, steht in der README unter „Was es
  bewusst nicht kann".
- **Automatisch erzeugte Übersetzungen.** Beide Sprachen sind von Hand
  geschrieben und sollen gleich gut lesbar sein.

## Fehler melden

Was eine Meldung schnell bearbeitbar macht:

- Die Fassung aus dem Über-Fenster und den Windows-Build.
- Den kürzesten Weg zum Auslösen.
- Den betroffenen Registry-Pfad, wenn es um einen Eintrag geht.
- Den Auszug aus `%LOCALAPPDATA%\ctxmenu\ctxmenu.log` — **vorher durchsehen**,
  er nennt Pfade und Dateinamen von Ihrem Rechner.

Eine Sicherheitslücke gehört **nicht** in ein Issue: siehe `SECURITY_DE.md`.
