//! Saying something to a user who has no window and no console.
//!
//! `--favourite` is started by a click in the Explorer menu. There is no
//! terminal for `errln!` and no window to hang a dialog on, so whatever this
//! program has to say has to find its own channel.
//!
//! # Why a notification and not a message box
//!
//! The registry command of a web tool favourite ends in `"%1"`, and Windows
//! reads that as "once per file": ten selected files start ten processes.
//! Every one of them used to finish with a `MessageBoxW`, so ten modal windows
//! piled up on top of each other and each one had to be clicked away by hand.
//! A notification is handed to the platform and forgotten; nothing waits for a
//! click, and ten of them do not stack.
//!
//! # Why a WinRT toast and not a tray balloon
//!
//! The first version of this file used `Shell_NotifyIconW` with `NIF_INFO`,
//! because a balloon needs no identity of any kind and this program installs
//! nothing. It worked as far as the shell's own door, and that turned out to
//! be the wrong door.
//!
//! **A `NIF_INFO` balloon is transient: it is never written to the Action
//! Center.** Measured by reading the shell's own `wpndatabase.db`, with a
//! WinRT toast fired into the same query as a control -- the control turns up
//! in the store within seconds, the balloon never does. So a balloon Windows
//! chooses not to draw is not merely unseen, it is *gone*. On this machine
//! Focus Assist stands at `Microsoft.QuietHoursProfile.AlarmsOnly`, which is
//! exactly that case: ten files go out, one fails, and the only surviving
//! record is the log file. That is not a report, that is a receipt nobody was
//! handed.
//!
//! A toast behaves the other way round. Suppressed or not, it is stored, and
//! it is still there the next time somebody opens the Action Center. That
//! single difference is the reason for this rewrite.
//!
//! # An invented AppUserModelID gets the message stored, and no further
//!
//! The documented route to a toast wants an AUMID, and an AUMID is usually
//! earned with a shortcut in the Start menu or a packaged identity -- neither
//! of which a single portable `.exe` has. For *storage* it turns out not to
//! matter. Measured on this machine (Windows 10 Pro 19045) by counting
//! `ToastNotificationManager::History::GetHistory(aumid)` before and after:
//!
//! ```text
//! ctxmenu.ContextMenuManager (nowhere registered)  history 2 -> 3
//! PowerShell                 (properly registered) history 4 -> 5
//! ```
//!
//! `CreateToastNotifierWithId` takes an invented identifier, and the toast
//! reaches the store just the same.
//!
//! For *appearing on screen* it does not do. Measured on 2026-08-19 with Focus
//! Assist off, `ToastEnabled` at 1 and the per-app switches written by hand:
//! the same toast never drew a banner under this identifier and did draw one
//! under PowerShell's. What PowerShell has and an unregistered identifier does
//! not is a Start menu shortcut naming it in `System.AppUserModel.ID`. Hence
//! [`crate::startmenu`], which writes exactly that one file -- and see there
//! for the price, for how long the shell takes to notice, and for what it
//! keeps afterwards.
//!
//! What the invented identifier does *not* bring along either is a name. Read
//! back through `UserNotificationListener`, the toasts filed under an
//! unregistered AUMID carry `DisplayName = ""` -- the Action Center has
//! nothing to write above the message. Hence [`name_the_sender`], and see
//! there for why one small registry value is worth it.
//!
//! # COM initialises itself here
//!
//! Nothing on the WinRT side of this file calls `CoInitializeEx` or
//! `RoInitialize`, and nothing needs to: `windows_core`'s `load_factory` asks
//! `RoGetActivationFactory` for the class, and on `CO_E_NOTINITIALIZED` it
//! calls `CoIncrementMTAUsage` itself and retries. The first WinRT call in the
//! process puts the process into the multithreaded apartment, which is where
//! `ToastNotificationManager` wants to be anyway. An apartment call of our own
//! could only make that worse -- a thread already in an STA cannot be moved,
//! and in GUI mode `run_native` owns the main thread. [`crate::startmenu`]
//! does need a real apartment for `IShellLink`, and gives it back before
//! returning, so by the time the lines below run the thread is again in none.
//!
//! # `Show` returns before the banner has been drawn
//!
//! By the time `Show` comes back the toast is *stored*: the history has it and
//! the Action Center will show it whenever somebody looks. Drawing the banner
//! is not finished at that point, and a process that ends here loses it.
//! Measured on 2026-08-20 by firing the same toast from a test process that
//! was terminated the instant `Show` returned:
//!
//! ```text
//! terminated straight after Show           history grew, no banner   (2 of 2)
//! one more platform call, then terminated  history grew, banner      (2 of 2)
//! ```
//!
//! Hence the discarded `Setting()` in [`show`]. It is not a sleep and not a
//! guess at a duration: it is one synchronous question to the same service,
//! and the answer cannot come back before the service has worked through what
//! `Show` handed it. `--favourite` is exactly the process this saves -- after
//! the toast it does nothing but return from `main`.
//!
//! Still no message pump, no artificial sleep, and nothing that could change
//! an `ExitCode` or leave a process behind -- the properties the balloon had,
//! kept for the same reasons.

