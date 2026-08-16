// Release builds start without a console window; debug builds keep it so the
// CLI and `println!` stay usable during development (ToDo 13.3).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use ctxmenu::{app, cli, console, elevation, errln, log, smoke, webtool};

fn main() -> ExitCode {
    // Before the first write: a GUI-subsystem binary starts without standard
    // handles, and attaching later would come too late (see console.rs).
    console::attach_to_parent();

    // Before anything that could fail. The release profile aborts on panic, so
    // this hook is the only moment at which one is still observable -- without
    // it the window simply disappears and the user has nothing to report.
    log::catch_panics();

    // The elevated job mode is intercepted before anything else: an elevated
    // instance must not open a second window (ToDo 13.2).
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some(elevation::JOB_ARG) {
        return match raw.get(1) {
            Some(job) => match elevation::run_job(std::path::Path::new(job)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    errln!("\x1eJob fehlgeschlagen\x1fjob failed\x1d: {error:#}");
                    console::flush();
                    ExitCode::FAILURE
                }
            },
            None => {
                errln!(
                    "{} \x1eerwartet eine Job-Datei\x1fexpects a job file\x1d",
                    elevation::JOB_ARG
                );
                console::flush();
                ExitCode::FAILURE
            }
        };
    }

    // A web tool favourite, started by a click in the Explorer menu. Also
    // intercepted before the argument parser, and for the same reason as the
    // job mode: this must never end up opening a window. There is no console
    // either, so everything it has to say goes into a message box.
    if raw.first().map(String::as_str) == Some(webtool::RUN_ARG) {
        let (Some(id), Some(file)) = (raw.get(1), raw.get(2)) else {
            webtool::shell::report(
                "ctxmenu",
                &format!(
                    "{} \x1eerwartet eine Kennung und eine Datei\x1fexpects an id and a file\x1d",
                    webtool::RUN_ARG
                ),
                webtool::shell::Report::Error,
            );
            return ExitCode::FAILURE;
        };

        return match webtool::run(id, std::path::Path::new(file)) {
            Ok(message) => {
                // Silence would be indistinguishable from a broken entry, and
                // the clipboard mode in particular needs to say what to do
                // next.
                webtool::shell::report("ctxmenu", &message, webtool::shell::Report::Info);
                ExitCode::SUCCESS
            }
            Err(error) => {
                // Logged as well as shown: this path runs from a context menu
                // click, with no console and no window behind the box, so the
                // message box is the only thing the user sees -- and it is gone
                // the moment they click it away.
                let message = ctxmenu::bilingual::error(&error, ctxmenu::bilingual::language());
                log::write(log::Kind::Error, &format!("--favourite {id}: {message}"));
                webtool::shell::report("ctxmenu", &message, webtool::shell::Report::Error);
                ExitCode::FAILURE
            }
        };
    }

    // Everything else is ordinary argument handling.
    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            errln!("{error}");
            console::flush();
            return ExitCode::FAILURE;
        }
    };

    let result = match command {
        cli::Command::Help => {
            ctxmenu::outln!("{}", cli::help());
            Ok(())
        }
        cli::Command::Version => {
            // Name and number, the shape every command line tool answers in —
            // and the first thing to ask when a report says "it does not work".
            ctxmenu::outln!("ctxmenu {}", ctxmenu::VERSION);
            Ok(())
        }
        cli::Command::Gui(start) => app::run(start).map_err(|e| anyhow::anyhow!("eframe: {e}")),
        cli::Command::Scan(args) => cli::run_scan(args),
        cli::Command::Programs => cli::run_programs(),
        cli::Command::FileType(ext) => cli::run_file_type(&ext),
        cli::Command::Apply {
            action,
            path,
            confirmed,
        } => cli::run_apply(action, &path, confirmed),
        cli::Command::Create(entry) => cli::run_create(&entry),
        cli::Command::Created => cli::run_created(),
        cli::Command::Favourite(what) => cli::run_favourite(what),
        cli::Command::Backups => cli::run_backups(),
        cli::Command::BackupAll => cli::run_backup_all(),
        cli::Command::Restore(directory) => cli::run_restore(&directory),
        cli::Command::Delete { path, confirmed } => cli::run_delete(&path, confirmed),
        cli::Command::Smoke => smoke::run().map_err(|e| anyhow::anyhow!("eframe: {e}")),
    };

    let code = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = ctxmenu::bilingual::error(&error, ctxmenu::bilingual::language());
            log::write(log::Kind::Error, &message);
            errln!("\x1eFehler\x1ferror\x1d: {message}");
            ExitCode::FAILURE
        }
    };

    console::flush();
    code
}
