//! Doing the part of the work that needs administrator rights.
//!
//! The manifest says `asInvoker`, so the window itself never asks for
//! elevation. Only when a plan turns out to touch keys this process cannot
//! write does it hand that half to a second instance of itself, started with
//! `runas` (ToDo 13.2).
//!
//! The child gets a job file, writes a result file next to it and exits. The
//! parent waits for it and reads the result back, so a partial failure is
//! reported rather than guessed at.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
// OpenProcessToken lives in System::Threading, not Security, even though it
// is gated on the Win32_Security feature.
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify,
    SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SW_HIDE};
use windows::core::{Owned, PCWSTR, w};

use crate::registry::plan::{Plan, Report, execute};

/// Keeps a started console tool from flashing a window. Same constant and same
/// reason as in `registry::backup`.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The argument that puts a started instance into job mode.
///
/// An explicit marker, and the child checks that it really is elevated before
/// doing anything: without that, a child that failed to elevate for an
/// unexpected reason would ask for elevation again, and again.
pub const JOB_ARG: &str = "--apply-job";

/// What came of asking for elevation.
#[derive(Debug)]
pub enum Outcome {
    /// The child ran and exited with this code.
    Finished(u32),
    /// The user declined the UAC prompt. A decision, not an error (ToDo 13.2).
    Cancelled,
    Failed(String),
}

/// Is this process running with an elevated token?
pub fn is_elevated() -> bool {
    unsafe {
        let mut raw = HANDLE::default();
        // GetCurrentProcess returns the pseudo-handle -1. It is not a kernel
        // object, so it must never be closed — only the token below is owned.
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw).is_err() {
            return false;
        }
        let token = Owned::new(raw);

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            *token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok();

        // The field is a u32 flag, not a bool.
        ok && elevation.TokenIsElevated != 0
    }
}

/// Restarts this executable elevated to run one job file, and waits for it.
///
/// Waiting is why this uses `ShellExecuteExW` rather than the simpler
/// `ShellExecuteW`: the latter hands back no process handle at all, so the
/// parent could never learn what happened.
pub fn run_elevated_job(job: &Path) -> Outcome {
    let Ok(exe) = std::env::current_exe() else {
        return Outcome::Failed("Eigenen Pfad nicht ermittelbar / cannot find own path".into());
    };
    let directory = exe.parent().map(Path::to_path_buf).unwrap_or_default();

    // lpParameters is one command line, not an argument list. A job path under
    // `C:\Users\Vor Name\…` would otherwise split into two arguments and the
    // child would open nothing.
    let parameters = format!("{JOB_ARG} \"{}\"", job.display());

    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(&parameters);
    // Without an explicit directory the child starts in system32, where a
    // relative path in the job would resolve somewhere it must not.
    let dir = wide(&directory.to_string_lossy());

    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // NOCLOSEPROCESS hands back hProcess so the parent can wait;
        // NOASYNC keeps the call from returning before the child exists.
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    unsafe {
        if let Err(error) = ShellExecuteExW(&mut info) {
            // 1223 is ERROR_CANCELLED: the user clicked no. That is an answer,
            // not a fault, and must not be reported as one.
            return if error.code().0 as u32 & 0xFFFF == 1223 {
                Outcome::Cancelled
            } else {
                Outcome::Failed(format!("{error}"))
            };
        }

        if info.hProcess.is_invalid() {
            return Outcome::Failed("Kein Prozess-Handle / no process handle".into());
        }
        let child = Owned::new(info.hProcess);

        WaitForSingleObject(*child, INFINITE);

        let mut code = 0u32;
        if GetExitCodeProcess(*child, &mut code).is_err() {
            return Outcome::Failed("Exitcode nicht lesbar / exit code unreadable".into());
        }
        Outcome::Finished(code)
    }
}

