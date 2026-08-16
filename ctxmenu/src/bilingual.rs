//! One language on screen, both languages in the source.
//!
//! The lower layers — `registry`, `webtool`, `elevation`, `favourites`,
//! `settings` — write their messages down in German and English at once,
//! because none of them ever learns which language the user picked. Until now
//! they said both at the same time, so every reader read half a sentence they
//! did not need. The two alternatives are marked instead, and the cut happens
//! once, where the text finally reaches a screen or a console.
//!
//! # What a marked message looks like
//!
//! ```text
//! "\x1eEintrag anlegen\x1fcreating entry\x1d: {path}"
//!    German half    ^   English half   ^      shared, printed either way
//! ```
//!
//! Three characters out of the C0 information-separator block: `\x1e` opens
//! the German alternative, `\x1f` separates it from the English one, `\x1d`
//! ends the group. That block was put into ASCII for exactly this job and its
//! characters appear in nothing this program handles — not in a registry path,
//! not in a file name, not in a command line, not in a URL, and inside JSON
//! only in escaped form.
//!
//! The obvious cheaper idea — cut a finished message at `" / "` — was tried
//! and dropped. `"{:.1} fps / {:.2} ms"` and `"MUI-Cache: {} Treffer / {}
//! Auflösungen"` are not translations of anything; a favourite may be called
//! `"Scan / Prüfen"`; `scan --json` prints registry data that nobody wrote for
//! a reader. A rule that guesses gets those wrong, and a wrongly cut sentence
//! is harder to act on than one that says too much.
//!
//! # Why the shared text stays outside the group
//!
//! Whatever stands outside a group is printed in both languages, so a payload
//! is written once and neither half repeats it. That is also what makes the
//! chained form work without knowing anything about chains: `{error:#}` joins
//! the levels of an `anyhow::Error` with `": "`, and because every group
//! carries its own end marker, cutting stays local to the group. Nothing here
//! has to find out where one level stops and the next begins.

use std::borrow::Cow;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::settings::{Language, Settings};

/// Opens the German alternative.
pub const OPEN: char = '\u{1e}';
/// Separates the German alternative from the English one.
pub const SPLIT: char = '\u{1f}';
/// Closes the group. Everything after it is shared again.
pub const CLOSE: char = '\u{1d}';

/// One of the three characters this module gives a meaning to.
pub fn is_marker(c: char) -> bool {
    c == OPEN || c == SPLIT || c == CLOSE
}

/// Cuts every marked group down to one language.
///
/// Text that carries no marker is returned borrowed and byte for byte
/// unchanged — that is the whole promise of this function, and the reason a
/// `Cow` is handed back rather than a `String`: an untouched message is
/// visible in the type. Registry paths, JSON, file names and a service's own
/// wording therefore pass through this untouched, whatever they contain.
///
/// Malformed input is treated as text, never as a boundary. A lone marker, a
/// group without its end, an end that arrives before the separator: in each of
/// those cases every word survives and only the stray marker characters are
/// dropped. Saying too much is a nuisance; saying half of something is a bug.
pub fn pick(text: &str, language: Language) -> Cow<'_, str> {
    // Every marker, not just the opener: a lone separator carried in from a
    // service's answer has to be swept out too, and it must not make the
    // common case allocate.
    if !text.contains(is_marker) {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find(OPEN) {
        out.push_str(&rest[..open]);
        let after = &rest[open + OPEN.len_utf8()..];

        match group(after) {
            Some(found) => {
                out.push_str(match language {
                    Language::German => found.german,
                    Language::English => found.english,
                });
                rest = found.rest;
            }
            // Nothing usable behind this marker: drop the marker, keep the
            // text, carry on looking. A later group in the same line is still
            // cut correctly.
            None => rest = after,
        }
    }

    out.push_str(rest);
    // Whatever survived from a malformed group would otherwise reach the
    // screen as an invisible control character.
    out.retain(|c| !is_marker(c));
    Cow::Owned(out)
}

/// Both alternatives of one group, plus what follows it.
struct Group<'a> {
    german: &'a str,
    english: &'a str,
    rest: &'a str,
}

