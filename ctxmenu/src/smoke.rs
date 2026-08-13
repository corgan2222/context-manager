//! Minimal eframe window used as the setup smoke test from ToDo 2.11.
//!
//! It exists to prove the toolchain end to end — linker, `rc.exe`, icon,
//! manifest, OpenGL context, system theme — before any registry code is
//! blamed for a setup problem. Milestone 4 replaces it with the real UI.

struct SmokeApp {
    started: std::time::Instant,
    reported: bool,
}

impl eframe::App for SmokeApp {
    // eframe 0.36 replaced `update(&mut self, ctx, frame)` with `ui(&mut self,
    // ui, frame)`, and panels now take a `&mut Ui` instead of a `&Context`.
    // The ToDo was written against 0.31 and still shows the old shape.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Reported once to stderr as well, so point 4 of the ToDo 2.11 check
        // list can be verified from a script instead of by squinting at the
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
            ui.heading("ctxmenu — Smoke-Test");
            ui.separator();

            let theme = if ui.visuals().dark_mode {
                "dunkel"
            } else {
                "hell"
            };
            ui.label(format!("Erkanntes Theme: {theme}"));
            ui.label(format!(
                "Systemtheme laut eframe: {:?}",
                ui.ctx().system_theme()
            ));
            ui.label(format!("Aktives Theme: {:?}", ui.ctx().theme()));
            ui.label(format!(
                "Laufzeit: {:.1} s",
                self.started.elapsed().as_secs_f32()
            ));

            ui.add_space(8.0);
            ui.label(
                "Erwartet: Fenster erscheint, Theme folgt der Windows-Einstellung, \
                 im Release-Build keine Konsole.",
            );
        });
    }
}

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 320.0])
            .with_min_inner_size([400.0, 240.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ctxmenu — Smoke-Test",
        options,
        Box::new(|cc| {
            // Explicit: the three-way choice from ToDo 9.1. Whether `System`
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