/// Tells the shell that associations changed.
///
/// Deliberately called from the *unelevated* parent: a notification sent by
/// the elevated child reaches the elevated session, not the Explorer the user
/// is looking at. It also cannot report failure — the function returns
/// nothing — so nobody should treat it as proof the menu updated. Changes to
/// COM handlers need an Explorer restart regardless (ToDo 13.4).
pub fn notify_shell() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Opens an Explorer window with this file selected.
///
/// `/select,` rather than opening the folder plain: the question behind the
/// button is "which file is this", and a folder with two hundred entries and
/// no selection does not answer it.
///
/// The path is passed as one argument, not as a command line, so a program
/// under `C:\Program Files\…` needs no quoting of ours — `Command` does it.
/// Deliberately fire and forget: Explorer detaches immediately, and there is
/// nothing to wait for or report.
pub fn show_in_explorer(path: &Path) -> Result<()> {
    // The parameter really is one string with a comma in it. `/select, "path"`
    // with a space after the comma opens the wrong thing — Explorer treats the
    // space as the start of a second argument.
    let argument = format!("/select,{}", path.display());
    Command::new("explorer.exe")
        .arg(argument)
        .spawn()
        .with_context(|| format!("Explorer für {path:?} / for {path:?}"))?;
    Ok(())
}

/// Restarts the shell, so a changed COM handler is actually gone.
///
/// `notify_shell` above is the polite version and enough for a static verb. A
/// COM handler is a DLL that Explorer loaded into its own process long ago;
/// no notification unloads it, which is why every tool in this corner ends up
/// reaching for the same blunt instrument. Until now this one only *said* so
/// and left the doing to the user (ToDo 13.4).
///
/// Closes every open Explorer window. The button that calls this says so.
pub fn restart_explorer() -> Result<()> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt as _;

    let exe = std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("taskkill.exe");

    // `taskkill` rather than looking the process up and terminating it here:
    // one call, no PROCESS_TERMINATE right of our own to arrange, and it
    // covers the several-Explorer-processes case ("launch folder windows in a
    // separate process") without enumerating anything.
    let mut command = Command::new(&exe);
    command.args(["/f", "/im", "explorer.exe"]);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command
        .output()
        .with_context(|| format!("{exe:?} konnte nicht gestartet werden / could not be started"))?;

    // Exit code 128 is "no such process" — the shell was already down, which
    // is not a reason to stop before starting it again.
    if !output.status.success() && output.status.code() != Some(128) {
        let said = String::from_utf8_lossy(&output.stdout);
        let complained = String::from_utf8_lossy(&output.stderr);
        let message = match complained.trim().is_empty() {
            true => said.trim().to_string(),
            false => complained.trim().to_string(),
        };
        bail!("taskkill: {message}");
    }

    if wait_for_shell(SHELL_RESTART_WAIT) {
        return Ok(());
    }

    // Only now, and only if Windows did not do it itself. `AutoRestartShell`
    // is on by default, and starting a second Explorer while the shell is
    // already back does not restart anything — it opens a stray folder window
    // in the user's face.
    Command::new("explorer.exe")
        .spawn()
        .context("explorer.exe konnte nicht gestartet werden / could not be started")?;
    Ok(())
}

/// How long to give Windows to bring the shell back on its own.
///
/// Measured on this machine: the taskbar reappears in well under a second.
/// The cap exists for the case where `AutoRestartShell` is off, where waiting
/// longer would only delay the manual start that is then needed anyway.
const SHELL_RESTART_WAIT: std::time::Duration = std::time::Duration::from_millis(2500);

/// Waits for the taskbar window to exist again.
///
/// `Shell_TrayWnd` is the shell's own class and outlives any single Explorer
/// window, so its presence answers "is there a shell?" rather than "is a
/// folder open?". Polled rather than hooked: this runs once, on a button.
fn wait_for_shell(limit: std::time::Duration) -> bool {
    let step = std::time::Duration::from_millis(100);
    let mut waited = std::time::Duration::ZERO;

    while waited < limit {
        std::thread::sleep(step);
        waited += step;
        let found = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) };
        if found.is_ok() {
            return true;
        }
    }
    false
}

