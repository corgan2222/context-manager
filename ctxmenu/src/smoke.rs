//! Minimal eframe window used as the setup smoke test.
//!
//! It exists to prove the toolchain end to end — linker, `rc.exe`, icon,
//! manifest, OpenGL context, system theme — before any registry code is
//! blamed for a setup problem. Milestone 4 replaces it with the real UI.
//!
//! Reachable in the shipped binary via `ctxmenu --smoke` (see `cli.rs`,
//! documented in both language sections of `--help`), so its texts go
//! through [`crate::bilingual`] like console output does — not the full
//! `Strings` table from `i18n.rs`, which belongs to the main window and
//! needs a `Settings` this standalone window never loads.

use crate::bilingual;

struct SmokeApp {
    started: std::time::Instant,
    reported: bool,
}

impl eframe::App for SmokeApp {
    // eframe 0.36 replaced `update(&mut self, ctx, frame)` with `ui(&mut self,
    // ui, frame)`, and panels now take a `&mut Ui` instead of a `&Context`.
    // Examples written against 0.31 still show the old shape.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Reported once to stderr as well, so the theme part of the setup
        // check can be verified from a script instead of by squinting at the
        // window. Release builds have no console, hence stderr and not stdout.
        if !self.reported {
            self.reported = true;
            eprintln!(
                "smoke: system_theme={:?} active_theme={:?} dark_mode={}",
                ui.ctx().system_theme(),
                ui.ctx().theme(),
                ui.visuals().dark_mode
            );
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(window_title());
            ui.separator();

            let theme = if ui.visuals().dark_mode {
                bilingual::shown("\x1edunkel\x1fdark\x1d")
            } else {
                bilingual::shown("\x1ehell\x1flight\x1d")
            };
            ui.label(format!(
                "{}: {theme}",
                bilingual::shown("\x1eErkanntes Theme\x1fDetected theme\x1d")
            ));
            ui.label(format!(
                "{}: {:?}",
                bilingual::shown("\x1eSystemtheme laut eframe\x1fSystem theme per eframe\x1d"),
                ui.ctx().system_theme()
            ));
            ui.label(format!(
                "{}: {:?}",
                bilingual::shown("\x1eAktives Theme\x1fActive theme\x1d"),
                ui.ctx().theme()
            ));
            ui.label(format!(
                "{}: {:.1} s",
                bilingual::shown("\x1eLaufzeit\x1fRuntime\x1d"),
                self.started.elapsed().as_secs_f32()
            ));

            ui.add_space(8.0);
            ui.label(
                bilingual::shown(
                    "\x1eErwartet: Fenster erscheint, Theme folgt der Windows-Einstellung, \
                     im Release-Build keine Konsole.\
                     \x1fExpected: window appears, theme follows the Windows setting, no \
                     console in the release build.\x1d",
                )
                .into_owned(),
            );
        });
    }
}

/// `"ctxmenu"` stays as is — it is the command name, not prose — the rest is
/// picked for the language this process shows text in, same as everywhere
/// else that has no `Settings` at hand (see [`bilingual::shown`]).
fn window_title() -> String {
    format!(
        "ctxmenu — {}",
        bilingual::shown("\x1eSmoke-Test\x1fsmoke test\x1d")
    )
}

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 320.0])
            .with_min_inner_size([400.0, 240.0]),
        ..Default::default()
    };

    eframe::run_native(
        &window_title(),
        options,
        Box::new(|cc| {
            // Explicit: the three-way theme choice. Whether `System`
            // actually follows a live theme switch on this Windows build is
            // what the smoke test is meant to reveal.
            cc.egui_ctx.set_theme(egui::ThemePreference::System);
            Ok(Box::new(SmokeApp {
                started: std::time::Instant::now(),
                reported: false,
            }))
        }),
    )
}