/// Reads one group from text that begins directly after an [`OPEN`].
///
/// Returns `None` unless the group is complete and simple: a separator, then
/// an end marker, and no further marker in between. A nested or unfinished
/// group is not repaired here — it is handed back to the caller as ordinary
/// text.
///
/// The check on the two halves is what stops foreign text from moving the
/// boundary. A message such as `"Kein Feld {path} …"` interpolates a field
/// name that came out of a service's answer; if that name carried a marker of
/// its own, the group would be cut in the wrong place. It is refused instead,
/// and the reader sees both languages rather than the wrong half of one.
fn group(after_open: &str) -> Option<Group<'_>> {
    let split = after_open.find(SPLIT)?;
    let close = after_open.find(CLOSE)?;
    if close < split {
        return None;
    }

    let german = &after_open[..split];
    let english = &after_open[split + SPLIT.len_utf8()..close];
    if german.contains(is_marker) || english.contains(is_marker) {
        return None;
    }

    Some(Group {
        german,
        english,
        rest: &after_open[close + CLOSE.len_utf8()..],
    })
}

/// The same cut, in whatever language this process is currently showing.
///
/// For the funnels that have no `Settings` at hand: the console, the message
/// box a favourite raises when it was started from Explorer, the file picker's
/// filter labels.
pub fn shown(text: &str) -> Cow<'_, str> {
    pick(text, language())
}

/// One cell of a table: cut to one language first, padded to `width` after.
///
/// The order is the whole reason this function exists. Everywhere else the cut
/// happens on the way out, in [`crate::console::line`], and for a table that is
/// one step too late. `format!("{:<22}", marked)` counts the three marker
/// characters as text and pads to 22 with them counted in; the printer then
/// removes them again, together with the half nobody asked for, so the column
/// arrives short — and short by a different amount in each language. Measured
/// on the scan header, `\x1eSchlüssel\x1fkey\x1d` in a 22-wide column: 16
/// characters printed in German and 10 in English, where both should have been
/// 22, so every row underneath stopped lining up.
///
/// Padded, never shortened. A heading wider than its column pushes the line to
/// the right, which looks careless; a heading cut off in the middle cannot be
/// read at all.
pub fn column(text: &str, width: usize, language: Language) -> String {
    format!("{:<width$}", pick(text, language))
}

/// The whole chain of an `anyhow::Error`, in one language.
///
/// Convenience over [`pick`], not a second mechanism: `{error:#}` produces the
/// levels joined with `": "`, and the markers are cut inside that string just
/// as they are anywhere else.
pub fn error(error: &anyhow::Error, language: Language) -> String {
    pick(&format!("{error:#}"), language).into_owned()
}

/// The language this process shows text in.
///
/// The user's saved choice, falling back to the Windows UI language on a
/// machine that has never opened the window — "the PC is either English or
/// German" is what it already knows about itself. The window calls
/// [`set_language`] when the setting changes, so a running program follows
/// along instead of finishing in the language it started in.
///
/// The settings file is read at most once. This is asked for once per printed
/// line, and the first thing it is usually asked for is the message saying why
/// something else failed.
pub fn language() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        GERMAN => Language::German,
        ENGLISH => Language::English,
        _ => {
            static START: OnceLock<Language> = OnceLock::new();
            *START
                .get_or_init(|| Settings::load_or_default(crate::theme::system_language()).language)
        }
    }
}

/// Tells the rest of the program which language the window is showing.
pub fn set_language(language: Language) {
    CURRENT.store(
        match language {
            Language::German => GERMAN,
            Language::English => ENGLISH,
        },
        Ordering::Relaxed,
    );
}

