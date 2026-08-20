//! Six processes, one click.
//!
//! The registry command of a web tool favourite ends in `"%1"`, and Windows
//! reads that as "once per file": six selected files start six processes.
//! None of them knows about the others, and two things that ought to happen
//! once used to happen six times — the question before the first upload, and
//! the report at the end.
//!
//! # What is shared, and how
//!
//! One file per favourite and per run, under `%TEMP%\ctxmenu-batch\`, guarded
//! by two named mutexes. Nothing here is a database: it is three lines of
//! text, and every reader holds a lock while it reads, changes and writes them
//! back.
//!
//! ```text
//! started 1755648000123        when the first process of this run got here
//! consent yes                  what the one dialog was answered with
//! done bild.ohne-meta.png      one line per finished file
//! ```
//!
//! Two mutexes rather than one, because they are held for very different
//! lengths of time. `ctxmenu.consent.<id>` is held across a dialog — that is
//! however long the person at the screen takes. `ctxmenu.batch.<id>` is held
//! across a read, a write and one call into the notification platform, which
//! is milliseconds. Sharing one lock between the two would mean the five
//! waiting processes queue behind a human.
//!
//! # Which processes belong together
//!
//! The starts cluster, the finishes do not: six uploads of the same picture
//! finish anywhere between eight seconds and two minutes apart, but Explorer
//! spawns all six processes within about a second. So a run is defined by when
//! its processes *started*: [`Batch::join`] runs before any work, and a
//! process joins the session already in the file only if that session began
//! less than [`JOIN_WINDOW`] ago. A click a minute later gets a session of its
//! own, even while the first one is still uploading.
//!
//! The file name carries the day, so a run of tomorrow cannot attach itself to
//! one of today, and [`Batch::join`] deletes what is left from other days.
//!
//! # When the coordination fails
//!
//! Every entry point answers "could not" rather than raising, and the caller
//! then does what this program did before any of this existed: ask its own
//! question, show its own notification. Six notifications are a nuisance; a
//! file that was never sent because a lock could not be taken is a defect.
//! That is the whole of the trade, and it is why there is a timeout on every
//! wait.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::HSTRING;

use super::Outcome;

/// How long after the first process a later one may still join its run.
///
/// Generous on purpose, because the two ways of getting it wrong do not cost
/// the same. Too long, and two deliberate clicks a minute apart share one
/// notification -- which is close to what was asked for anyway. Too short, and
/// the sixth process starts a *new* session, which takes the list of the first
/// five with it: the file is overwritten, and the notification that named five
/// files is replaced by one naming a single file.
///
/// Measured on 2026-08-20: six processes started from PowerShell with
/// `Start-Process` were 10.9 seconds apart end to end -- one launcher's
/// overhead, not Explorer's, which spawns its six within a second. A limit of
/// ten seconds would have been too tight for the measurement itself, which is
/// as good a warning as any.
const JOIN_WINDOW: Duration = Duration::from_secs(60);

/// How long a process waits for the one that is showing the dialog.
///
/// The question is answered by a person, so this is generous. What it must not
/// be is absent: a dialog nobody answers would otherwise hold five processes
/// for as long as the machine is on, each with a file the user asked to have
/// sent.
const CONSENT_PATIENCE: Duration = Duration::from_secs(120);

/// How long a process waits for the shared file.
///
/// Only ever held across a read, a write and one call into the notification
/// platform. Anything approaching this is a process that has died holding it,
/// and a mutex an owner died holding is handed on as abandoned rather than
/// waited for -- so this is a bound on the pathological case, not on the
/// normal one.
const FILE_PATIENCE: Duration = Duration::from_secs(10);

/// One process's place in a run.
///
/// Held from before the work starts until the report at the end, because the
/// two ends have to agree on which run this is: [`Batch::report`] refuses to
/// write into a session that began after this process joined.
pub struct Batch {
    id: String,
    started: u64,
}

