// Release builds start without a console window; debug builds keep it so the
// CLI and `println!` stay usable during development (ToDo 13.3).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use ctxmenu::{app, cli, console, errln, smoke};

fn main() -> ExitCode {
    // Before the first write: a GUI-subsystem binary starts without standard
    // handles, and attaching later would come too late (see console.rs).
    console::attach_to_parent();

    // Argument handling happens before any window is created. The elevated job
    // mode from ToDo 13.2 will hook in here for the same reason: an elevated
    // instance must not open a second window.
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
        cli::Command::Gui { synthetic, bench } => {
            app::run(synthetic, bench).map_err(|e| anyhow::anyhow!("eframe: {e}"))
        }
        cli::Command::Scan(args) => cli::run_scan(args),
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