use anyhow::{Context as _, Result};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{
    NotificationData, NotificationUpdateResult, ToastNotification, ToastNotificationManager,
    ToastNotifier,
};
use windows::core::HSTRING;
use windows_registry::CURRENT_USER;

/// Which of the two things happened.
///
/// The same split the message box drew with `MB_ICONINFORMATION` and
/// `MB_ICONERROR`, and it still draws it -- [`crate::webtool::shell::report`]
/// falls back to a dialog and picks its icon from this. On the toast itself it
/// buys less; see [`payload`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

/// The identity every toast from this program is filed under.
///
/// Invented, and deliberately so -- see the module documentation. The shape is
/// the conventional `CompanyName.ProductName`; what matters is only that it
/// stays the same, because the history, the user's per-app notification
/// settings and the display name below all hang off this exact string.
const AUMID: &str = "ctxmenu.ContextMenuManager";

/// The name the Action Center writes above the message.
const DISPLAY_NAME: &str = "ctxmenu";

/// Where Windows looks that name up.
const AUMID_KEY: &str = r"SOFTWARE\Classes\AppUserModelId\ctxmenu.ContextMenuManager";

/// How much of each text survives into the toast.
///
/// Not a struct field this time -- the balloon's 64 and 256 `u16` were fixed
/// by `NOTIFYICONDATAW`, and a toast has no such array. What it has is a
/// documented 5 KB limit on the whole XML payload, and an over-long payload is
/// refused outright, which would land the caller in the message box this file
/// exists to avoid. So the guard stays, with room the balloon never had.
///
/// The arithmetic behind the numbers: [`escaped`] can turn one character into
/// five bytes (`&amp;`), so the worst case is `(64 + 512) * 5` plus about 120
/// bytes of markup -- under 3 KB, with the limit still a long way off.
const TITLE_LIMIT: usize = 64;
const TEXT_LIMIT: usize = 512;

/// Shows one Windows notification. Both texts are already cut to one language.
///
/// An `Err` means the notification platform refused it outright: no WinRT, or
/// a desktop with no notification platform at all. It does *not* mean the user
/// failed to see it -- a toast the platform accepted and then suppressed still
/// went to the Action Center. The caller has something else to say it with for
/// the first case and deliberately nothing for the second; see
/// [`crate::webtool::shell::report`].
pub fn show(title: &str, text: &str, level: Level) -> Result<()> {
    let notifier = notifier()?;
    let toast = toast(&payload(title, text, level))?;

    notifier
        .Show(&toast)
        .context("\x1eToast abgelehnt\x1ftoast refused\x1d")?;

    settled(&notifier);
    Ok(())
}