impl Batch {
    /// Finds the run this process belongs to, or starts one.
    ///
    /// Called before any work, so that runs are told apart by when their
    /// processes started rather than by when they happened to finish. `None`
    /// means the coordination is unavailable and every caller should fall back
    /// to doing its own thing.
    pub fn join(id: &str) -> Option<Batch> {
        let path = session_path(id)?;
        let _guard = Guard::take(&file_lock(id), FILE_PATIENCE)?;

        sweep(path.parent()?, today());

        let now = now_millis();
        let mut session = read(&path);
        if session.started == 0 || now.saturating_sub(session.started) > millis(JOIN_WINDOW) {
            session = Session {
                started: now,
                ..Session::default()
            };
            write(&path, &session).ok()?;
        }

        Some(Batch {
            id: id.to_string(),
            started: session.started,
        })
    }

    /// Adds this process's result to the run's one notification.
    ///
    /// `false` means it did not get there, and the caller shows its own.
    pub fn report(&self, title: &str, outcome: &Outcome) -> bool {
        let Some(path) = session_path(&self.id) else {
            return false;
        };
        let Some(_guard) = Guard::take(&file_lock(&self.id), FILE_PATIENCE) else {
            return false;
        };

        let mut session = read(&path);
        if session.started != self.started {
            // A later click has taken the file over. Writing into it would put
            // this file's name under that run's notification.
            return false;
        }

        let body = collected(
            &mut session,
            crate::bilingual::shown(&outcome.label).into_owned(),
            &crate::bilingual::shown(&outcome.message),
        );
        if write(&path, &session).is_err() {
            return false;
        }

        crate::notify::show_or_update(
            &crate::bilingual::shown(title),
            &body,
            crate::notify::Level::Info,
            &crate::notify::Slot {
                tag: &self.started.to_string(),
                group: &fingerprint(&self.id),
                sequence: session.done.len() as u32,
            },
        )
        .is_ok()
    }
}

/// The one question before a file leaves the machine, asked once for a run.
///
/// Whoever gets the gate reads the recorded consent again -- another process
/// may have written it while this one was waiting -- and only asks if there is
/// still nothing there. The answer, *including* a no, goes into the session
/// file, so the five waiting processes honour a refusal instead of asking
/// again.
///
/// A no is deliberately not written to `favourites.json`: refusing today is
/// not a standing decision, and the file has exactly one field this program is
/// allowed to write.
pub fn consent(batch: Option<&Batch>, id: &str, ask: impl FnOnce() -> bool) -> bool {
    let gate = batch.and_then(|_| Guard::take(&consent_lock(id), CONSENT_PATIENCE));

    let Some((batch, _gate)) = batch.zip(gate) else {
        // No coordination: exactly what this did before, question and all.
        return recorded(ask(), id);
    };

    // Read again rather than trust the copy from before the wait: the process
    // that held the gate has just written both of these.
    if already_agreed(id) {
        return true;
    }

    if let Some(path) = session_path(id)
        && let Some(_guard) = Guard::take(&file_lock(id), FILE_PATIENCE)
        && let Some(answer) = read(&path).consent
    {
        return answer;
    }

    let answer = recorded(ask(), id);

    // Even a no is worth keeping for the length of the run, and losing this
    // write costs at most a repeated question.
    if let Some(path) = session_path(id)
        && let Some(_guard) = Guard::take(&file_lock(id), FILE_PATIENCE)
    {
        let mut session = read(&path);
        if session.started == batch.started {
            session.consent = Some(answer);
            let _ = write(&path, &session);
        }
    }

    answer
}

/// Takes this file into the run and says what the notification should now read.
///
/// One file looks exactly as it did before any of this existed: the whole
/// sentence, with the path in it and the byte count after it. From the second
/// onwards those sentences neither fit nor get read, so the bare names take
/// over, one under the other -- which is what was asked for, and the full path
/// is the part that could not be read there anyway.
///
/// A line already in the list is not added again. Six processes that all have
/// the same thing to say -- "nothing was sent", after one refusal answered for
/// all of them -- then make one line rather than six.
fn collected(session: &mut Session, label: String, message: &str) -> String {
    if !session.done.contains(&label) {
        session.done.push(label);
    }

    match session.done.as_slice() {
        [_only] => message.to_string(),
        many => many.join("\n"),
    }
}

