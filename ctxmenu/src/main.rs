// Release builds start without a console window; debug builds keep it so the
// CLI and `println!` stay usable during development (ToDo 13.3).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod model;
mod registry;
mod smoke;

use std::process::ExitCode;

fn main() -> ExitCode {
    // Argument handling happens before any window is created. The elevated job
    // mode from ToDo 13.2 will hook in here for the same reason: an elevated
    // instance must not open a second window.
    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let result = match command {
        cli::Command::Help => {
            println!("{}", cli::HELP);
            Ok(())
        }
        cli::Command::Scan(args) => cli::run_scan(args),
        cli::Command::Smoke => smoke::run().map_err(|e| anyhow::anyhow!("eframe: {e}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Fehler / error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