/// Which notification an update is meant for.
///
/// `tag` and `group` together are the platform's name for one entry in the
/// Action Center; showing a second toast under a name already in use replaces
/// the first rather than adding to it. `sequence` grows with every change, and
/// the platform drops anything that arrives with an older one -- which is what
/// makes six processes with no knowledge of each other safe to let write.
pub struct Slot<'a> {
    pub tag: &'a str,
    pub group: &'a str,
    pub sequence: u32,
}

/// Puts one notification on screen and changes it afterwards, in place.
///
/// The half of the collected report that the platform does; see
/// [`crate::webtool::batch`] for how the processes agree on what to write.
///
/// # Why the text lives in the data and not in the XML
///
/// [`ToastNotifier::UpdateWithTagAndGroup`] replaces *data-bound* values, not
/// markup: it takes a [`NotificationData`] and fills the `{name}` placeholders
/// of a toast that is already on screen. So the payload of an updatable
/// carries `<text>{ctx_body}</text>` and never the text itself, and the first
/// `Show` has to supply the same data the updates later replace.
///
/// The gain is that an update does *not* draw a new banner. Measured on
/// 2026-08-20 with a three-line body replacing a one-line one: `Update`
/// answered `Succeeded`, the banner already on screen grew where it stood, and
/// nothing flashed a second time. Showing a fresh toast under the same tag
/// would have replaced the entry just as well, but with a new banner each time
/// -- which is six banners for six files, the thing this exists to stop.
///
/// # When it falls back to showing one
///
/// `Update` answers `NotificationNotFound` once the entry has gone: the user
/// dismissed it, or the Action Center dropped it. There is then nothing to
/// change, and a new toast under the same tag is the right answer -- the same
/// answer the first process gives, which is why both paths end in the same
/// three lines.
pub fn show_or_update(title: &str, text: &str, level: Level, slot: &Slot) -> Result<()> {
    let notifier = notifier()?;
    let data = bound(title, text, slot.sequence)?;
    let tag = HSTRING::from(slot.tag);
    let group = HSTRING::from(slot.group);

    // Not for the first one: there is nothing on screen yet, and asking would
    // only be answered with `NotificationNotFound`.
    if slot.sequence > 1
        && matches!(
            notifier.UpdateWithTagAndGroup(&data, &tag, &group),
            Ok(result) if result == NotificationUpdateResult::Succeeded
        )
    {
        // For the same reason as after `Show`, and it was missed here first:
        // `Update` returns once the store has the new text, and redrawing the
        // banner already on screen comes after that. Without this the process
        // is gone before the redraw and the banner keeps the text it had --
        // measured on 2026-08-20 as a banner that stood unchanged for ten
        // seconds while four more files finished behind it.
        settled(&notifier);
        return Ok(());
    }

    let toast = toast(&bound_payload(level))?;
    toast
        .SetTag(&tag)
        .context("\x1eToast-Kennzeichen\x1ftoast tag\x1d")?;
    toast
        .SetGroup(&group)
        .context("\x1eToast-Gruppe\x1ftoast group\x1d")?;
    toast
        .SetData(&data)
        .context("\x1eToast-Daten\x1ftoast data\x1d")?;

    notifier
        .Show(&toast)
        .context("\x1eToast abgelehnt\x1ftoast refused\x1d")?;

    settled(&notifier);
    Ok(())
}

/// The notifier both entry points send through, with the identity in place.
///
/// `WithId`, not the bare `CreateToastNotifier`: the argument-less one asks
/// the process for its own AppUserModelID, and a portable `.exe` has none.
fn notifier() -> Result<ToastNotifier> {
    name_the_sender();

    // The other half of the same identity: the registry value above gives the
    // sender a name, this gives it the registration Windows wants before it
    // will draw a banner. Both are decoration on the delivery and neither may
    // fail loudly -- see there.
    crate::startmenu::ensure(AUMID);

    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .context("\x1eCreateToastNotifier\x1fCreateToastNotifier\x1d")
}