/// `UNSET` means nobody has said yet, so [`language`] falls back to the
/// settings file. An atomic rather than a lock: this is read on every printed
/// line and written twice in the life of the program.
static CURRENT: AtomicU8 = AtomicU8::new(UNSET);
const UNSET: u8 = 0;
const GERMAN: u8 = 1;
const ENGLISH: u8 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    const GERMAN: Language = Language::German;
    const ENGLISH: Language = Language::English;

    /// Writes a group the way the source does.
    fn marked(german: &str, english: &str) -> String {
        format!("{OPEN}{german}{SPLIT}{english}{CLOSE}")
    }

    #[test]
    fn text_without_a_marker_comes_back_untouched_and_unallocated() {
        for plain in [
            "Zugriff verweigert (os error 5)",
            "HKCU\\Software\\Classes\\*\\shell\\Foo",
            "",
        ] {
            assert!(
                matches!(pick(plain, GERMAN), Cow::Borrowed(same) if same == plain),
                "{plain:?} was rewritten"
            );
            assert!(matches!(pick(plain, ENGLISH), Cow::Borrowed(_)));
        }
    }

    #[test]
    fn the_german_half_is_shown_for_german_and_the_english_half_for_english() {
        let message = marked("Keine Web-Adresse", "not a web address");

        assert_eq!(pick(&message, GERMAN), "Keine Web-Adresse");
        assert_eq!(pick(&message, ENGLISH), "not a web address");
    }

    #[test]
    fn the_text_outside_a_group_belongs_to_both_languages() {
        // The payload is written once and read by everyone: this is why the
        // converted messages did not have to grow a second copy of their
        // arguments.
        let message = format!(
            "\"reg.exe\" {}: exit code 1",
            marked("konnte nicht gestartet werden", "could not be started")
        );

        assert_eq!(
            pick(&message, GERMAN),
            "\"reg.exe\" konnte nicht gestartet werden: exit code 1"
        );
        assert_eq!(
            pick(&message, ENGLISH),
            "\"reg.exe\" could not be started: exit code 1"
        );
    }

    #[test]
    fn a_line_with_three_pairs_cuts_all_three() {
        // cli.rs prints this one; it used to read
        // "Programme vorhanden / present: 12, nicht mehr da / gone: 3, …".
        let message = format!(
            "{}: 12, {}: 3, {}: 1",
            marked("Programme vorhanden", "present"),
            marked("nicht mehr da", "gone"),
            marked("nicht prüfbar", "unknown"),
        );

        assert_eq!(
            pick(&message, GERMAN),
            "Programme vorhanden: 12, nicht mehr da: 3, nicht prüfbar: 1"
        );
        assert_eq!(pick(&message, ENGLISH), "present: 12, gone: 3, unknown: 1");
    }

    #[test]
    fn a_chained_anyhow_error_is_cut_at_every_level() {
        // The shape that made a plain " / " rule impossible: two levels joined
        // by ": ", each with its own pair, plus an OS message that has no pair
        // at all and must survive whole.
        let inner = anyhow::anyhow!(
            "{}: Zugriff verweigert (os error 5)",
            marked("Wert setzen fehlgeschlagen", "could not set the value")
        );
        let outer = inner.context(marked("Eintrag anlegen", "creating entry"));

        assert_eq!(
            error(&outer, GERMAN),
            "Eintrag anlegen: Wert setzen fehlgeschlagen: Zugriff verweigert (os error 5)"
        );
        assert_eq!(
            error(&outer, ENGLISH),
            "creating entry: could not set the value: Zugriff verweigert (os error 5)"
        );
    }

    #[test]
    fn a_slash_between_two_numbers_is_not_a_language_boundary() {
        // All three live in the source today and are not translations of one
        // another. A rule that cut at " / " would have halved them.
        for innocent in [
            "58.9 fps / 1.42 ms",
            "MUI-Cache: 41 Treffer / 12 Auflösungen",
            "Seite 3 / 7",
        ] {
            assert_eq!(pick(innocent, GERMAN), innocent);
            assert_eq!(pick(innocent, ENGLISH), innocent);
        }
    }

    #[test]
    fn a_name_the_user_chose_survives_whatever_is_in_it() {
        // A favourite called "Scan / Prüfen", a key named "open / close", a
        // path with a slashed directory: all of them reach a console line or a
        // dialog, and none of them is a translation.
        let payload = "Scan / Prüfen";
        let message = format!(
            "{}: {payload}",
            marked("Kennung schon vergeben", "id already taken")
        );

        assert_eq!(
            pick(&message, GERMAN),
            "Kennung schon vergeben: Scan / Prüfen"
        );
        assert_eq!(pick(&message, ENGLISH), "id already taken: Scan / Prüfen");
    }

    #[test]
    fn machine_readable_output_is_never_touched() {
        // `ctxmenu scan --json` goes through the same console funnel as every
        // message. If cutting could reach it, an entry name with a slash in it
        // would produce JSON that no longer parses.
        let json = r#"{"name":"Mit Notepad öffnen / open with notepad","scope":"user"}"#;

        for language in [GERMAN, ENGLISH] {
            assert!(matches!(pick(json, language), Cow::Borrowed(_)));
            assert_eq!(pick(json, language), json);
        }
    }

    #[test]
    fn a_group_without_an_end_marker_keeps_every_word() {
        // Half a message is worse than a doubled one, so an unfinished group
        // is read as text.
        let broken = format!("{OPEN}Abbruch{SPLIT}cancelled: 5");

        assert_eq!(pick(&broken, GERMAN), "Abbruchcancelled: 5");
        assert_eq!(pick(&broken, ENGLISH), "Abbruchcancelled: 5");
    }

    #[test]
    fn an_end_marker_before_the_separator_is_not_a_group() {
        let broken = format!("{OPEN}Abbruch{CLOSE}cancelled{SPLIT}");

        assert_eq!(pick(&broken, GERMAN), "Abbruchcancelled");
        assert_eq!(pick(&broken, ENGLISH), "Abbruchcancelled");
    }

    #[test]
    fn a_stray_marker_from_a_foreign_answer_is_dropped_not_obeyed() {
        // A web service may answer with anything at all, control characters
        // included. Its text must not be able to cut our sentence.
        let answer = format!("Der Dienst antwortete: feld{SPLIT}wert{CLOSE}ende");

        assert_eq!(pick(&answer, GERMAN), "Der Dienst antwortete: feldwertende");
        assert_eq!(
            pick(&answer, ENGLISH),
            "Der Dienst antwortete: feldwertende"
        );
    }

    #[test]
    fn a_stray_opener_is_dropped_and_the_groups_behind_it_are_still_cut() {
        // One damaged marker must not cost the rest of the line. The first
        // opener has no group of its own, so it goes; what follows is read
        // normally.
        let message = format!(
            "{OPEN}{} und {}",
            marked("kaputt", "broken"),
            marked("gut", "fine")
        );

        assert_eq!(pick(&message, GERMAN), "kaputt und gut");
        assert_eq!(pick(&message, ENGLISH), "broken und fine");
    }

    #[test]
    fn foreign_text_carrying_a_marker_cannot_move_the_boundary() {
        // The field name comes out of a service's answer. Even if it smuggles
        // a separator in, the group is refused rather than cut in the wrong
        // place — both languages are shown, which is the harmless failure.
        let field = format!("na{SPLIT}me");
        let message = format!("{OPEN}Kein Feld {field} in der Antwort{SPLIT}no such field{CLOSE}");

        for language in [GERMAN, ENGLISH] {
            let shown = pick(&message, language);
            assert!(shown.contains("Kein Feld"), "{shown:?}");
            assert!(shown.contains("no such field"), "{shown:?}");
            assert!(!shown.contains(is_marker), "{shown:?}");
        }
    }

    #[test]
    fn no_marker_ever_reaches_the_screen() {
        let samples = [
            marked("a", "b"),
            format!("{OPEN}dangling"),
            format!("x{SPLIT}y{CLOSE}z"),
            format!("{}{}", marked("a", "b"), marked("c", "d")),
            "harmlos".to_string(),
        ];

        for sample in samples {
            for language in [GERMAN, ENGLISH] {
                let shown = pick(&sample, language);
                assert!(!shown.contains(is_marker), "{sample:?} -> {shown:?}");
            }
        }
    }

    #[test]
    fn cutting_never_adds_a_word_that_was_not_there() {
        // Every character of the output came from the input; the function only
        // ever removes. Cheap to state, and it rules out the whole class of
        // bugs where a marker is mistaken for a placeholder.
        let message = format!("vorn {} hinten", marked("deutsch", "english"));

        for language in [GERMAN, ENGLISH] {
            for word in pick(&message, language).split_whitespace() {
                assert!(message.contains(word), "{word:?} was invented");
            }
        }
    }

    #[test]
    fn a_column_is_cut_before_it_is_padded_and_not_the_other_way_round() {
        // The bug the function was written against, stated as a number: the
        // late cut takes the markers and the unwanted half out of a column
        // that was already counted as full.
        let heading = marked("Schlüssel", "key");
        let too_late = format!("{heading:<22}");
        assert_eq!(pick(&too_late, GERMAN).chars().count(), 16);
        assert_eq!(pick(&too_late, ENGLISH).chars().count(), 10);

        // Cut first, and both languages fill the column they were given.
        for (language, word) in [(GERMAN, "Schlüssel"), (ENGLISH, "key")] {
            let cell = column(&heading, 22, language);
            assert_eq!(cell.chars().count(), 22, "{cell:?}");
            assert!(cell.starts_with(word), "{cell:?}");
            assert!(!cell.contains(is_marker), "{cell:?}");
        }
    }

    #[test]
    fn a_heading_wider_than_its_column_keeps_every_letter() {
        // Widening the line is a blemish; a heading chopped in half is
        // unreadable, so the column gives way rather than the word.
        let heading = marked("Anzeigename", "Display name");

        assert_eq!(column(&heading, 4, ENGLISH), "Display name");
        assert_eq!(column(&heading, 4, GERMAN), "Anzeigename");
    }

    #[test]
    fn a_column_without_any_marker_is_padded_like_any_other_text() {
        // "Scope" and "Flags" are spelled the same in both languages and carry
        // no group; they go through the same call so the header reads as one
        // piece of code rather than two.
        assert_eq!(column("Scope", 7, GERMAN), "Scope  ");
        assert_eq!(column("Scope", 7, ENGLISH), "Scope  ");
    }

    #[test]
    fn the_process_language_can_be_set_and_read_back() {
        let before = language();

        set_language(Language::English);
        assert_eq!(language(), Language::English);
        assert_eq!(shown(&marked("deutsch", "english")), "english");

        set_language(Language::German);
        assert_eq!(language(), Language::German);
        assert_eq!(shown(&marked("deutsch", "english")), "deutsch");

        set_language(before);
    }
}

