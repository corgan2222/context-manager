//! The application window.
//!
//! Immediate mode means this whole file runs many times per second, so the
//! rule from ToDo 4.3 governs everything here: no registry access, no icon
//! extraction, no version resource lookup in the frame path. Anything costly
//! is precomputed and kept in the state, and anything slow runs on a thread.

use std::sync::mpsc::{Receiver, channel};

use egui::{Sense, Ui};
use egui_extras::{Column, TableBuilder};
use windows::Win32::Foundation::HWND;

use crate::i18n::{self, Strings};
use crate::icons::cache::IconCache;
use crate::model::{Category, ContextEntry, EntryKind, ScanProgress, ScanResult};
use crate::program::group::{self, ProgramGroup};
use crate::program::identity::NameResolver;
use crate::registry::backup::{self, BackupManifest};
use crate::registry::scan::{self, ScanOptions};
use crate::settings::{Language, Settings, ThemeChoice};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Categories,
    FileTypes,
    Programs,
    Backups,
}

impl Tab {
    /// Lets a start-up tab be named on the command line, which is how the
    /// window gets photographed on a tab other than the first one.
    pub fn from_slug(value: &str) -> Option<Tab> {
        match value.to_ascii_lowercase().as_str() {
            "categories" | "kategorien" => Some(Tab::Categories),
            "filetypes" | "dateitypen" => Some(Tab::FileTypes),
            "programs" | "programme" => Some(Tab::Programs),
            "backups" | "sicherungen" => Some(Tab::Backups),
            _ => None,
        }
    }
}

/// What the scan thread reports back.
enum ScanMessage {
    Progress(ScanProgress),
    /// Boxed because a full result is far larger than a progress report, and
    /// the channel sizes itself after its largest variant.
    Done(Box<ScanResult>),
}

pub struct App {
    scan: Option<ScanResult>,

    /// Indices into `scan.entries` that the table should draw.
    ///
    /// The single most important field for performance: filter, search and
    /// sorting are evaluated here once, not per frame (ToDo 4.3).
    visible_rows: Vec<usize>,
    filter_dirty: bool,

    tab: Tab,
    selected_category: Option<Category>,
    /// Selected extension in the file type tab.
    selected_ext: Option<String>,
    /// Index into `groups` for the program tab.
    selected_group: Option<usize>,
    /// Built once after each scan; never in the frame path.
    groups: Vec<ProgramGroup>,
    selection: Option<usize>,
    search: String,

    scan_rx: Option<Receiver<ScanMessage>>,
    scanning: bool,
    progress: (usize, usize),
    progress_label: String,

    backups: Vec<(std::path::PathBuf, BackupManifest)>,
    backup_error: Option<String>,

    icons: IconCache,
    tr: &'static Strings,
    settings: Settings,

    /// Kept so the title bar can follow later theme switches.
    hwnd: Option<HWND>,
    /// Last dark-mode state pushed to DWM, so the call happens on change only.
    titlebar_dark: Option<bool>,
    titlebar_supported: bool,

    frame_times: FrameTimes,
    bench: Option<Bench>,
    theme_reported: bool,
}

/// Drives the window at full speed for a fixed number of frames, scrolling as
/// it goes, then reports and closes.
///
/// Exists because the milestone 4 target — 60 fps at 2.000 rows — is otherwise
/// only checkable by looking at the window and believing it. Scrolling is part
/// of it: a virtualized table is cheap precisely because it rebuilds only the
/// visible rows, and that rebuild is what has to stay cheap.
struct Bench {
    warmup: usize,
    remaining: usize,
    scroll: usize,
}