/// One toast from one XML document.
fn toast(xml: &str) -> Result<ToastNotification> {
    let document = XmlDocument::new().context("\x1eXmlDocument\x1fXmlDocument\x1d")?;
    document
        .LoadXml(&HSTRING::from(xml))
        .context("\x1eToast-XML abgelehnt\x1ftoast XML refused\x1d")?;

    ToastNotification::CreateToastNotification(&document)
        .context("\x1eCreateToastNotification\x1fCreateToastNotification\x1d")
}

/// Questions to the platform whose answers are thrown away: what is wanted is
/// the round trip. See the module documentation -- without one, this process is
/// gone before the banner is drawn and the message reaches the Action Center
/// and nothing else.
///
/// Two of them, and the second is the expensive one on purpose. `Setting()`
/// was enough while the toast carried its text in its own markup; a toast
/// whose text arrives as [`NotificationData`] gives the platform more to do
/// before it can draw anything, and one cheap question came back too early.
/// Measured on 2026-08-20, one file through `--favourite`, the banner region
/// photographed a second later:
///
/// ```text
/// text in the markup, Setting() only            banner   (the build before this one)
/// text in the data,   Setting() only            none     (5 runs, 0 banners)
/// text in the data,   Setting() and GetHistory  banner
/// ```
///
/// `GetHistory` is a real query against the notification store, so its answer
/// cannot arrive before the store has taken in what `Show` handed it. Still
/// not a sleep and still nothing that could leave a process behind.
fn settled(notifier: &ToastNotifier) {
    let _ = notifier.Setting();
    let _ = ToastNotificationManager::History()
        .and_then(|history| history.GetHistoryWithId(&HSTRING::from(AUMID)));
}

/// The two texts as the values a bound toast reads them out of.
///
/// No entity escaping here, unlike [`payload`]: these never pass a parser.
/// They are handed to the platform as strings and put into the toast as text,
/// so an `&` in a file name has to arrive as an `&` -- escaping it would show
/// the user `&amp;`. What still has to go is what [`allowed`] rules out, for
/// the same reason as there.
fn bound(title: &str, text: &str, sequence: u32) -> Result<NotificationData> {
    let data = NotificationData::new().context("\x1eNotificationData\x1fNotificationData\x1d")?;
    let values = data
        .Values()
        .context("\x1eNotificationData.Values\x1fNotificationData.Values\x1d")?;

    values
        .Insert(
            &HSTRING::from(TITLE_KEY),
            &HSTRING::from(cleaned(capped(title, TITLE_LIMIT))),
        )
        .context("\x1eToast-Titel\x1ftoast title\x1d")?;
    values
        .Insert(
            &HSTRING::from(BODY_KEY),
            &HSTRING::from(cleaned(capped(text, TEXT_LIMIT))),
        )
        .context("\x1eToast-Text\x1ftoast text\x1d")?;

    data.SetSequenceNumber(sequence)
        .context("\x1eToast-Folgenummer\x1ftoast sequence number\x1d")?;

    Ok(data)
}