/// Guards the rewrite itself, by reading the source it produced.
///
/// Not a unit test of a function: a check that the messages which were
/// converted stayed converted, and that the next one is not written in the old
/// shape. It reads the crate's own `src`, which is where the messages live.
#[cfg(test)]
mod source {
    use super::*;

    /// How the three markers are spelled in Rust source.
    const IN_SOURCE: [(&str, char); 3] = [(r"\x1e", OPEN), (r"\x1f", SPLIT), (r"\x1d", CLOSE)];

    /// Files that talk about the mechanism instead of using it.
    const EXEMPT: [&str; 2] = ["bilingual.rs", "i18n.rs"];

    /// Reads a source file the way the compiler would: marker escapes turned
    /// into real markers, and a backslash at the end of a line folded away
    /// together with the indentation that follows it. Without the folding,
    /// every message wrapped over two lines would look broken even though the
    /// string it produces is fine.
    fn readable(path: &std::path::Path) -> String {
        let mut text = std::fs::read_to_string(path).unwrap_or_default();
        for (escape, marker) in IN_SOURCE {
            text = text.replace(escape, &marker.to_string());
        }
        let mut folded = String::with_capacity(text.len());
        let mut rest = text.as_str();
        // The working copy is checked out with CRLF, so the continuation is a
        // backslash followed by two characters, not one.
        while let Some(cut) = rest
            .match_indices('\\')
            .map(|(at, _)| at)
            .find(|at| rest[at + 1..].starts_with(['\n', '\r']))
        {
            folded.push_str(&rest[..cut]);
            rest = rest[cut + 1..]
                .trim_start_matches(['\r', '\n'])
                .trim_start_matches([' ', '\t']);
        }
        folded.push_str(rest);
        folded
    }

