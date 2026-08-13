//! Making the command line usable from a GUI-subsystem binary.
//!
//! Release builds are linked as `windows_subsystem = "windows"` so that
//! double-clicking the app does not flash a console (ToDo 13.3). The side
//! effect is that the process starts with no standard handles at all, and the
//! first `println!` then fails — which in Rust means a panic, not a silent
//! drop. Measured on this machine before the fix:
//!
//! ```text
//! panicked at library\std\src\io\stdio.rs: failed printing to stdout:
//! Die Pipe wird gerade geschlossen. (os error 232)
//! ```

use std::io::Write;

/// Attaches to the console of the calling process, if there is one.
///
/// Creates no window: with no parent console — a double-click from Explorer —
/// this simply fails and the app stays silent, which is exactly the intent.
/// Must run before the first write, because that is when Rust caches the
/// standard handle.
pub fn attach_to_parent() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        // Failure is the normal case when started from Explorer.
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}

/// Prints a line, treating a failed write as nothing worth crashing over.
///
/// `println!` panics when stdout cannot be written — a closed pipe is enough,
/// which is what happens when output is piped into a command that exits early.
/// A diagnostic tool aborting because its output went nowhere is the wrong
/// trade every time.
pub fn line(text: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{text}");
}

/// Same for progress and error output.
pub fn err_line(text: &str) {
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = writeln!(lock, "{text}");
}

/// Flushes what is left, ignoring failures for the reason above.
pub fn flush() {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
}

/// `println!` that cannot bring the process down.
#[macro_export]
macro_rules! outln {
    () => { $crate::console::line("") };
    ($($arg:tt)*) => { $crate::console::line(&format!($($arg)*)) };
}

/// `eprintln!` that cannot bring the process down.
#[macro_export]
macro_rules! errln {
    ($($arg:tt)*) => { $crate::console::err_line(&format!($($arg)*)) };
}