/// Gives the Action Center a name to put above the message.
///
/// Without this the sender line is empty, because an AUMID that is registered
/// nowhere has nothing to be looked up in -- measured through
/// `UserNotificationListener`, which reports `DisplayName = ""` for the toasts
/// filed under it. An unnamed message in a list of named ones reads like a
/// fault, and the whole point of moving to a toast was that somebody finds it
/// there later.
///
/// This is the one registry write in this program that changes nothing about a
/// context menu, so it is worth being explicit about its shape:
///
/// * **HKCU only.** Nothing here is machine-wide and nothing here needs
///   elevation.
/// * **One value.** `DisplayName`, and not `IconUri` -- an icon would have to
///   be a `.png` lying on disk, and putting a file somewhere permanent is
///   exactly the installing this program promises not to do.
/// * **Idempotent, and quiet when there is nothing to do.** Ten processes
///   start at once for ten selected files; the read comes first so that nine
///   of them write nothing.
/// * **Failure is ignored on purpose.** The name is decoration. A message with
///   no sender is worth more than no message, and this must never be the
///   reason [`show`] returns an `Err` and summons a dialog.
///
/// It is the ordinary way an unpackaged program does this. Firefox, HandBrake,
/// TreeSize and PowerToys all carry the same key on this machine, each with
/// the same single `DisplayName` value.
fn name_the_sender() {
    if let Ok(key) = CURRENT_USER.open(AUMID_KEY)
        && let Ok(name) = key.get_string("DisplayName")
        && name == DISPLAY_NAME
    {
        return;
    }

    let _ = CURRENT_USER
        .create(AUMID_KEY)
        .and_then(|key| key.set_string("DisplayName", DISPLAY_NAME));
}

/// The toast as the notification platform wants it: one XML document.
///
/// `ToastGeneric` with two `<text>` elements -- the first is drawn as the
/// heading, the second as the body, and both are what the Action Center keeps.
///
/// `duration="long"` is all [`Level`] can buy here. The toast platform has no
/// per-message error icon; the picture beside a toast comes from the sender's
/// registered `IconUri` and is the same for every message it sends. So an
/// error stays on screen for the long interval instead of the short one, and
/// in the Action Center the two look alike and the text carries the difference
/// -- which it can, because the caller writes it.
fn payload(title: &str, text: &str, level: Level) -> String {
    let duration = match level {
        Level::Info => "",
        Level::Error => " duration=\"long\"",
    };

    format!(
        "<toast{duration}><visual><binding template=\"ToastGeneric\">\
         <text>{}</text><text>{}</text>\
         </binding></visual></toast>",
        escaped(capped(title, TITLE_LIMIT)),
        escaped(capped(text, TEXT_LIMIT)),
    )
}

/// What the two `{…}` placeholders of an updatable toast are called.
///
/// Prefixed, because the names live in the same space as the ones the platform
/// uses for a progress bar (`progressValue`, `progressStatus`). Nothing here
/// draws a progress bar today, and a collision would be a silent one.
const TITLE_KEY: &str = "ctx_title";
const BODY_KEY: &str = "ctx_body";

/// The same document as [`payload`], with the two texts left out.
///
/// They arrive separately, as the values of a [`NotificationData`], and that is
/// what makes the notification changeable afterwards -- see [`show_or_update`].
fn bound_payload(level: Level) -> String {
    let duration = match level {
        Level::Info => "",
        Level::Error => " duration=\"long\"",
    };

    format!(
        "<toast{duration}><visual><binding template=\"ToastGeneric\">\
         <text>{{{TITLE_KEY}}}</text><text>{{{BODY_KEY}}}</text>\
         </binding></visual></toast>"
    )
}

/// `text` cut to at most `limit` characters, on a character boundary.
///
/// Cut along `chars`, never along bytes: half a UTF-8 sequence is not a string
/// at all, and `&str` will not even hold one.
fn capped(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((byte, _)) => &text[..byte],
        None => text,
    }
}

/// `text` as XML character data, with everything a parser cannot hold removed
/// rather than passed on.
///
/// Two jobs, and the second one is the one that would have bitten:
///
/// * The three markup characters become entities. Not `"` and `'` -- this text
///   only ever lands between tags, never inside an attribute, where they would
///   need escaping and where nothing here goes.
/// * Characters XML 1.0 forbids are dropped. The C0 block is not decoration
///   here: `crate::bilingual` marks its two languages with the information
///   separators U+001E, U+001F and U+001D, and a stray one that outlived the
///   cut would make `LoadXml` reject the *whole document*. One unbalanced
///   marker in one message, and the user gets the message box this file exists
///   to avoid -- for ten files at once. A character quietly missing from a
///   sentence is the cheaper failure by a wide margin.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());

    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            character if allowed(character) => out.push(character),
            _ => {}
        }
    }

    out
}

