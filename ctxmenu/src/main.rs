// Release builds start without a console window; debug builds keep it so the
// CLI and `println!` stay usable during development (ToDo 13.3).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use ctxmenu::{app, cli, console, elevation, errln, smoke};

fn main() -> ExitCode {
    // Before the first write: a GUI-subsystem binary starts without standard
    // handles, and attaching later would come too late (see console.rs).
    console::attach_to_parent();

    // The elevated job mode is intercepted before anything else: an elevated
    // instance must not open a second window (ToDo 13.2).
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some(elevation::JOB_ARG) {
        return match raw.get(1) {
            Some(job) => match elevation::run_job(std::path::Path::new(job)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    errln!("Job fehlgeschlagen / job failed: {error:#}");
                    console::flush();
                    ExitCode::FAILURE
                }
            },
            None => {
                errln!(
                    "{} erwartet eine Job-Datei / expects a job file",
                    elevation::JOB_ARG
                );
                console::flush();
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
            ctxmenu::outln!("{}", cli::HELP);
            Ok(())
        }
        cli::Command::Gui {
            synthetic,
            bench,
            tab,
        } => app::run(synthetic, bench, tab).map_err(|e| anyhow::anyhow!("eframe: {e}")),
        cli::Command::Scan(args) => cli::run_scan(args),
        cli::Command::Programs => cli::run_programs(),
        cli::Command::FileType(ext) => cli::run_file_type(&ext),
        cli::Command::Apply {
            action,
            path,
            confirmed,
        } => cli::run_apply(action, &path, confirmed),
        cli::Command::Backups => cli::run_backups(),
        cli::Command::Restore(directory) => cli::run_restore(&directory),
        cli::Command::Delete { path, confirmed } => cli::run_delete(&path, confirmed),
        cli::Command::Smoke => smoke::run().map_err(|e| anyhow::anyhow!("eframe: {e}")),
    };

    let code = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            errln!("Fehler / error: {error:#}");
            ExitCode::FAILURE
        }
    };

    console::flush();
    code
}