impl App {
    /// `synthetic` replaces the registry scan with generated entries, so the
    /// table can be measured at the 2.000 rows milestone 4 asks for even
    /// though this machine's registry only holds a fraction of that.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        synthetic: Option<usize>,
        bench_frames: Option<usize>,
        start_tab: Tab,
    ) -> Self {
        install_fonts(&cc.egui_ctx);

        let settings = Settings::load_or_default(theme::system_language());
        cc.egui_ctx.set_theme(settings.theme.to_preference());

        let tr = strings_for(settings.language);
        let hwnd = theme::window_handle(cc);

        let mut app = Self {
            scan: None,
            visible_rows: Vec::new(),
            filter_dirty: true,
            tab: start_tab,
            selected_category: None,
            selected_ext: None,
            selected_group: None,
            groups: Vec::new(),
            selection: None,
            search: String::new(),
            scan_rx: None,
            scanning: false,
            progress: (0, 0),
            progress_label: String::new(),
            backups: Vec::new(),
            backup_error: None,
            icons: IconCache::new(&cc.egui_ctx),
            tr,
            settings,
            hwnd,
            titlebar_dark: None,
            titlebar_supported: true,
            frame_times: FrameTimes::default(),
            theme_reported: false,
            bench: bench_frames.map(|frames| Bench {
                // The first frames pay for fonts, textures and window setup;
                // measuring those would flatter or slander the result.
                warmup: 120,
                remaining: frames,
                scroll: 0,
            }),
        };

        match synthetic {
            Some(count) => {
                app.scan = Some(crate::synthetic::scan_result(count));
                app.filter_dirty = true;
            }
            None => app.start_scan(&cc.egui_ctx),
        }

        app.reload_backups();
        app
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        if self.scanning {
            return;
        }

        let (tx, rx) = channel();
        let ctx = ctx.clone();

        std::thread::Builder::new()
            .name("registry-scan".into())
            .spawn(move || {
                let options = ScanOptions::with_curated_file_types();
                let sender = tx.clone();
                let progress_ctx = ctx.clone();

                let result = scan::scan(&options, move |progress| {
                    let _ = sender.send(ScanMessage::Progress(progress));
                    // egui sleeps until something happens; without this the
                    // list would only fill on the next mouse move.
                    progress_ctx.request_repaint();
                });

                let _ = tx.send(ScanMessage::Done(Box::new(result)));
                ctx.request_repaint();
            })
            .expect("scan thread");

        self.scan_rx = Some(rx);
        self.scanning = true;
        self.selection = None;
    }

    /// Drains the scan channel. Never blocks.
    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };

        let mut finished = false;
        for message in rx.try_iter() {
            match message {
                ScanMessage::Progress(progress) => {
                    self.progress = (progress.done, progress.total);
                    self.progress_label = progress.label;
                }
                ScanMessage::Done(result) => {
                    let result = *result;
                    // Version resource lookups hit the disk, so the grouping
                    // is built once here and never per frame.
                    let mut names = NameResolver::new();
                    self.groups = group::build(&result, &mut names);
                    self.scan = Some(result);
                    self.filter_dirty = true;
                    finished = true;
                }
            }
        }

        if finished {
            self.scanning = false;
            self.scan_rx = None;
        }
    }

    fn reload_backups(&mut self) {
        match backup::list() {
            Ok(list) => {
                self.backups = list;
                self.backup_error = None;
            }
            Err(error) => {
                self.backups.clear();
                self.backup_error = Some(format!("{error:#}"));
            }
        }
    }

    /// Rebuilds `visible_rows`. Runs on change only, never per frame.
    ///
    /// Which entries are candidates depends on the tab; the search then
    /// narrows that set. Keeping both steps here means the table itself never
    /// filters anything.
    fn rebuild_visible(&mut self) {
        self.visible_rows.clear();
        let Some(scan) = &self.scan else { return };

        let candidates: Vec<usize> = match self.tab {
            // Base categories only. Without the second condition, "no
            // category selected" would also pour in every file type entry
            // and the count next to the tree would be a different number
            // from the sum of its children.
            Tab::Categories => scan
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| match &self.selected_category {
                    Some(category) => &e.category == category,
                    None => Category::BASE.contains(&e.category),
                })
                .map(|(i, _)| i)
                .collect(),

            Tab::FileTypes => match &self.selected_ext {
                Some(ext) => {
                    // Levels 1 and 2 apply to every file, so they belong in
                    // the list for this type even though they are not stored
                    // under it (ToDo 10.4). Showing only levels 3 to 7 would
                    // understate what a right-click actually offers.
                    let mut rows: Vec<usize> = scan
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| {
                            matches!(
                                e.category,
                                Category::AllFiles | Category::AllFilesystemObjects
                            )
                        })
                        .map(|(i, _)| i)
                        .collect();
                    if let Some(info) = scan.file_types.iter().find(|f| f.ext() == ext) {
                        rows.extend(info.entry_indices.iter().copied());
                    }
                    rows
                }
                None => Vec::new(),
            },

            Tab::Programs => self
                .selected_group
                .and_then(|i| self.groups.get(i))
                .map(|g| g.entry_indices.clone())
                .unwrap_or_default(),

            Tab::Backups => Vec::new(),
        };

        let needle = self.search.trim().to_lowercase();
        for index in candidates {
            let entry = &scan.entries[index];
            if !needle.is_empty() && !matches_search(entry, &needle) {
                continue;
            }
            self.visible_rows.push(index);
        }
    }

    /// Keeps the DWM title bar in step with the interface.
    fn sync_titlebar(&mut self, ui: &Ui) {
        let dark = ui.visuals().dark_mode;
        if self.titlebar_dark == Some(dark) {
            return;
        }

        if let Some(hwnd) = self.hwnd {
            self.titlebar_supported = theme::set_titlebar_dark(hwnd, dark);
            self.titlebar_dark = Some(dark);
        }
    }

    /// Writes what the theme actually resolved to, once, to stderr.
    ///
    /// The three-way choice is the kind of thing that looks right until it
    /// silently is not, and a screenshot cannot tell "light theme chosen" from
    /// "dark theme not detected".
    fn report_theme_once(&mut self, ui: &Ui) {
        if self.theme_reported {
            return;
        }
        self.theme_reported = true;

        crate::errln!(
            "theme: setting={:?} system={:?} active={:?} dark_mode={} titlebar_applied={:?}",
            self.settings.theme,
            ui.ctx().system_theme(),
            ui.ctx().theme(),
            ui.visuals().dark_mode,
            self.titlebar_dark
        );
        crate::console::flush();
    }

    /// Advances the benchmark, if one is running.
    fn drive_bench(&mut self, ctx: &egui::Context) {
        let Some(bench) = &mut self.bench else { return };

        // egui idles until something happens, so a benchmark has to keep
        // asking for frames or it would measure the sleep.
        ctx.request_repaint();

        if bench.warmup > 0 {
            bench.warmup -= 1;
            if bench.warmup == 0 {
                // Discard the setup frames.
                self.frame_times = FrameTimes::default();
            }
            return;
        }

        bench.scroll += 7;
        bench.remaining -= 1;

        if bench.remaining == 0 {
            let rows = self.visible_rows.len();
            crate::errln!(
                "bench: rows={rows} frames_measured={} avg={:.3}ms p95={:.3}ms worst={:.3}ms fps={:.1}",
                self.frame_times.count(),
                self.frame_times.average_ms(),
                self.frame_times.percentile_ms(0.95),
                self.frame_times.worst_ms(),
                self.frame_times.fps()
            );
            crate::console::flush();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Entries in the base categories — what the category tree covers.
    fn entry_count(&self) -> usize {
        self.scan.as_ref().map_or(0, |s| {
            s.entries
                .iter()
                .filter(|e| Category::BASE.contains(&e.category))
                .count()
        })
    }

    /// Everything the scan found, including the file type chain.
    fn total_entry_count(&self) -> usize {
        self.scan.as_ref().map_or(0, |s| s.entries.len())
    }

    fn category_count(&self, category: &Category) -> usize {
        self.scan
            .as_ref()
            .and_then(|s| s.by_category.get(category))
            .map_or(0, |v| v.len())
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Cheap per-frame bookkeeping, all of it non-blocking.
        self.frame_times.push(ctx.input(|i| i.stable_dt));
        self.poll_scan();
        self.icons.poll(&ctx);
        self.sync_titlebar(ui);

        if self.filter_dirty {
            self.rebuild_visible();
            self.filter_dirty = false;
        }

        self.report_theme_once(ui);
        self.drive_bench(&ctx);

        self.top_bar(ui, &ctx);
        self.status_bar(ui);

        match self.tab {
            Tab::Categories => {
                self.category_tree(ui);
                self.detail_panel(ui);
                egui::CentralPanel::default().show(ui, |ui| self.entry_table(ui));
            }
            Tab::Backups => {
                egui::CentralPanel::default().show(ui, |ui| self.backup_list(ui));
            }
            Tab::FileTypes => {
                self.file_type_tree(ui);
                self.detail_panel(ui);
                egui::CentralPanel::default().show(ui, |ui| self.entry_table(ui));
            }
            Tab::Programs => {
                self.program_list(ui);
                self.detail_panel(ui);
                egui::CentralPanel::default().show(ui, |ui| self.entry_table(ui));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------

impl App {
    fn top_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Categories, self.tr.tab_categories),
                    (Tab::FileTypes, self.tr.tab_filetypes),
                    (Tab::Programs, self.tr.tab_programs),
                    (Tab::Backups, self.tr.tab_backups),
                ] {
                    if ui.selectable_label(self.tab == tab, label).clicked() {
                        self.tab = tab;
                        // Each tab draws from a different candidate set.
                        self.filter_dirty = true;
                        self.selection = None;
                        if tab == Tab::Backups {
                            self.reload_backups();
                        }
                    }
                }

                ui.separator();

                if ui
                    .add_enabled(!self.scanning, egui::Button::new(self.tr.btn_rescan))
                    .clicked()
                {
                    self.start_scan(ctx);
                }

                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(self.tr.search_hint)
                        .desired_width(260.0),
                );
                // Rebuilding on `changed()` instead of every frame is what
                // keeps typing responsive at a few thousand rows (ToDo 11.5).
                if search.changed() {
                    self.filter_dirty = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.settings_controls(ui, ctx);
                });
            });
            ui.add_space(4.0);
        });
    }

    fn settings_controls(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut changed = false;

        egui::ComboBox::from_id_salt("theme")
            .selected_text(match self.settings.theme {
                ThemeChoice::System => self.tr.theme_system,
                ThemeChoice::Light => self.tr.theme_light,
                ThemeChoice::Dark => self.tr.theme_dark,
            })
            .show_ui(ui, |ui| {
                for (choice, label) in [
                    (ThemeChoice::System, self.tr.theme_system),
                    (ThemeChoice::Light, self.tr.theme_light),
                    (ThemeChoice::Dark, self.tr.theme_dark),
                ] {
                    if ui
                        .selectable_value(&mut self.settings.theme, choice, label)
                        .changed()
                    {
                        changed = true;
                    }
                }
            });

        egui::ComboBox::from_id_salt("language")
            .selected_text(self.settings.language.label())
            .show_ui(ui, |ui| {
                for language in [Language::German, Language::English] {
                    if ui
                        .selectable_value(&mut self.settings.language, language, language.label())
                        .changed()
                    {
                        changed = true;
                    }
                }
            });

        if changed {
            // Language switching is a single assignment; it takes effect on
            // the next frame with no restart (ToDo 8).
            self.tr = strings_for(self.settings.language);
            ctx.set_theme(self.settings.theme.to_preference());
            // Force the title bar to be re-evaluated on the next frame.
            self.titlebar_dark = None;
            let _ = self.settings.save();
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.scanning {
                    ui.spinner();
                    let (done, total) = self.progress;
                    ui.label(format!("{} {done}/{total}", self.tr.status_scanning));
                    ui.label(&self.progress_label);
                } else {
                    ui.label(self.tr.status_ready);
                    ui.separator();
                    ui.label(
                        self.tr
                            .fmt_entries_found
                            .replace("{}", &self.total_entry_count().to_string()),
                    );
                    ui.separator();
                    ui.label(format!("{} sichtbar / shown", self.visible_rows.len()));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The instrument for the milestone 4 target. Shown rather
                    // than logged so the number is visible while scrolling,
                    // which is exactly when it would degrade.
                    ui.label(format!(
                        "{:.1} fps / {:.2} ms",
                        self.frame_times.live_fps(),
                        self.frame_times.recent_average_ms()
                    ));
                    ui.separator();
                    let (loaded, pending, failed) = self.icons.stats();
                    ui.label(format!("Icons {loaded}/{pending}/{failed}"));
                    if !self.titlebar_supported {
                        ui.separator();
                        ui.label("Titelleiste: kein DWM-Attribut");
                    }
                });
            });
        });
    }

    fn category_tree(&mut self, ui: &mut Ui) {
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(240.0)
            .size_range(180.0..=420.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let total = self.entry_count();
                        if ui
                            .selectable_label(
                                self.selected_category.is_none(),
                                format!("{}  ({total})", self.tr.tab_categories),
                            )
                            .clicked()
                        {
                            self.selected_category = None;
                            self.selection = None;
                            self.filter_dirty = true;
                        }

                        ui.separator();

                        for category in Category::BASE {
                            let count = self.category_count(&category);
                            let selected = self.selected_category.as_ref() == Some(&category);
                            let label =
                                format!("{}  ({count})", category_label(&category, self.tr));

                            // Greyed out rather than hidden: an empty category
                            // is information, not clutter. There is no
                            // SelectableLabel widget type in 0.36, only the
                            // Ui method, so disabling goes through a scope.
                            let response = ui
                                .add_enabled_ui(count > 0, |ui| {
                                    ui.selectable_label(selected, label)
                                })
                                .inner;

                            if response.clicked() {
                                self.selected_category = Some(category.clone());
                                self.selection = None;
                                self.filter_dirty = true;
                            }
                        }

                        ui.take_available_space();
                    });
            });
    }

    /// File types, grouped, with the number of entries each one adds.
    fn file_type_tree(&mut self, ui: &mut Ui) {
        egui::Panel::left("filetypes")
            .resizable(true)
            .default_size(260.0)
            .size_range(200.0..=460.0)
            .show(ui, |ui| {
                ui.add_space(4.0);

                let hide_empty = &mut self.settings.hide_empty_types;
                if ui.checkbox(hide_empty, self.tr.filter_hide_empty).changed() {
                    let _ = self.settings.save();
                }
                ui.separator();

                let Some(scan) = &self.scan else {
                    ui.spinner();
                    return;
                };

                let hide_empty = self.settings.hide_empty_types;
                let mut clicked: Option<String> = None;

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for group in crate::registry::filetypes::TypeGroup::ALL {
                            let types: Vec<_> = scan
                                .file_types
                                .iter()
                                .filter(|f| f.group == group)
                                .filter(|f| !hide_empty || f.own_entry_count() > 0)
                                .collect();
                            if types.is_empty() {
                                continue;
                            }

                            let total: usize = types.iter().map(|f| f.own_entry_count()).sum();
                            egui::CollapsingHeader::new(format!(
                                "{}  ({total})",
                                type_group_label(group)
                            ))
                            .id_salt(group)
                            .default_open(group == crate::registry::filetypes::TypeGroup::Images)
                            .show(ui, |ui| {
                                for info in types {
                                    let selected = self.selected_ext.as_deref() == Some(info.ext());
                                    let label =
                                        format!("{}  ({})", info.ext(), info.own_entry_count());
                                    if ui.selectable_label(selected, label).clicked() {
                                        clicked = Some(info.ext().to_string());
                                    }
                                }
                            });
                        }
                        ui.take_available_space();
                    });

                if let Some(ext) = clicked {
                    self.selected_ext = Some(ext);
                    self.selection = None;
                    self.filter_dirty = true;
                }
            });
    }

    /// Programs, largest first — the one worth twenty deletions is on top.
    fn program_list(&mut self, ui: &mut Ui) {
        egui::Panel::left("programs")
            .resizable(true)
            .default_size(340.0)
            .size_range(240.0..=560.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.label(
                    self.tr
                        .fmt_entries_found
                        .replace("{}", &self.groups.len().to_string()),
                );
                ui.separator();

                let mut clicked: Option<usize> = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, group) in self.groups.iter().enumerate() {
                            let selected = self.selected_group == Some(index);
                            let label =
                                format!("{:>3}×  {}", group.entry_count(), group.display_name);

                            let response = ui.selectable_label(selected, label);
                            if response.clicked() {
                                clicked = Some(index);
                            }
                            // The full path is long and only occasionally
                            // wanted, so it lives in the tooltip.
                            response.on_hover_text(&group.key);

                            if group.is_system {
                                ui.indent(index, |ui| {
                                    ui.colored_label(
                                        ui.visuals().weak_text_color(),
                                        self.tr.badge_system,
                                    );
                                });
                            }
                        }
                        ui.take_available_space();
                    });

                if let Some(index) = clicked {
                    self.selected_group = Some(index);
                    self.selection = None;
                    self.filter_dirty = true;
                }
            });
    }

    fn entry_table(&mut self, ui: &mut Ui) {
        // Destructured up front: the row closure needs the entries and the
        // icon cache at the same time, which a plain `&mut self` capture
        // would not allow.
        let Self {
            scan,
            visible_rows,
            icons,
            selection,
            tr,
            bench,
            ..
        } = self;

        let Some(scan) = scan else {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return;
        };

        if visible_rows.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(tr.msg_no_selection);
            });
            return;
        }

        let mut table = TableBuilder::new(ui);

        // A benchmark that never scrolls would measure one static screenful
        // and miss the cost that matters: rebuilding rows while moving.
        if let Some(bench) = bench
            && bench.warmup == 0
            && !visible_rows.is_empty()
        {
            table = table.scroll_to_row(bench.scroll % visible_rows.len(), None);
        }

        table
            .id_salt("entries")
            .striped(true)
            .resizable(true)
            // Without an interactive sense there is no hover highlight and
            // row clicks never register.
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .column(Column::exact(26.0))
            .column(Column::initial(260.0).at_least(120.0).clip(true))
            .column(Column::initial(90.0).at_least(70.0).clip(true))
            .column(Column::initial(80.0).at_least(60.0).clip(true))
            .column(Column::initial(110.0).at_least(70.0).clip(true))
            .column(Column::remainder().at_least(140.0).clip(true))
            .header(24.0, |mut header| {
                header.col(|_ui| {});
                header.col(|ui| {
                    ui.strong(tr.col_name);
                });
                header.col(|ui| {
                    ui.strong(tr.col_type);
                });
                header.col(|ui| {
                    ui.strong(tr.col_scope);
                });
                header.col(|ui| {
                    ui.strong(tr.col_flags);
                });
                header.col(|ui| {
                    ui.strong(tr.col_command);
                });
            })
            .body(|body| {
                // The virtualized variant: only visible rows are built. At a
                // few thousand entries this is the difference between a
                // scrolling list and a slideshow (ToDo 4.5).
                body.rows(26.0, visible_rows.len(), |mut row| {
                    let entry = &scan.entries[visible_rows[row.index()]];
                    let index = visible_rows[row.index()];

                    // Must precede the first cell; it only affects cells added
                    // after the call.
                    row.set_selected(*selection == Some(index));

                    row.col(|ui| {
                        if let Some(reference) = &entry.icon_ref {
                            // Cheap: either a ready texture or the
                            // placeholder plus a queued request.
                            let texture = icons.get(reference).clone();
                            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                                texture.id(),
                                egui::vec2(16.0, 16.0),
                            )));
                        }
                    });
                    row.col(|ui| {
                        ui.label(&entry.display_name);
                    });
                    row.col(|ui| {
                        ui.label(match entry.kind {
                            EntryKind::Verb { .. } => tr.kind_verb,
                            EntryKind::ShellEx { .. } => tr.kind_shellex,
                        });
                    });
                    row.col(|ui| {
                        ui.label(entry.scope.label());
                    });
                    row.col(|ui| {
                        badges(ui, entry, tr);
                    });
                    row.col(|ui| {
                        ui.label(detail_text(entry));
                    });

                    if row.response().clicked() {
                        *selection = Some(index);
                    }
                });
            });
    }

    fn detail_panel(&mut self, ui: &mut Ui) {
        egui::Panel::right("details")
            .resizable(true)
            .default_size(340.0)
            .size_range(240.0..=600.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.heading(self.tr.detail_title);
                ui.separator();

                let Some(entry) = self
                    .selection
                    .and_then(|i| self.scan.as_ref().and_then(|s| s.entries.get(i)))
                else {
                    ui.label(self.tr.detail_nothing_selected);
                    return;
                };

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        field(ui, self.tr.detail_display_name, &entry.display_name);
                        field(ui, self.tr.detail_registry_path, &entry.registry_path);
                        if let Some(raw) = &entry.raw_display {
                            field(ui, self.tr.detail_raw_value, raw);
                        }
                        if let Some(icon) = &entry.icon_ref {
                            field(ui, self.tr.detail_icon, icon);
                        }
                        if let Some(position) = &entry.position {
                            field(ui, self.tr.detail_position, position);
                        }
                        if let Some(applies) = &entry.applies_to {
                            field(ui, self.tr.detail_applies_to, applies);
                        }

                        match &entry.kind {
                            EntryKind::Verb {
                                command,
                                sub_commands,
                            } => {
                                if let Some(command) = command {
                                    field(ui, self.tr.detail_command, command);
                                }
                                if !sub_commands.is_empty() {
                                    ui.separator();
                                    for child in sub_commands {
                                        ui.label(format!("  ↳ {}", child.display_name));
                                    }
                                }
                            }
                            EntryKind::ShellEx {
                                clsid,
                                server_path,
                                blocked,
                            } => {
                                field(ui, self.tr.detail_clsid, clsid);
                                if let Some(server) = server_path {
                                    field(ui, self.tr.detail_server, server);
                                }
                                if *blocked {
                                    ui.colored_label(
                                        ui.visuals().warn_fg_color,
                                        self.tr.badge_blocked,
                                    );
                                }
                                ui.add_space(6.0);
                                ui.label(self.tr.msg_com_handler_note);
                            }
                        }

                        if entry.read_only {
                            ui.add_space(6.0);
                            ui.colored_label(ui.visuals().warn_fg_color, self.tr.msg_needs_admin);
                        }
                    });
            });
    }

    fn backup_list(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading(self.tr.tab_backups);
            if ui.button(self.tr.btn_rescan).clicked() {
                self.reload_backups();
            }
        });
        ui.separator();

        if let Some(error) = &self.backup_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }

        if self.backups.is_empty() {
            ui.label(self.tr.msg_backup_first);
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (path, manifest) in &self.backups {
                    egui::CollapsingHeader::new(format!(
                        "{}  —  {}  ({})",
                        manifest.created_at.format("%Y-%m-%d %H:%M:%S"),
                        manifest.action,
                        manifest.entries.len()
                    ))
                    .id_salt(path)
                    .show(ui, |ui| {
                        ui.label(path.display().to_string());
                        for entry in &manifest.entries {
                            ui.label(format!("  {}", entry.registry_path));
                        }
                        for missing in &manifest.missing {
                            ui.colored_label(
                                ui.visuals().weak_text_color(),
                                format!("  {missing} (fehlte / missing)"),
                            );
                        }
                    });
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn strings_for(language: Language) -> &'static Strings {
    match language {
        Language::German => &i18n::DE,
        Language::English => &i18n::EN,
    }
}

/// Human label for a file type group.
///
/// Not in the i18n table: these are the eight fixed buckets of the curated
/// list, and adding sixteen more fields for them would bury the strings that
/// actually vary.
fn type_group_label(group: crate::registry::filetypes::TypeGroup) -> &'static str {
    use crate::registry::filetypes::TypeGroup as G;
    match group {
        G::Documents => "Dokumente / Documents",
        G::Images => "Bilder / Images",
        G::Raw => "RAW",
        G::Audio => "Audio",
        G::Video => "Video",
        G::Archives => "Archive / Archives",
        G::Code => "Code",
        G::System => "System",
        G::Other => "Sonstige / Other",
    }
}