/// Where the child writes its report.
fn result_path(job: &Path) -> PathBuf {
    job.with_extension("result.json")
}

/// Writes a plan to a job file in the temp directory.
pub fn write_job(plan: &Plan) -> Result<PathBuf> {
    let name = format!(
        "ctxmenu_job_{}_{}.json",
        std::process::id(),
        chrono::Local::now().format("%Y%m%dT%H%M%S%3f")
    );
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, serde_json::to_string_pretty(plan)?)
        .with_context(|| format!("Job-Datei / job file {path:?}"))?;
    Ok(path)
}

/// Parent side: hand the elevated half over and collect the report.
pub fn run_elevated(plan: &Plan) -> Result<Report> {
    let job = write_job(plan)?;
    let outcome = run_elevated_job(&job);
    let result = result_path(&job);

    let report = match outcome {
        Outcome::Cancelled => {
            let _ = std::fs::remove_file(&job);
            bail!("Vom Benutzer abgebrochen / cancelled by the user");
        }
        Outcome::Failed(message) => {
            let _ = std::fs::remove_file(&job);
            bail!("Start mit Administratorrechten fehlgeschlagen / elevation failed: {message}");
        }
        Outcome::Finished(_) => match std::fs::read_to_string(&result) {
            Ok(raw) => serde_json::from_str(&raw)
                .context("Ergebnisdatei unlesbar / result file unreadable")?,
            Err(_) => bail!(
                "Der erhöhte Vorgang hat keinen Bericht hinterlassen / \
                 the elevated run left no report"
            ),
        },
    };

    let _ = std::fs::remove_file(&job);
    let _ = std::fs::remove_file(&result);
    Ok(report)
}

/// Child side: run one job file and leave the report next to it.
///
/// Refuses to run unelevated. That is the loop breaker: without it, a child
/// that somehow started without elevation would find the same work still
/// undone and ask for elevation again.
pub fn run_job(job: &Path) -> Result<()> {
    if !is_elevated() {
        bail!("Job-Modus ohne Administratorrechte / job mode without elevation");
    }

    let raw =
        std::fs::read_to_string(job).with_context(|| format!("Job-Datei / job file {job:?}"))?;
    let plan: Plan =
        serde_json::from_str(&raw).context("Job-Datei unlesbar / job file unreadable")?;

    let report = execute(&plan)?;
    std::fs::write(result_path(job), serde_json::to_string_pretty(&report)?)
        .context("Ergebnisdatei / result file")?;

    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_elevation_check_answers_without_panicking() {
        // Which answer depends on how the tests were started; what matters is
        // that it resolves and stays stable within one process.
        let first = is_elevated();
        assert_eq!(first, is_elevated());
    }

    #[test]
    fn a_job_file_round_trips_through_json() {
        let plan = Plan::new("test", Vec::new());
        let path = write_job(&plan).expect("writable temp directory");

        let raw = std::fs::read_to_string(&path).expect("readable");
        let back: Plan = serde_json::from_str(&raw).expect("valid json");
        assert_eq!(back.label, "test");
        assert!(back.operations.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_result_file_sits_next_to_the_job() {
        let job = Path::new(r"C:\temp\ctxmenu_job_1.json");
        assert_eq!(
            result_path(job),
            Path::new(r"C:\temp\ctxmenu_job_1.result.json")
        );
    }

    #[test]
    fn job_mode_refuses_to_run_without_elevation() {
        if is_elevated() {
            // Cannot exercise the guard from an elevated test run.
            return;
        }
        let error = run_job(Path::new(r"C:\gibt\es\nicht.json"))
            .expect_err("must refuse before even reading the file");
        assert!(
            format!("{error}").contains("Administratorrechte"),
            "unexpected error: {error}"
        );
    }
}
