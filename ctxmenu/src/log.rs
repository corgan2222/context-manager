//! A single log file, for the errors nobody was there to read.
//!
//! Until now an error was a dialog and then nothing: the user closed it, and by
//! the time they thought of reporting it, the wording was gone. A panic was
//! worse — the release profile builds with `panic = "abort"`, so the window
//! vanishes without a trace.
//!
//! Deliberately not a logging framework. No levels, no targets, no filter, no
//! dependency: this file exists so that a bug report can carry the exact
//! sentence the program produced, and that needs an append and a timestamp.
//!
//! What is written, and nothing else:
//!
//! * every error the user was shown,
//! * every panic, with its location,
//! * the start of each run, so entries can be told apart.
//!
//! What is never written: the contents of a file, a key, or a header value. The
//! log is meant to be attachable to a public issue after a glance — it does name
//! registry paths and file names, which is why `docs/SECURITY.md` and
//! `docs/CONTRIBUTING.md` both say to look before sending.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

/// Above this the file is rotated to `ctxmenu.log.old`, which the next rotation
/// replaces. Two files, a bounded amount of disk, and enough history to cover
/// the session before the one that broke.
const MAX_BYTES: u64 = 256 * 1024;

/// Serialises writers within this process. Two threads reporting at once is
/// normal here — a worker thread finishing a scan while the frame path reports
/// something else.
static WRITING: Mutex<()> = Mutex::new(());

/// `%LOCALAPPDATA%\ctxmenu\ctxmenu.log`
///
/// Beside the settings and the backups rather than next to the executable: the
/// program is a single file that people run from a download folder, and a
/// program that writes into `Downloads` is a program that surprises people.
pub fn path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("ctxmenu").join("ctxmenu.log"))
}

/// The directory the log lives in, for a button that opens it.
pub fn directory() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("ctxmenu"))
}

/// Appends one line. Never fails loudly: a program that cannot write its log is
/// still a program that works, and an error box about the error box would be a
/// poor trade.
pub fn write(kind: Kind, message: &str) {
    let Some(path) = path() else { return };
    let _guard = WRITING.lock();

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate_if_large(&path);

    // One line per entry, however many lines the message has: a log where one
    // entry can span lines cannot be read with a filter, and an `anyhow` chain
    // arrives with newlines in it more often than not.
    let mut line = String::with_capacity(message.len() + 64);
    let _ = write!(
        line,
        "{} {} {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        kind.tag(),
        message.replace('\r', "").replace('\n', " \u{b7} ")
    );
    line.push('\n');

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// What kind of entry this is. Short tags, so the timestamp and the message are
/// what the eye lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The program started. Carries the version and how it was invoked.
    Start,
    /// An error that was shown to the user.
    Error,
    /// A panic. The program is about to end.
    Panic,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Start => "START",
            Kind::Error => "ERROR",
            Kind::Panic => "PANIC",
        }
    }
}

/// Notes the start of a run, so the entries below it have a context.
pub fn note_start(how: &str) {
    write(
        Kind::Start,
        &format!("ctxmenu {} \u{b7} {}", crate::VERSION, how),
    );
}

/// Sends panics to the log before the process ends.
///
/// Installed once, from `main`. With `panic = "abort"` in the release profile
/// there is no unwinding and no chance to catch anything later — the hook is
/// the only moment at which a panic is still observable, and without it the
/// window simply disappears.
pub fn catch_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let where_ = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".into());
        write(Kind::Panic, &format!("{where_} \u{b7} {info}"));
        previous(info);
    }));
}

/// Moves the file aside once it grows past [`MAX_BYTES`].
fn rotate_if_large(path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_BYTES {
        return;
    }
    let old = path.with_extension("log.old");
    // Rename over the previous rotation. If that fails -- the old file open in
    // an editor, say -- the current file simply keeps growing, which is a
    // better outcome than losing the entry that is about to be written.
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::rename(path, &old);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_sits_beside_the_settings_and_the_backups() {
        let path = path().expect("a machine has a local data directory");
        assert!(path.ends_with("ctxmenu\\ctxmenu.log"));
        assert_eq!(
            directory().unwrap(),
            path.parent().unwrap(),
            "the button that opens the folder opens the log's folder"
        );
    }

    #[test]
    fn an_entry_is_one_line_however_many_the_message_had() {
        // An anyhow chain arrives with newlines in it; a log where one entry
        // spans lines cannot be read with a filter.
        let message = "erste Zeile\r\nzweite Zeile\nditte";
        let folded = message.replace('\r', "").replace('\n', " \u{b7} ");
        assert!(!folded.contains('\n'));
        assert_eq!(folded, "erste Zeile \u{b7} zweite Zeile \u{b7} ditte");
    }

    #[test]
    fn every_kind_has_a_tag_and_they_are_all_different() {
        let tags = [Kind::Start.tag(), Kind::Error.tag(), Kind::Panic.tag()];
        assert!(tags.iter().all(|t| !t.is_empty()));
        assert_eq!(
            tags.iter().collect::<std::collections::HashSet<_>>().len(),
            tags.len()
        );
    }

    #[test]
    fn rotation_leaves_the_current_file_alone_while_it_is_small() {
        let dir = std::env::temp_dir().join("ctxmenu-log-test-small");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ctxmenu.log");
        std::fs::write(&path, b"kurz").unwrap();

        rotate_if_large(&path);

        assert!(path.exists(), "a small log is not rotated");
        assert!(!path.with_extension("log.old").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_past_its_limit_moves_aside_and_starts_over() {
        let dir = std::env::temp_dir().join("ctxmenu-log-test-large");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ctxmenu.log");
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();

        rotate_if_large(&path);

        assert!(!path.exists(), "the full log moved aside");
        assert!(
            path.with_extension("log.old").exists(),
            "and it is still there to read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