/// The half of [`escaped`] that a bound value still needs.
///
/// A value handed to [`NotificationData`] is not markup and is not parsed, so
/// the three entities would arrive on screen spelled out. The dropping stays:
/// a bilingual marker that outlived the cut would be an unprintable character
/// in the middle of a file name, and the tab and the line breaks are what put
/// six names under each other.
fn cleaned(text: &str) -> String {
    text.chars().filter(|c| allowed(*c)).collect()
}

/// Whether XML 1.0 allows this character at all.
///
/// The `Char` production of the specification, section 2.2. Rust has already
/// ruled out the surrogate range for us -- a `char` cannot hold one -- so what
/// is left to exclude is most of C0 and the two non-characters at the end of
/// the basic plane.
fn allowed(character: char) -> bool {
    matches!(character,
        '\t' | '\n' | '\r'
        | ' '..='\u{d7ff}'
        | '\u{e000}'..='\u{fffd}'
        | '\u{10000}'..='\u{10ffff}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_is_left_alone() {
        assert_eq!(capped("ok", 64), "ok");
        assert_eq!(capped("", 64), "");
        assert_eq!(escaped("ok"), "ok");
    }

    #[test]
    fn a_long_text_is_cut_to_the_limit() {
        let long = "a".repeat(700);
        assert_eq!(capped(&long, TEXT_LIMIT).len(), TEXT_LIMIT);
        assert_eq!(capped(&long, TITLE_LIMIT).len(), TITLE_LIMIT);
    }

    #[test]
    fn the_cut_lands_on_a_character_and_not_in_the_middle_of_one() {
        // Every character here is four bytes in UTF-8, so a byte-wise cut
        // would not even produce a `&str`. Counting characters must give the
        // limit, and counting bytes must give four times as much.
        let text = "\u{1F600}".repeat(100);
        let cut = capped(&text, 10);
        assert_eq!(cut.chars().count(), 10);
        assert_eq!(cut.len(), 40);
    }

    #[test]
    fn german_text_survives_the_cut() {
        // Measured in characters, not in bytes: umlauts are two bytes each in
        // UTF-8, and a sentence that fits must not be shortened for that.
        let text = "Öffnen fehlgeschlagen: Größe überschritten";
        assert_eq!(capped(text, TEXT_LIMIT), text);
    }

    #[test]
    fn the_three_markup_characters_become_entities() {
        assert_eq!(
            escaped("a & b < c > d"),
            "a &amp; b &lt; c &gt; d",
            "an ampersand in a URL is the everyday case"
        );
        assert_eq!(
            escaped(r#"He said "hi" and 'bye'"#),
            r#"He said "hi" and 'bye'"#,
            "quotes stay: this text never lands in an attribute"
        );
    }

    #[test]
    fn a_stray_bilingual_marker_never_reaches_the_parser() {
        // The regression this guards: U+001E, U+001F and U+001D are how
        // `crate::bilingual` marks its two languages. One that outlived the
        // cut would make `LoadXml` reject the whole document, and the user
        // would get ten message boxes instead of one toast.
        let text = format!("kaputt{}Rest", crate::bilingual::OPEN);
        assert_eq!(escaped(&text), "kaputtRest");

        for marker in [
            crate::bilingual::OPEN,
            crate::bilingual::SPLIT,
            crate::bilingual::CLOSE,
        ] {
            assert!(!allowed(marker), "{marker:?} is not XML character data");
        }
    }

    #[test]
    fn tab_and_the_two_line_breaks_are_kept() {
        // The only C0 characters XML allows, and a multi-line body is the
        // normal shape of an error message here.
        assert_eq!(escaped("a\tb\nc\rd"), "a\tb\nc\rd");
        assert_eq!(escaped("a\u{0}b\u{7}c\u{b}d"), "abcd");
    }

    #[test]
    fn a_payload_names_the_template_and_carries_both_texts() {
        let xml = payload("Fertig", "3 Dateien gesendet", Level::Info);
        assert!(xml.starts_with("<toast>"), "{xml}");
        assert!(xml.contains(r#"template="ToastGeneric""#), "{xml}");
        assert!(xml.contains("<text>Fertig</text>"), "{xml}");
        assert!(xml.contains("<text>3 Dateien gesendet</text>"), "{xml}");
        assert!(xml.ends_with("</toast>"), "{xml}");
    }

    #[test]
    fn only_an_error_asks_to_stay_on_screen() {
        assert!(!payload("t", "b", Level::Info).contains("duration"));
        assert!(
            payload("t", "b", Level::Error).contains(r#"<toast duration="long">"#),
            "an error is worth the long interval"
        );
    }

    #[test]
    fn a_payload_stays_far_below_the_platforms_limit() {
        // The worst case the arithmetic beside `TEXT_LIMIT` describes: every
        // character escaping to five bytes. 5 KB is where the platform stops
        // accepting a payload.
        let title = "&".repeat(500);
        let text = "&".repeat(5000);
        let xml = payload(&title, &text, Level::Error);
        assert!(xml.len() < 3072, "{} bytes", xml.len());
    }

    #[test]
    fn a_bound_payload_carries_the_two_placeholders_and_no_text() {
        let xml = bound_payload(Level::Info);
        assert!(xml.contains("<text>{ctx_title}</text>"), "{xml}");
        assert!(xml.contains("<text>{ctx_body}</text>"), "{xml}");
        assert!(
            xml.contains(r#"template="ToastGeneric""#),
            "the same template as the fixed one, so the two look alike: {xml}"
        );
        assert!(
            bound_payload(Level::Error).contains(r#"<toast duration="long">"#),
            "an error is worth the long interval here too"
        );
    }

    #[test]
    fn the_placeholders_are_spelled_the_same_in_the_xml_and_in_the_data() {
        // Two halves that have to agree and are written down separately: the
        // markup names them in braces, `bound` inserts them without. A typo in
        // either is a toast that shows `{ctx_body}` to the user.
        let xml = bound_payload(Level::Info);
        assert!(xml.contains(&format!("{{{TITLE_KEY}}}")));
        assert!(xml.contains(&format!("{{{BODY_KEY}}}")));
    }

    #[test]
    fn a_bound_value_keeps_the_characters_a_parsed_one_has_to_escape() {
        // The difference between the two paths, and the reason for the second
        // function: a bound value never passes a parser, so escaping it would
        // put `&amp;` in front of the user.
        assert_eq!(cleaned("Rechnung & Co.png"), "Rechnung & Co.png");
        assert_eq!(escaped("Rechnung & Co.png"), "Rechnung &amp; Co.png");
    }

    #[test]
    fn a_bound_value_drops_what_no_toast_can_hold() {
        // A bilingual marker that outlived the cut, and the line break that
        // puts six file names under each other.
        let text = format!("eins.png{}zwei.png", crate::bilingual::OPEN);
        assert_eq!(cleaned(&text), "eins.pngzwei.png");
        assert_eq!(cleaned("eins.png\nzwei.png"), "eins.png\nzwei.png");
    }

    #[test]
    fn the_key_path_and_the_identifier_cannot_drift_apart() {
        // Two constants that have to agree: the notifier is created with
        // `AUMID`, and Windows looks the name up under a path that spells the
        // same string out again.
        assert_eq!(
            AUMID_KEY,
            format!(r"SOFTWARE\Classes\AppUserModelId\{AUMID}")
        );
    }
}
