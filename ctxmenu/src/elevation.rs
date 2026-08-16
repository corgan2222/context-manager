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
use serde::{Deserialize, Serialize};
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
use crate::settings::Language;

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
        return Outcome::Failed(
            "\x1eEigenen Pfad nicht ermittelbar\x1fcannot find own path\x1d".into(),
        );
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
            return Outcome::Failed("\x1eKein Prozess-Handle\x1fno process handle\x1d".into());
        }
        let child = Owned::new(info.hProcess);

        WaitForSingleObject(*child, INFINITE);

        let mut code = 0u32;
        if GetExitCodeProcess(*child, &mut code).is_err() {
            return Outcome::Failed("\x1eExitcode nicht lesbar\x1fexit code unreadable\x1d".into());
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
/// Two rules, and getting either wrong opens the user's Documents folder
/// instead — which is exactly what the first version of this did:
///
/// 1. **The quotes go around the path, not around the whole argument.**
///    `Command::arg` quotes an argument containing spaces as a unit, so
///    `explorer.exe "/select,C:\Program Files\…"` reaches Explorer, and
///    Explorer does not recognise that as a switch at all. `raw_arg` passes
///    the command line through untouched, and the quotes are placed by hand
///    where Explorer wants them: `/select,"C:\Program Files\…"`.
/// 2. **No space after the comma.** `/select, "…"` is read as two arguments.
///
/// Deliberately fire and forget: Explorer detaches immediately, and there is
/// nothing to wait for or report.
pub fn show_in_explorer(path: &Path) -> Result<()> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt as _;

    let argument = format!("/select,\"{}\"", path.display());
    let mut command = Command::new("explorer.exe");
    #[cfg(windows)]
    command.raw_arg(&argument);
    #[cfg(not(windows))]
    command.arg(&argument);

    command
        .spawn()
        .with_context(|| format!("\x1eExplorer für\x1fExplorer for\x1d {path:?}"))?;
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

    let output = command.output().with_context(|| {
        format!("{exe:?} \x1ekonnte nicht gestartet werden\x1fcould not be started\x1d")
    })?;

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
        .context("\x1eexplorer.exe konnte nicht gestartet werden\x1fcould not be started\x1d")?;
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

/// What a job file carries for the elevated child.
///
/// `#[serde(flatten)]` keeps `Plan`'s own fields at the top level of the
/// file, so a job file this same struct wrote is byte-for-byte what a plain
/// `Plan` expects — and, the other way round, a job file left over from a
/// build before `language` existed has none of these keys either, and
/// `#[serde(default)]` fills it in rather than refusing to load.
#[derive(Debug, Serialize, Deserialize)]
struct Job {
    #[serde(flatten)]
    plan: Plan,
    /// The language the window was showing when the job was written.
    ///
    /// The elevated child never opens a window and so never calls
    /// [`crate::bilingual::set_language`] itself; without this, its call to
    /// `backup::export` fell back to re-reading `settings.json` from disk,
    /// which can name a language the user already changed away from — the
    /// manifest note then permanently disagrees with the screen (todo 25).
    #[serde(default)]
    language: Language,
}

/// Writes a plan to a job file in the temp directory.
pub fn write_job(plan: &Plan) -> Result<PathBuf> {
    let name = format!(
        "ctxmenu_job_{}_{}.json",
        std::process::id(),
        chrono::Local::now().format("%Y%m%dT%H%M%S%3f")
    );
    let path = std::env::temp_dir().join(name);
    let job = Job {
        plan: plan.clone(),
        language: crate::bilingual::language(),
    };
    std::fs::write(&path, serde_json::to_string_pretty(&job)?)
        .with_context(|| format!("\x1eJob-Datei\x1fjob file\x1d {path:?}"))?;
    Ok(path)
}

/// Deletes the job and result files when dropped, whichever way the
/// enclosing function returns.
///
/// `run_elevated` used to remove both only at its very end, one statement
/// after the `?` and the `bail!` in its `Outcome::Finished` arm — both of
/// which return past that point, leaving the job file (the full plan: every
/// registry path and action) behind in `%TEMP%` on every such failure
/// (todo 19). A guard covers every return path, including ones added later.
struct JobFiles {
    job: PathBuf,
    result: PathBuf,
}

impl Drop for JobFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.job);
        let _ = std::fs::remove_file(&self.result);
    }
}

/// Parent side: hand the elevated half over and collect the report.
pub fn run_elevated(plan: &Plan) -> Result<Report> {
    let job = write_job(plan)?;
    let result = result_path(&job);
    let _cleanup = JobFiles {
        job: job.clone(),
        result: result.clone(),
    };

    let outcome = run_elevated_job(&job);
    collect_report(outcome, &result)
}

/// Turns the outcome of an elevated run into its report, or an error.
///
/// Split out from `run_elevated` so these branches can be exercised without
/// actually asking Windows for elevation.
fn collect_report(outcome: Outcome, result: &Path) -> Result<Report> {
    match outcome {
        Outcome::Cancelled => bail!("\x1eVom Benutzer abgebrochen\x1fcancelled by the user\x1d"),
        Outcome::Failed(message) => bail!(
            "\x1eStart mit Administratorrechten fehlgeschlagen\x1felevation failed\x1d: {message}"
        ),
        Outcome::Finished(_) => match std::fs::read_to_string(result) {
            Ok(raw) => serde_json::from_str(&raw)
                .context("\x1eErgebnisdatei unlesbar\x1fresult file unreadable\x1d"),
            Err(_) => bail!(
                "\x1eDer erhöhte Vorgang hat keinen Bericht hinterlasse\
                 n\x1fthe elevated run left no report\x1d"
            ),
        },
    }
}