    /// The first 40 characters after an unclosed marker, for the panic
    /// message below.
    ///
    /// Counted in `char`s, not bytes: `&after[..40.min(after.len())]` looks
    /// equivalent but panics whenever byte 40 lands inside a multi-byte
    /// character — an umlaut, unremarkable in this project's German source —
    /// which is exactly the moment this diagnostic is needed most.
    fn near(after: &str) -> String {
        after.chars().take(40).collect()
    }

    fn sources() -> Vec<std::path::PathBuf> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut todo = vec![root];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    todo.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path
                        .file_name()
                        .is_some_and(|n| !EXEMPT.iter().any(|skip| n == *skip))
                {
                    found.push(path);
                }
            }
        }
        found
    }

    #[test]
    fn every_marked_message_in_the_source_is_a_complete_group() {
        for path in sources() {
            let text = readable(&path);
            let mut rest = text.as_str();
            while let Some(open) = rest.find(OPEN) {
                let after = &rest[open + OPEN.len_utf8()..];
                let found = group(after).unwrap_or_else(|| {
                    panic!(
                        "{}: a group is never closed near {:?}",
                        path.display(),
                        near(after)
                    )
                });
                assert!(
                    !runs_past_its_literal(found.german) && !runs_past_its_literal(found.english),
                    "{}: a group runs past the end of its literal: {:?}",
                    path.display(),
                    found.german
                );
                rest = found.rest;
            }
        }
    }

    #[test]
    fn the_diagnostic_snippet_survives_a_multi_byte_boundary_at_forty() {
        // Regression: `&after[..40.min(after.len())]` panicked here instead
        // of producing the message it was building, whenever byte 40 landed
        // inside a multi-byte character. 39 ASCII bytes put the following
        // "ä" exactly on that boundary.
        let after = format!("{}ä {}", "a".repeat(39), "no closing marker follows");
        assert!(
            !after.is_char_boundary(40),
            "test setup missed the byte-40 boundary"
        );

        let snippet = near(&after);
        assert!(snippet.starts_with(&"a".repeat(39)));
        assert!(snippet.contains('ä'));
    }

    #[test]
    fn no_message_is_left_saying_both_languages_at_once() {
        // The one state worse than either end of the rewrite is the middle of
        // it: one language in front, both behind. `" / "` between two words is
        // how the old shape looked, and nothing in the source needs it any
        // more — arithmetic, ratios and comments do not have spaces around a
        // slash between letters.
        let mut left = Vec::new();
        for path in sources() {
            for (number, line) in std::fs::read_to_string(&path)
                .unwrap_or_default()
                .lines()
                .enumerate()
            {
                if !line.trim_start().starts_with("//") && bilingual_looking(line) {
                    left.push(format!(
                        "{}:{}: {}",
                        path.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            left.is_empty(),
            "{} message(s) still say it twice:\n{}",
            left.len(),
            left.join("\n")
        );
    }

    /// Did half a group swallow the end of the literal it lives in? A newline
    /// or a quote that is not escaped means the closing marker was forgotten
    /// and the group ran on into the code. `\"` is ordinary content — the
    /// `--sub` message quotes its own example.
    fn runs_past_its_literal(half: &str) -> bool {
        if half.contains(['\n', '\r']) {
            return true;
        }
        let mut escaped = false;
        for c in half.chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => return true,
                _ => {}
            }
        }
        false
    }

    /// A German word, a slash, an English word, inside a string literal — the
    /// old shape, and nothing else in this code base.
    ///
    /// Both halves of the test earn their keep. Without the letters, `n / k`
    /// and `1000.0 / average` would be reported; without the literal check,
    /// `hits as f64 / total as f64` would be. What is left is text written for
    /// a reader.
    ///
    /// A question mark counts as the end of a word here, and that is not
    /// decoration: `"schreibgeschützt? / read-only?"` in `update.rs` survived
    /// the rewrite for exactly this reason, because the character in front of
    /// the slash was `?` rather than a letter. Sentence-ending punctuation is
    /// where a translated question stops, so it belongs on this side of the
    /// check. Arithmetic is unaffected — `?` before a slash means the `?`
    /// operator, which never appears inside a string literal.
    fn bilingual_looking(line: &str) -> bool {
        let ends_a_word = |c: char| c.is_alphabetic() || matches!(c, '?' | '!' | '.');
        line.match_indices(" / ").any(|(at, _)| {
            let before = line[..at].chars().next_back().unwrap_or(' ');
            let after = line[at + 3..].chars().next().unwrap_or(' ');
            ends_a_word(before) && after.is_alphabetic() && inside_a_literal(line, at)
        })
    }

    #[test]
    fn a_translated_question_is_caught_even_though_it_ends_in_punctuation() {
        // The line as it stood in update.rs until 2026-08-16. The guard read
        // over it for two months because `?` is not alphabetic, so the one
        // message written in the old shape was the one message never reported.
        let missed = r#"    "{} \x1ebeiseite legen\x1fmoving aside\x1d: schreibgeschützt? / read-only?","#;
        assert!(bilingual_looking(missed));

        // The shapes that made the letter check necessary in the first place
        // stay out, and `?` before a slash outside a literal is the operator.
        for arithmetic in [
            "let share = hits as f64 / total as f64;",
            "let each = 1000.0 / average;",
            "let per = width / columns;",
            "let value = parse(text)? / divisor;",
        ] {
            assert!(!bilingual_looking(arithmetic), "reported: {arithmetic}");
        }
    }

    /// Is the byte at `at` inside a string literal? Counted by unescaped
    /// quotes, which is enough for one line of Rust.
    fn inside_a_literal(line: &str, at: usize) -> bool {
        let mut quotes = 0;
        let mut escaped = false;
        for c in line[..at].chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quotes += 1,
                _ => {}
            }
        }
        quotes % 2 == 1
    }
}