fn category_label(category: &Category, tr: &'static Strings) -> &'static str {
    match category {
        Category::AllFiles => tr.cat_all_files,
        Category::AllFilesystemObjects => tr.cat_all_filesystem_objects,
        Category::Directory => tr.cat_directory,
        Category::DirectoryBackground => tr.cat_directory_background,
        Category::Folder => tr.cat_folder,
        Category::DesktopBackground => tr.cat_desktop_background,
        Category::Drive => tr.cat_drive,
        // Only reachable from milestone 7 onwards.
        _ => tr.tab_filetypes,
    }
}

fn matches_search(entry: &ContextEntry, needle: &str) -> bool {
    entry.display_name.to_lowercase().contains(needle)
        || entry.key_name.to_lowercase().contains(needle)
        || entry.registry_path.to_lowercase().contains(needle)
        || match &entry.kind {
            EntryKind::Verb { command, .. } => command
                .as_ref()
                .is_some_and(|c| c.to_lowercase().contains(needle)),
            EntryKind::ShellEx {
                clsid, server_path, ..
            } => {
                clsid.to_lowercase().contains(needle)
                    || server_path
                        .as_ref()
                        .is_some_and(|s| s.to_lowercase().contains(needle))
            }
        }
}

fn detail_text(entry: &ContextEntry) -> &str {
    match &entry.kind {
        EntryKind::Verb { command, .. } => command.as_deref().unwrap_or("—"),
        EntryKind::ShellEx { clsid, .. } => clsid,
    }
}

fn badges(ui: &mut Ui, entry: &ContextEntry, tr: &'static Strings) {
    // Colours come out of the current visuals rather than being fixed: the
    // same RGB is not equally readable in both themes (ToDo 9.2).
    let warn = ui.visuals().warn_fg_color;
    let weak = ui.visuals().weak_text_color();

    if entry.read_only {
        ui.colored_label(weak, "🔒");
    }
    if entry.hidden {
        ui.colored_label(weak, tr.badge_hidden);
    }
    if entry.extended {
        ui.colored_label(warn, "⇧");
    }
    if let EntryKind::ShellEx { blocked: true, .. } = entry.kind {
        ui.colored_label(warn, tr.badge_blocked);
    }
    if let Some(position) = &entry.position {
        ui.label(position.chars().take(3).collect::<String>());
    }
}

fn field(ui: &mut Ui, label: &str, value: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(label).weak().small());
    // Selectable so a registry path can be copied out into regedit.
    ui.add(egui::Label::new(value).selectable(true).wrap());
}