/// Child side: run one job file and leave the report next to it.
///
/// Refuses to run unelevated. That is the loop breaker: without it, a child
/// that somehow started without elevation would find the same work still
/// undone and ask for elevation again.
pub fn run_job(job: &Path) -> Result<()> {
    if !is_elevated() {
        bail!("\x1eJob-Modus ohne Administratorrechte\x1fjob mode without elevation\x1d");
    }

    let raw = std::fs::read_to_string(job)
        .with_context(|| format!("\x1eJob-Datei\x1fjob file\x1d {job:?}"))?;
    let parsed: Job =
        serde_json::from_str(&raw).context("\x1eJob-Datei unlesbar\x1fjob file unreadable\x1d")?;

    // This process never opens a window, so nothing else would ever call
    // `set_language` -- without this, `execute` (by way of `backup::export`)
    // wrote the manifest note in whatever language `settings.json` happened
    // to hold, which can already disagree with the one shown on screen.
    crate::bilingual::set_language(parsed.language);

    let report = execute(&parsed.plan)?;
    std::fs::write(result_path(job), serde_json::to_string_pretty(&report)?)
        .context("\x1eErgebnisdatei\x1fresult file\x1d")?;

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

    /// Regression for todo 19: a partial return out of `run_elevated` --
    /// the `bail!` and the `?` in its `Outcome::Finished` arm both did this
    /// -- used to skip the cleanup at the bottom of the function and leave
    /// both files behind in `%TEMP%` forever.
    #[test]
    fn dropping_the_job_guard_removes_both_files() {
        let dir = std::env::temp_dir();
        let job = dir.join(format!("ctxmenu_test_job_{}.json", std::process::id()));
        let result = result_path(&job);
        std::fs::write(&job, "{}").expect("writable temp directory");
        std::fs::write(&result, "{}").expect("writable temp directory");

        {
            let _guard = JobFiles {
                job: job.clone(),
                result: result.clone(),
            };
        }

        assert!(!job.exists(), "job file survived the guard");
        assert!(!result.exists(), "result file survived the guard");
    }

    #[test]
    fn collect_report_refuses_a_cancelled_prompt_without_touching_any_file() {
        let error = collect_report(Outcome::Cancelled, Path::new(r"C:\gibt\es\nicht.json"))
            .expect_err("a declined prompt is not a report");
        assert!(format!("{error}").contains("abgebrochen"));
    }

    #[test]
    fn collect_report_names_the_failure_when_elevation_itself_failed() {
        let error = collect_report(
            Outcome::Failed("kein Handle".into()),
            Path::new(r"C:\gibt\es\nicht.json"),
        )
        .expect_err("a failed elevation is not a report");
        assert!(format!("{error}").contains("kein Handle"));
    }

    #[test]
    fn collect_report_complains_when_a_finished_run_left_no_result_file() {
        let error = collect_report(Outcome::Finished(0), Path::new(r"C:\gibt\es\nicht.json"))
            .expect_err("a missing result file is not a report");
        assert!(format!("{error}").contains("keinen Bericht"));
    }

    #[test]
    fn collect_report_reads_the_result_file_a_finished_run_left_behind() {
        let path = std::env::temp_dir().join(format!(
            "ctxmenu_test_result_{}_{}.json",
            std::process::id(),
            line!()
        ));
        let report = Report {
            backup_directories: vec!["irgendwo".into()],
            results: Vec::new(),
        };
        std::fs::write(&path, serde_json::to_string(&report).unwrap()).unwrap();

        let back =
            collect_report(Outcome::Finished(0), &path).expect("a written report reads back");
        assert_eq!(back.backup_directories, vec!["irgendwo".to_string()]);

        let _ = std::fs::remove_file(&path);
    }

    /// Regression for todo 25: the elevated child never calls
    /// `bilingual::set_language`, so it needs the language handed to it
    /// through the job file instead of guessing from `settings.json`.
    #[test]
    fn the_job_file_carries_the_current_language() {
        let before = crate::bilingual::language();
        crate::bilingual::set_language(Language::English);

        let plan = Plan::new("test", Vec::new());
        let path = write_job(&plan).expect("writable temp directory");
        let raw = std::fs::read_to_string(&path).expect("readable");
        let job: Job = serde_json::from_str(&raw).expect("valid json");
        assert_eq!(job.language, Language::English);

        let _ = std::fs::remove_file(&path);
        crate::bilingual::set_language(before);
    }

    /// A job file written before this field existed has no `language` key
    /// at all -- `#[serde(default)]` must load it anyway rather than turn a
    /// leftover file from an interrupted run into a parse error.
    #[test]
    fn an_older_job_file_without_a_language_field_still_loads() {
        let raw = serde_json::to_string(&Plan::new("test", Vec::new())).unwrap();
        assert!(
            !raw.contains("language"),
            "a bare Plan must not itself mention language: {raw}"
        );

        let job: Job = serde_json::from_str(&raw).expect("an old job file must still load");
        assert_eq!(job.plan.label, "test");
        assert_eq!(job.language, Language::German, "serde's own default");
    }
}