/// Writes a yes down where it belongs and hands the answer back unchanged.
///
/// The one field, and not the whole favourite: this process was started by a
/// right-click and the window may well be open beside it, with a favourite
/// half renamed. Writing back a copy read at the top of the run would take
/// that rename with it. A failure to write must never stop the upload the user
/// just agreed to, hence the discarded result.
fn recorded(answer: bool, id: &str) -> bool {
    if answer {
        let _ = crate::favourites::remember_consent(id);
    }
    answer
}

/// Whether the saved list already carries this tool's agreement.
///
/// Read from disk rather than from the copy the run started with: the process
/// that held the gate may have written it a moment ago, which is the entire
/// point of waiting for that gate.
fn already_agreed(id: &str) -> bool {
    match crate::favourites::find(id) {
        Ok(favourite) => match favourite.tool {
            crate::favourites::Tool::Web(web) => web.confirmed,
            crate::favourites::Tool::Program { .. } => false,
        },
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// The shared file.
// ---------------------------------------------------------------------------

/// What one run has got to so far.
#[derive(Debug, Default, PartialEq)]
struct Session {
    /// Milliseconds since the epoch, and the identity of the run: it is the
    /// notification's tag as well as what [`Batch::report`] checks against.
    started: u64,
    /// The answer to the one question, `None` while nobody has asked yet.
    consent: Option<bool>,
    /// One per finished file, in the order they finished.
    done: Vec<String>,
}

/// The file as text, one fact per line.
///
/// Deliberately not JSON: it is read and written under a lock by this program
/// alone, a person looking at it during a run should be able to read it, and a
/// line that has gone strange costs one line rather than the whole file.
fn render(session: &Session) -> String {
    let mut out = format!("started {}\n", session.started);
    if let Some(consent) = session.consent {
        out.push_str(if consent {
            "consent yes\n"
        } else {
            "consent no\n"
        });
    }
    for done in &session.done {
        out.push_str("done ");
        // A line break inside a label would become a second entry on the way
        // back in. Nothing produces one today; a file name could.
        out.push_str(&done.replace(['\r', '\n'], " "));
        out.push('\n');
    }
    out
}

/// The other direction. Anything unrecognised is skipped rather than refused:
/// half a session is worth more than none.
fn parse(raw: &str) -> Session {
    let mut session = Session::default();

    for line in raw.lines() {
        match line.split_once(' ') {
            Some(("started", value)) => session.started = value.trim().parse().unwrap_or(0),
            Some(("consent", value)) => session.consent = Some(value.trim() == "yes"),
            Some(("done", value)) if !value.trim().is_empty() => {
                session.done.push(value.trim().to_string())
            }
            _ => {}
        }
    }

    session
}

fn read(path: &std::path::Path) -> Session {
    std::fs::read_to_string(path)
        .map(|raw| parse(&raw))
        .unwrap_or_default()
}

fn write(path: &std::path::Path, session: &Session) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, render(session))
}