/// Segoe UI, so the window does not look foreign on Windows.
///
/// egui ships its own font, which is immediately recognisable and wrong for a
/// system tool. A failed read leaves the default font rather than panicking
/// (ToDo 9.3).
fn install_fonts(ctx: &egui::Context) {
    // Segoe UI Variable only exists from Windows 11 on, so the older file is
    // tried first — it is present everywhere.
    for candidate in [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\SegUIVar.ttf",
    ] {
        let Ok(data) = std::fs::read(candidate) else {
            continue;
        };

        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "segoe".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(data)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe".to_owned());
        ctx.set_fonts(fonts);
        return;
    }
}

/// A short rolling window of frame times.
struct FrameTimes {
    samples: Vec<f32>,
    next: usize,
}

const FRAME_WINDOW: usize = 1024;
/// How many of the most recent frames the live readout averages over.
const LIVE_WINDOW: usize = 60;

impl Default for FrameTimes {
    fn default() -> Self {
        Self {
            samples: Vec::with_capacity(FRAME_WINDOW),
            next: 0,
        }
    }
}

impl FrameTimes {
    fn push(&mut self, dt: f32) {
        if self.samples.len() < FRAME_WINDOW {
            self.samples.push(dt);
        } else {
            self.samples[self.next] = dt;
            self.next = (self.next + 1) % FRAME_WINDOW;
        }
    }