/// Removes what other days left behind.
///
/// The directory is this program's own, so everything in it that is not from
/// today is finished with. Every failure is ignored: a file that will not go
/// away is a few dozen bytes in `%TEMP%`, and a run must not fail over it.
fn sweep(directory: &std::path::Path, today: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if from_another_day(&name, today) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether a file in the directory belongs to a day that is over.
fn from_another_day(name: &str, today: u64) -> bool {
    name.ends_with(".txt") && !name.ends_with(&format!("-{today}.txt"))
}

/// Where this favourite's run of today is written down.
fn session_path(id: &str) -> Option<PathBuf> {
    Some(std::env::temp_dir().join("ctxmenu-batch").join(format!(
        "{}-{}.txt",
        fingerprint(id),
        today()
    )))
}

fn today() -> u64 {
    now_millis() / 86_400_000
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

/// A favourite id as something a kernel object name and a file name can both
/// hold.
///
/// FNV-1a, sixteen hex digits. Ids are made from the favourite's name and may
/// carry anything a person typed; a mutex name may not contain a backslash and
/// a file name may not contain half a dozen other characters. Collisions merge
/// two runs into one notification, which is a cosmetic fault at worst.
fn fingerprint(id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn file_lock(id: &str) -> String {
    format!(r"Local\ctxmenu.batch.{}", fingerprint(id))
}

fn consent_lock(id: &str) -> String {
    format!(r"Local\ctxmenu.consent.{}", fingerprint(id))
}

// ---------------------------------------------------------------------------
// The lock itself.
// ---------------------------------------------------------------------------

/// A named mutex, held for as long as this value lives.
///
/// `Local\` rather than `Global\`: the processes to coordinate are all started
/// by one click of one user in one session, and a global name would be a name
/// every other session on the machine has to agree about.
struct Guard(HANDLE);

impl Guard {
    /// `None` if the mutex could not be made or was not free in time. Both
    /// mean the same thing to every caller: carry on alone.
    fn take(name: &str, patience: Duration) -> Option<Guard> {
        let handle = unsafe { CreateMutexW(None, false, &HSTRING::from(name)) }.ok()?;

        let waited = unsafe { WaitForSingleObject(handle, patience.as_millis() as u32) };
        // WAIT_ABANDONED is ownership too: the process that held it died, and
        // Windows hands the mutex on rather than leaving it locked forever.
        if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
            return Some(Guard(handle));
        }

        unsafe { CloseHandle(handle) }.ok();
        None
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_survives_the_round_trip() {
        let session = Session {
            started: 1_755_648_000_123,
            consent: Some(true),
            done: vec!["eins.png".into(), "zwei.png".into()],
        };
        assert_eq!(parse(&render(&session)), session);
    }

    #[test]
    fn a_fresh_file_says_nothing_about_consent() {
        // The difference that matters: `None` means nobody has asked yet and
        // this process should, `Some(false)` means somebody asked and the
        // answer was no.
        let session = Session {
            started: 7,
            ..Session::default()
        };
        let back = parse(&render(&session));
        assert_eq!(back.consent, None);
        assert!(back.done.is_empty());
        assert_eq!(back.started, 7);
    }

    #[test]
    fn a_refusal_is_written_down_as_such() {
        let session = Session {
            started: 7,
            consent: Some(false),
            done: Vec::new(),
        };
        assert!(render(&session).contains("consent no"));
        assert_eq!(parse(&render(&session)).consent, Some(false));
    }

    #[test]
    fn a_damaged_line_costs_a_line_and_not_the_run() {
        let raw = "started 42\nkaputt\n\ndone eins.png\nconsent vielleicht\ndone zwei.png\n";
        let session = parse(raw);
        assert_eq!(session.started, 42);
        assert_eq!(session.done, ["eins.png", "zwei.png"]);
        assert_eq!(
            session.consent,
            Some(false),
            "anything that is not a yes is not consent to send a file"
        );
    }

    #[test]
    fn nothing_at_all_is_an_empty_session_rather_than_an_error() {
        let session = parse("");
        assert_eq!(session.started, 0);
        assert_eq!(session.consent, None);
        assert!(session.done.is_empty());
    }

    #[test]
    fn a_line_break_in_a_name_cannot_forge_a_second_entry() {
        let session = Session {
            started: 1,
            consent: None,
            done: vec!["eins.png\ndone zwei.png".into()],
        };
        assert_eq!(
            parse(&render(&session)).done.len(),
            1,
            "a file name is not allowed to write the file"
        );
    }

    #[test]
    fn one_file_reads_exactly_as_it_did_before() {
        // The requirement in one test: a single run must not sprout a list or
        // a counter. It gets the sentence it always got.
        let mut session = Session::default();
        let body = collected(
            &mut session,
            "bild.ohne-meta.png".into(),
            r"Gespeichert: D:\Bilder\bild.ohne-meta.png (527029 Bytes)",
        );
        assert_eq!(
            body,
            r"Gespeichert: D:\Bilder\bild.ohne-meta.png (527029 Bytes)"
        );
    }

    #[test]
    fn six_files_are_six_names_under_each_other() {
        let mut session = Session::default();
        let mut body = String::new();
        for number in 1..=6 {
            body = collected(
                &mut session,
                format!("bild{number}.ohne-meta.png"),
                &format!(r"Gespeichert: D:\Bilder\bild{number}.ohne-meta.png (5 Bytes)"),
            );
        }

        assert_eq!(body.lines().count(), 6);
        assert_eq!(body.lines().next(), Some("bild1.ohne-meta.png"));
        assert_eq!(body.lines().last(), Some("bild6.ohne-meta.png"));
        assert!(
            !body.contains(r"D:\"),
            "the full path is the part that cannot be read there: {body}"
        );
        assert!(
            !body.contains("Bytes"),
            "and the byte count is not what six lines are for: {body}"
        );
    }

    #[test]
    fn the_same_sentence_six_times_stays_one_line() {
        // What a refusal looks like: all six processes come back with the same
        // "nothing was sent", and one notification saying it once is the
        // report, not six lines saying it six times.
        let mut session = Session::default();
        let mut body = String::new();
        for _ in 0..6 {
            body = collected(&mut session, "Nichts gesendet".into(), "Nichts gesendet");
        }
        assert_eq!(body, "Nichts gesendet");
        assert_eq!(session.done.len(), 1);
    }

    #[test]
    fn only_today_survives_the_sweep() {
        assert!(!from_another_day("abc-20321.txt", 20321));
        assert!(from_another_day("abc-20320.txt", 20321));
        assert!(
            !from_another_day("etwas-anderes", 20321),
            "the directory is this program's own, but only its own files go"
        );
    }

    #[test]
    fn a_fingerprint_is_a_name_a_mutex_and_a_file_can_both_wear() {
        let awkward = fingerprint(r#"a\b/c:*?"<>| .png"#);
        assert_eq!(awkward.len(), 16);
        assert!(awkward.chars().all(|c| c.is_ascii_hexdigit()));

        assert_eq!(fingerprint("a"), fingerprint("a"));
        assert_ne!(fingerprint("a"), fingerprint("b"));
        assert_ne!(
            fingerprint("snapotter__metadaten_entfernen"),
            fingerprint("snapotter__fuers_web_optimieren")
        );
    }

    #[test]
    fn the_two_locks_of_one_favourite_are_two_names() {
        // Sharing one would queue the five waiting processes behind a human
        // answering a dialog.
        let id = "snapotter__metadaten_entfernen";
        assert_ne!(file_lock(id), consent_lock(id));
        assert!(file_lock(id).starts_with(r"Local\"));
        assert!(!file_lock(id)[6..].contains('\\'), "{}", file_lock(id));
    }

    #[test]
    fn a_named_mutex_lets_exactly_one_holder_through() {
        // The property the whole module rests on, checked against the real
        // kernel object rather than assumed.
        let name = format!(r"Local\ctxmenu.test.{}", std::process::id());
        let first = Guard::take(&name, Duration::from_secs(1)).expect("the first one gets it");

        let name_for_thread = name.clone();
        let while_held = std::thread::spawn(move || {
            Guard::take(&name_for_thread, Duration::from_millis(200)).is_some()
        })
        .join()
        .expect("thread");
        assert!(!while_held, "a second holder must wait rather than pass");

        drop(first);
        let after =
            std::thread::spawn(move || Guard::take(&name, Duration::from_millis(500)).is_some())
                .join()
                .expect("thread");
        assert!(after, "and get it once the first one is done");
    }
}