    fn count(&self) -> usize {
        self.samples.len()
    }

    fn average_ms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().sum::<f32>() / self.samples.len() as f32 * 1000.0
    }

    /// Average over the most recent frames only, for the live readout.
    fn recent_average_ms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let take = LIVE_WINDOW.min(self.samples.len());
        let start = self.samples.len() - take;
        self.samples[start..].iter().sum::<f32>() / take as f32 * 1000.0
    }

    /// The 95th percentile matters more than the average: a mean of 8 ms with
    /// occasional 40 ms frames still reads as stutter.
    fn percentile_ms(&self, quantile: f32) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(f32::total_cmp);
        let index = ((sorted.len() - 1) as f32 * quantile).round() as usize;
        sorted[index] * 1000.0
    }

    fn worst_ms(&self) -> f32 {
        self.samples.iter().copied().fold(0.0_f32, f32::max) * 1000.0
    }

    fn fps(&self) -> f32 {
        let average = self.average_ms();
        if average <= 0.0 {
            0.0
        } else {
            1000.0 / average
        }
    }

    fn live_fps(&self) -> f32 {
        let average = self.recent_average_ms();
        if average <= 0.0 {
            0.0
        } else {
            1000.0 / average
        }
    }
}

/// Launches the window.
pub fn run(
    synthetic: Option<usize>,
    bench_frames: Option<usize>,
    start_tab: Tab,
) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(i18n::DE.app_title)
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ctxmenu",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, synthetic, bench_frames, start_tab)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    #[test]
    fn search_covers_name_command_and_path() {
        let entries = synthetic::scan_result(40).entries;
        let entry = &entries[0];

        assert!(matches_search(entry, &entry.display_name.to_lowercase()));
        assert!(matches_search(entry, "synthetic00000"));
        assert!(matches_search(entry, "hkcu\\software\\classes"));
        assert!(!matches_search(entry, "definitely not present"));
    }

    #[test]
    fn search_is_case_insensitive() {
        let entries = synthetic::scan_result(10).entries;
        assert!(matches_search(&entries[0], "synthetic00000"));
        assert!(matches_search(
            &entries[0],
            "SYNTHETIC00000".to_lowercase().as_str()
        ));
    }

    #[test]
    fn every_base_category_has_a_translated_label() {
        for category in Category::BASE {
            for table in [&i18n::DE, &i18n::EN] {
                let label = category_label(&category, table);
                assert!(!label.trim().is_empty(), "{category:?} has no label");
                assert_ne!(
                    label, table.tab_filetypes,
                    "{category:?} fell through to the catch-all arm"
                );
            }
        }
    }

    #[test]
    fn frame_times_average_over_the_window() {
        let mut times = FrameTimes::default();
        assert_eq!(times.fps(), 0.0, "no samples yet");

        for _ in 0..120 {
            times.push(1.0 / 60.0);
        }
        assert_eq!(times.count(), 120);
        assert!((times.fps() - 60.0).abs() < 0.5, "got {}", times.fps());
        assert!((times.live_fps() - 60.0).abs() < 0.5);
        assert!((times.average_ms() - 16.667).abs() < 0.1);

        // One very slow frame must move the percentile, not vanish into the
        // mean -- that is the whole reason the percentile is reported.
        times.push(0.5);
        assert!(times.worst_ms() >= 500.0);
        assert!(times.percentile_ms(1.0) >= 500.0);
        assert!(times.percentile_ms(0.5) < 20.0);
    }
}
