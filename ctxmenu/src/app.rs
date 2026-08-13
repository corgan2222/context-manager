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

use crate::elevation;
use crate::favourites::{
    self, Favourite, Header, ResultAction, ResultSource, Tool, Upload, UploadBody, WebMode, WebTool,
};
use crate::i18n::{self, Strings};
use crate::icons::cache::IconCache;
use crate::model::{Category, ContextEntry, EntryKind, ScanProgress, ScanResult};
use crate::program::group::{self, ProgramGroup};
use crate::program::identity::NameResolver;
use crate::registry::backup::{self, BackupManifest};
use crate::registry::create::{self, NewEntry, Problem};
use crate::registry::plan::{Action, Operation, Plan, Report};
use crate::registry::scan::{self, ScanOptions};
use crate::settings::{Language, Settings, ThemeChoice};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Categories,
    FileTypes,
    Programs,
    Favourites,
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
            "favourites" | "favoriten" => Some(Tab::Favourites),
            "backups" | "sicherungen" => Some(Tab::Backups),
            _ => None,
        }
    }
}

/// Placeholder shown in the empty command field.
const HINT_COMMAND: &str = "\"C:\\Windows\\notepad.exe\" \"%1\"";
/// Placeholder shown in the empty icon field.
const HINT_ICON: &str = "C:\\Windows\\notepad.exe,0";
/// Placeholders in the favourite editor.
const HINT_PROGRAM: &str = "C:\\Program Files\\Werkzeug\\werkzeug.exe";
const HINT_ARGS: &str = "--flag \"%1\"";
const HINT_URL: &str = "https://squoosh.app";
const HINT_ENDPOINT: &str = "https://api.tinify.com/shrink";

/// A modal question or report.
enum Dialog {
    /// Drawn up but not yet applied. Holds everything the user needs to
    /// decide: how much, how reversible, and whether Windows will ask.
    Confirm {
        plan: Plan,
        needs_elevation: bool,
    },
    Running,
    Done(Report),
    Error(String),
    /// The form for one favourite.
    Favourite {
        draft: Box<Favourite>,
        /// Adding rather than editing; decides between insert and replace.
        fresh: bool,
    },
    /// "Where should this favourite appear?" — the one decision left once a
    /// tool is in the list.
    Place {
        favourite: Box<Favourite>,
        category: Category,
        /// Filled in when the category is a file type; kept across switching
        /// so a typed extension is not lost by clicking around.
        ext: String,
        perceived: String,
    },
    /// The form for a new entry of one's own (milestone 10).
    Editor {
        entry: Box<NewEntry>,
        /// Read when the dialog opens, not per frame: this is a file on disk,
        /// and the frame path has no business touching one (ToDo 4.3).
        recorded: Vec<NewEntry>,
    },
}

/// Which column the table is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Kind,
    Scope,
    AppliesTo,
    Command,
}

/// One line of the table.
///
/// A cascading menu is one registry key with its children nested underneath,
/// so a flat list of entry indices cannot show it. A row therefore names an
/// entry plus, optionally, the path down into its `sub_commands` — one index
/// per nesting level, because Windows allows a submenu inside a submenu.
///
/// Only top-level rows are selectable: a child lives at
/// `…\shell\<parent>\shell\<child>`, which is deliberately not expressible
/// as a `RegTarget`, so no action could be applied to it anyway.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Index into `ScanResult::entries`.
    pub entry: usize,
    /// Empty for the entry itself.
    pub path: Vec<usize>,
}

impl Row {
    fn top(entry: usize) -> Self {
        Row {
            entry,
            path: Vec::new(),
        }
    }

    fn is_top(&self) -> bool {
        self.path.is_empty()
    }
}

/// Walks a row's path down into the nested children.
///
/// Returns `None` only if the path does not fit the entry, which can happen
/// for a single frame after a rescan replaced the scan behind the rows.
fn resolve<'a>(scan: &'a ScanResult, row: &Row) -> Option<&'a ContextEntry> {
    let mut entry = scan.entries.get(row.entry)?;
    for step in &row.path {
        entry = children(entry)?.get(*step)?;
    }
    Some(entry)
}

fn children(entry: &ContextEntry) -> Option<&Vec<ContextEntry>> {
    match &entry.kind {
        EntryKind::Verb { sub_commands, .. } if !sub_commands.is_empty() => Some(sub_commands),
        _ => None,
    }
}

/// Appends `entry` and everything below it.
fn push_with_children(rows: &mut Vec<Row>, entry: &ContextEntry, row: Row) {
    let Some(kids) = children(entry) else {
        rows.push(row);
        return;
    };

    rows.push(row.clone());
    for (index, child) in kids.iter().enumerate() {
        let mut path = row.path.clone();
        path.push(index);
        push_with_children(
            rows,
            child,
            Row {
                entry: row.entry,
                path,
            },
        );
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
    visible_rows: Vec<Row>,
    filter_dirty: bool,
    /// Column and direction the list is ordered by. Applied when the rows are
    /// rebuilt, never per frame.
    sort: (SortBy, bool),
    /// Set when the list is about to show something else entirely. The table
    /// keeps its scroll offset otherwise, and a list that changed under a
    /// screenful of unchanged rows looks like a list that did not change.
    scroll_to_top: bool,

    tab: Tab,
    selected_category: Option<Category>,
    /// Selected extension in the file type tab.
    selected_ext: Option<String>,
    /// Index into `groups` for the program tab.
    selected_group: Option<usize>,
    /// Built once after each scan; never in the frame path.
    groups: Vec<ProgramGroup>,
    /// Indices into `scan.entries`. Multi-select, because the whole point of
    /// the program view is acting on twenty entries at once.
    selected: rustc_hash::FxHashSet<usize>,
    /// The row whose details are shown — the last one clicked.
    focused: Option<usize>,
    search: String,

    dialog: Option<Dialog>,
    /// Receives the report from the worker that applies a plan.
    action_rx: Option<Receiver<Result<Report, String>>>,

    scan_rx: Option<Receiver<ScanMessage>>,
    scanning: bool,
    progress: (usize, usize),
    progress_label: String,

    backups: Vec<(std::path::PathBuf, BackupManifest)>,
    backup_error: Option<String>,

    /// The tool box (`favourites.json`), read on entering the tab and after
    /// every change — never per frame.
    favourites: Vec<Favourite>,
    favourite_error: Option<String>,

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
    /// Milliseconds from process creation to the first frame that actually
    /// showed rows — the milestone 12 target of under two seconds.
    ///
    /// Measured from the process creation time rather than from `main`, so the
    /// loader, the static CRT and the window creation are all inside the
    /// number instead of hiding in front of it.
    first_list_ms: Option<f64>,
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
    /// How many arrow-key presses were fed in, and how far the cursor
    /// actually walked. Reported together: a keyboard handler that quietly
    /// does nothing looks exactly like one that works until somebody tries
    /// it, which is how this feature was found missing in the first place.
    keys_sent: usize,
    cursor_walked: usize,
    last_focus: Option<usize>,
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
        start_search: String,
    ) -> Self {
        install_fonts(&cc.egui_ctx);

        let settings = Settings::load_or_default(theme::system_language());
        cc.egui_ctx.set_theme(settings.theme.to_preference());

        // Why a table row sometimes ignored a click: egui makes every label
        // selectable by default, a selectable label senses click-and-drag, and
        // the label is registered *above* the cell that contains it. On a tie
        // the topmost widget wins, so clicking the text hit the label and the
        // row never heard about it — the more text, the worse. Nothing here
        // wants text selection; the rows want clicks.
        cc.egui_ctx.all_styles_mut(|style| {
            style.interaction.selectable_labels = false;
        });

        let tr = strings_for(settings.language);
        let hwnd = theme::window_handle(cc);

        let mut app = Self {
            scan: None,
            visible_rows: Vec::new(),
            filter_dirty: true,
            sort: (SortBy::Name, true),
            scroll_to_top: false,
            tab: start_tab,
            selected_category: None,
            selected_ext: None,
            selected_group: None,
            groups: Vec::new(),
            selected: rustc_hash::FxHashSet::default(),
            focused: None,
            search: start_search,
            dialog: None,
            action_rx: None,
            scan_rx: None,
            scanning: false,
            progress: (0, 0),
            progress_label: String::new(),
            backups: Vec::new(),
            backup_error: None,
            favourites: Vec::new(),
            favourite_error: None,
            icons: IconCache::new(&cc.egui_ctx),
            tr,
            settings,
            hwnd,
            titlebar_dark: None,
            titlebar_supported: true,
            frame_times: FrameTimes::default(),
            theme_reported: false,
            first_list_ms: None,
            bench: bench_frames.map(|frames| Bench {
                // The first frames pay for fonts, textures and window setup;
                // measuring those would flatter or slander the result.
                warmup: 120,
                remaining: frames,
                scroll: 0,
                keys_sent: 0,
                cursor_walked: 0,
                last_focus: None,
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
        self.clear_selection();
    }

    /// Drains the scan channel. Never blocks.
    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };

        let mut finished = false;
        let mut died = false;

        loop {
            match rx.try_recv() {
                Ok(ScanMessage::Progress(progress)) => {
                    self.progress = (progress.done, progress.total);
                    self.progress_label = progress.label;
                }
                Ok(ScanMessage::Done(result)) => {
                    let result = *result;
                    // Version resource lookups hit the disk, so the grouping
                    // is built once here and never per frame.
                    let mut names = NameResolver::new();
                    self.groups = group::build(&result, &mut names);
                    self.scan = Some(result);
                    self.filter_dirty = true;
                    finished = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // The worker died without sending a result. Without noticing,
                // the spinner would turn for ever and "Rescan" would stay
                // disabled — which looks exactly like a hung program.
                //
                // Detected here rather than by a separate probe: `try_recv`
                // *takes* a message when there is one, and a probe outside
                // this loop swallowed the finished scan. Measured, by the
                // benchmark reporting zero rows.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    died = !finished;
                    break;
                }
            }
        }

        if finished || died {
            self.scanning = false;
            self.scan_rx = None;
        }
        if died {
            crate::errln!("scan thread ended without a result");
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

        let searching = !self.search.trim().is_empty();
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

            // Without a selection the file type and program tabs show nothing,
            // which is right for browsing and wrong for searching: a term
            // typed into an empty tab would answer "no hits" while the entry
            // sits two clicks away. A search therefore widens the candidate
            // set to the whole tab.
            Tab::FileTypes if self.selected_ext.is_none() && !searching => Vec::new(),
            Tab::FileTypes if self.selected_ext.is_none() => scan
                .file_types
                .iter()
                .flat_map(|info| info.entry_indices.iter().copied())
                .collect(),

            Tab::Programs if self.selected_group.is_none() && searching => self
                .groups
                .iter()
                .flat_map(|group| group.entry_indices.iter().copied())
                .collect(),

            Tab::FileTypes => match &self.selected_ext {
                Some(ext) => {
                    // What belongs to *this* type comes first. The other way
                    // round — which is how this started — put 39 rows that are
                    // identical for every extension above the 19 that are not,
                    // so switching from .jpg to .mp3 changed only what was
                    // already below the fold and the tab looked dead after the
                    // first click.
                    let mut rows: Vec<usize> = scan
                        .file_types
                        .iter()
                        .find(|f| f.ext() == ext)
                        .map(|info| info.entry_indices.clone())
                        .unwrap_or_default();

                    // Levels 1 and 2 apply to every file, so they belong in
                    // the list for this type even though they are not stored
                    // under it (ToDo 10.4). Showing only levels 3 to 7 would
                    // understate what a right-click actually offers.
                    rows.extend(scan.entries.iter().enumerate().filter_map(|(i, e)| {
                        matches!(
                            e.category,
                            Category::AllFiles | Category::AllFilesystemObjects
                        )
                        .then_some(i)
                    }));
                    rows
                }
                None => Vec::new(),
            },

            Tab::Programs => self
                .selected_group
                .and_then(|i| self.groups.get(i))
                .map(|g| g.entry_indices.clone())
                .unwrap_or_default(),

            Tab::Backups | Tab::Favourites => Vec::new(),
        };

        let needle = self.search.trim().to_lowercase();
        let mut candidates: Vec<usize> = candidates
            .into_iter()
            .filter(|index| needle.is_empty() || matches_search(&scan.entries[*index], &needle))
            .collect();

        // Sorted here, once per change, not per frame. Children keep their
        // place under the parent they belong to: a cascading menu that sorted
        // itself apart would stop being a menu.
        let (column, ascending) = self.sort;
        candidates.sort_by(|a, b| {
            let (a, b) = (&scan.entries[*a], &scan.entries[*b]);
            let ordering = sort_key(a, column).cmp(&sort_key(b, column));
            if ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });

        for index in candidates {
            let entry = &scan.entries[index];
            // Children come with their parent rather than being filtered
            // separately: a submenu entry without the menu it hangs in would
            // say nothing about where it appears.
            push_with_children(&mut self.visible_rows, entry, Row::top(index));
        }

        // Somebody who picks a program wants to see what it does, not an empty
        // panel and a second click. Only when nothing is selected: an existing
        // selection is the user's and must survive a search or a re-sort.
        if self.focused.is_none()
            && let Some(first) = self.visible_rows.iter().find(|row| row.is_top())
        {
            self.focused = Some(first.entry);
        }
    }

    /// Moves the cursor with the keyboard.
    ///
    /// Runs before the table is drawn, because the table consumes what it
    /// scrolls to. Only top-level rows are stops: a submenu child cannot be
    /// acted on, so a cursor that landed on one would be a dead end.
    fn handle_keys(&mut self, ctx: &egui::Context) -> Option<usize> {
        // Not while a dialog is up: arrow keys belong to whatever is asking.
        if self.dialog.is_some() {
            return None;
        }

        let stops: Vec<usize> = self
            .visible_rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.is_top())
            .map(|(position, _)| position)
            .collect();
        if stops.is_empty() {
            return None;
        }

        let current = self
            .focused
            .and_then(|entry| {
                self.visible_rows
                    .iter()
                    .position(|row| row.is_top() && row.entry == entry)
            })
            .and_then(|position| stops.iter().position(|p| *p == position));

        let (down, up, home, end, extend) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    || i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                    || i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Home),
                i.consume_key(egui::Modifiers::NONE, egui::Key::End),
                i.modifiers.shift,
            )
        });

        let next = match (down, up, home, end) {
            (true, _, _, _) => Some(match current {
                Some(index) => (index + 1).min(stops.len() - 1),
                None => 0,
            }),
            (_, true, _, _) => Some(match current {
                Some(index) => index.saturating_sub(1),
                None => 0,
            }),
            (_, _, true, _) => Some(0),
            (_, _, _, true) => Some(stops.len() - 1),
            _ => None,
        }?;

        let row = &self.visible_rows[stops[next]];
        let entry = row.entry;
        self.focused = Some(entry);
        if !extend {
            self.selected.clear();
        }
        self.selected.insert(entry);

        // The row to scroll to, in table coordinates.
        Some(stops[next])
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

    /// Notes when the table first had something in it.
    ///
    /// Called at the end of a frame, so "the list is visible" means the rows
    /// were built in this frame, not merely that the data arrived.
    fn note_first_list(&mut self) {
        if self.first_list_ms.is_some() || self.visible_rows.is_empty() {
            return;
        }

        let ms = milliseconds_since_process_start();
        self.first_list_ms = Some(ms);
        crate::errln!(
            "startup_to_first_list_ms={ms:.0} entries={} rows={}",
            self.total_entry_count(),
            self.visible_rows.len()
        );
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

        // Every tenth frame, press the down arrow — through the same event
        // queue the window manager uses, so this exercises the real handler
        // rather than calling it directly.
        if bench.remaining % 10 == 0 {
            bench.keys_sent += 1;
            ctx.input_mut(|input| {
                input.events.push(egui::Event::Key {
                    key: egui::Key::ArrowDown,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            });
        }

        if self.focused != bench.last_focus {
            if bench.last_focus.is_some() {
                bench.cursor_walked += 1;
            }
            bench.last_focus = self.focused;
        }

        if bench.remaining == 0 {
            let rows = self.visible_rows.len();
            let bench = self.bench.as_ref().expect("bench is running");
            crate::errln!(
                "bench: rows={rows} frames_measured={} avg={:.3}ms p95={:.3}ms worst={:.3}ms fps={:.1} keys_sent={} cursor_walked={}",
                self.frame_times.count(),
                self.frame_times.average_ms(),
                self.frame_times.percentile_ms(0.95),
                self.frame_times.worst_ms(),
                self.frame_times.fps(),
                bench.keys_sent,
                bench.cursor_walked
            );
            crate::console::flush();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Clears the selection and takes the view back to the top.
    ///
    /// Called whenever the list is about to show a different set — another
    /// category, extension or program.
    fn clear_selection(&mut self) {
        self.selected.clear();
        self.focused = None;
        self.scroll_to_top = true;
    }

    /// Builds a plan from the current selection.
    ///
    /// Read-only entries are dropped rather than attempted: offering an action
    /// that is certain to fail wastes a backup and a UAC prompt on nothing.
    /// For COM handlers the block action needs the CLSID, so an entry without
    /// one is skipped too.
    fn plan_for_selection(&self, action: Action) -> Plan {
        let Some(scan) = &self.scan else {
            return Plan::new("leer", Vec::new());
        };

        let mut operations = Vec::new();
        for &index in &self.selected {
            let Some(entry) = scan.entries.get(index) else {
                continue;
            };
            let Ok(target) = crate::registry::paths::RegTarget::parse(&entry.registry_path) else {
                continue;
            };

            let clsid = match &entry.kind {
                EntryKind::ShellEx { clsid, .. } if !clsid.is_empty() => Some(clsid.clone()),
                _ => None,
            };
            // Blocking is a COM-handler mechanism; a static verb has no CLSID
            // and no equivalent, which is why ToDo 11.3 offers LegacyDisable
            // there instead.
            if matches!(action, Action::Block | Action::Unblock) && clsid.is_none() {
                continue;
            }

            operations.push(Operation {
                target,
                action: action.clone(),
                clsid,
                display_name: entry.display_name.clone(),
            });
        }

        Plan::new(action.label(), operations)
    }

    /// Opens the confirmation dialog for an action on the selection.
    fn propose(&mut self, action: Action) {
        let plan = self.plan_for_selection(action);
        if plan.is_empty() {
            self.dialog = Some(Dialog::Error(self.tr.msg_no_selection.to_string()));
            return;
        }
        // Probing writability touches the registry, so it happens here on the
        // click and not while drawing the dialog.
        let needs_elevation = plan.needs_elevation();
        self.dialog = Some(Dialog::Confirm {
            plan,
            needs_elevation,
        });
    }

    /// Backs up without changing anything.
    ///
    /// The selection, or everything currently listed when nothing is selected.
    /// Until now a backup only ever happened as a by-product of a change,
    /// which meant "let me keep this state before I start poking" had no
    /// button at all.
    fn backup_now(&mut self) {
        let Some(scan) = &self.scan else { return };

        let indices: Vec<usize> = if self.selected.is_empty() {
            self.visible_rows
                .iter()
                .filter(|row| row.is_top())
                .map(|row| row.entry)
                .collect()
        } else {
            self.selected.iter().copied().collect()
        };

        let paths: Vec<String> = indices
            .iter()
            .filter_map(|i| scan.entries.get(*i))
            .map(|entry| entry.registry_path.clone())
            .collect();

        if paths.is_empty() {
            self.dialog = Some(Dialog::Error(self.tr.msg_no_selection.to_string()));
            return;
        }

        match backup::export("manuell", &paths) {
            Ok(token) => {
                let directory = token.directory().display().to_string();
                self.reload_backups();
                self.dialog = Some(Dialog::Error(
                    self.tr.fmt_backup_created.replace("{}", &directory),
                ));
            }
            Err(error) => self.dialog = Some(Dialog::Error(format!("{error:#}"))),
        }
    }

    /// Applies a plan on a worker thread.
    ///
    /// On a thread because it takes a backup through `reg.exe` and may wait
    /// for a UAC prompt — both of which would freeze the window if they ran
    /// in the frame path.
    fn apply(&mut self, plan: Plan, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        // Copied out before the thread starts: `&'static Strings` is fine to
        // move, and the worker must not reach into `self`.
        let tr = self.tr;

        std::thread::Builder::new()
            .name("apply-plan".into())
            .spawn(move || {
                let (direct, elevated) = plan.partition();

                // The unelevated half first: if the user then declines the
                // UAC prompt, they still keep the changes that never needed
                // it, and the report says exactly which those were.
                let mut outcome =
                    crate::registry::plan::execute(&direct).map_err(|e| format!("{e:#}"));

                if !elevated.is_empty() {
                    match elevation::run_elevated(&elevated) {
                        Ok(second) => {
                            if let Ok(first) = &mut outcome {
                                first.merge(second);
                            }
                        }
                        Err(error) => {
                            let message = format!("{error:#}");
                            outcome = match outcome {
                                // Partial success plus a declined prompt is
                                // not a failure of the whole operation.
                                Ok(mut report) => {
                                    report.results.push(crate::registry::plan::OperationResult {
                                        display_name: tr.elevated_part.to_string(),
                                        registry_path: String::new(),
                                        action: Action::Hide,
                                        error: Some(message),
                                    });
                                    Ok(report)
                                }
                                Err(_) => Err(message),
                            };
                        }
                    }
                }

                let _ = tx.send(outcome);
                ctx.request_repaint();
            })
            .expect("apply thread");

        self.action_rx = Some(rx);
        self.dialog = Some(Dialog::Running);
    }

    /// Picks up the worker's report. Never blocks.
    fn poll_action(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.action_rx else { return };
        let Ok(outcome) = rx.try_recv() else { return };

        self.action_rx = None;
        self.dialog = Some(match outcome {
            Ok(report) => Dialog::Done(report),
            Err(message) => Dialog::Error(message),
        });

        // From the unelevated parent: a notification sent by the elevated
        // child would reach the wrong session.
        elevation::notify_shell();
        self.clear_selection();
        self.start_scan(ctx);
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

        // Before the keyboard is read, not after: the benchmark feeds
        // synthetic key presses into this frame's event queue, and a check
        // that runs first would always measure nothing.
        self.drive_bench(&ctx);
        let scroll_to = match std::mem::take(&mut self.scroll_to_top) {
            true => Some(0),
            false => self.handle_keys(&ctx),
        };

        self.report_theme_once(ui);
        self.poll_action(&ctx);

        self.top_bar(ui, &ctx);
        self.action_bar(ui, &ctx);
        self.status_bar(ui);
        self.dialogs(ui, &ctx);

        match self.tab {
            Tab::Categories => {
                self.category_tree(ui);
                self.detail_panel(ui);
                egui::CentralPanel::default().show(ui, |ui| self.entry_table(ui, scroll_to));
            }
            Tab::Backups => {
                egui::CentralPanel::default().show(ui, |ui| self.backup_list(ui));
            }
            Tab::Favourites => {
                egui::CentralPanel::default().show(ui, |ui| self.favourite_list(ui));
            }
            Tab::FileTypes => {
                self.file_type_tree(ui);
                self.detail_panel(ui);
                egui::CentralPanel::default().show(ui, |ui| self.entry_table(ui, scroll_to));
            }
            Tab::Programs => {
                self.program_list(ui);
                self.detail_panel(ui);
                egui::CentralPanel::default().show(ui, |ui| self.entry_table(ui, scroll_to));
            }
        }

        // After the panels: at this point the rows of this frame really have
        // been built, which is what "the list is visible" has to mean.
        self.note_first_list();
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
                    (Tab::Favourites, self.tr.tab_favourites),
                    (Tab::Backups, self.tr.tab_backups),
                ] {
                    if ui.selectable_label(self.tab == tab, label).clicked() {
                        self.tab = tab;
                        // Each tab draws from a different candidate set.
                        self.filter_dirty = true;
                        self.clear_selection();
                        if tab == Tab::Backups {
                            self.reload_backups();
                        }
                        if tab == Tab::Favourites {
                            // Read from disk on entering rather than per
                            // frame, and again after every change: the file is
                            // also written by the `--favourite` runner.
                            self.reload_favourites();
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

    /// The actions, offered from gentle to harsh (ToDo 11.3).
    ///
    /// Delete sits at the far end behind a separator, and never as the first
    /// thing under the cursor.
    fn action_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        // Both of these tabs act on their own list, not on scanned entries;
        // a bar full of buttons that would apply to nothing is worse than no
        // bar at all.
        if matches!(self.tab, Tab::Backups | Tab::Favourites) {
            return;
        }

        egui::Panel::top("actions").show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let count = self.selected.len();
                let any = count > 0;

                ui.label(self.tr.fmt_selected_count.replace("{}", &count.to_string()));
                ui.separator();

                // Creating one's own entry does not act on the selection, so
                // it sits before the selection controls rather than among the
                // actions that do.
                if ui
                    .button(self.tr.editor_new)
                    .on_hover_text(self.tr.tip_editor_new)
                    .clicked()
                {
                    self.dialog = Some(Dialog::Editor {
                        entry: Box::new(NewEntry {
                            category: self
                                .selected_category
                                .clone()
                                .unwrap_or(Category::Directory),
                            key_name: String::new(),
                            display_name: String::new(),
                            command: String::new(),
                            icon: None,
                            position: None,
                            extended: false,
                        }),
                        recorded: create::recorded().unwrap_or_default(),
                    });
                }
                ui.separator();

                if ui
                    .button(self.tr.btn_select_all)
                    .on_hover_text(self.tr.tip_select_all)
                    .clicked()
                {
                    self.selected = self
                        .visible_rows
                        .iter()
                        .filter(|row| row.is_top())
                        .map(|row| row.entry)
                        .collect();
                }
                if ui
                    .add_enabled(any, egui::Button::new(self.tr.btn_select_none))
                    .on_hover_text(self.tr.tip_select_none)
                    .clicked()
                {
                    self.clear_selection();
                }

                ui.separator();

                // Backing up without changing anything. Its own button because
                // "look first, decide later" is a legitimate way to use this
                // program, and until now a backup only ever happened as a side
                // effect of changing something.
                if ui
                    .button(self.tr.btn_backup_now)
                    .on_hover_text(self.tr.tip_backup_now)
                    .clicked()
                {
                    self.backup_now();
                }

                ui.separator();

                for (label, tip, action) in [
                    (self.tr.btn_disable, self.tr.tip_disable, Action::Hide),
                    (
                        self.tr.btn_shift_only,
                        self.tr.tip_shift_only,
                        Action::ShiftOnly,
                    ),
                    (self.tr.btn_block, self.tr.tip_block, Action::Block),
                ] {
                    if ui
                        .add_enabled(any, egui::Button::new(label))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.propose(action);
                    }
                }

                ui.separator();

                // Visually set apart: this is the one action a backup cannot
                // be shrugged off for.
                let delete = egui::Button::new(
                    egui::RichText::new(self.tr.btn_delete).color(ui.visuals().error_fg_color),
                );
                if ui
                    .add_enabled(any, delete)
                    .on_hover_text(self.tr.tip_delete)
                    .clicked()
                {
                    self.propose(Action::Delete);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(if elevation::is_elevated() {
                        self.tr.status_elevated
                    } else {
                        self.tr.status_not_elevated
                    });
                });
            });
            ui.add_space(2.0);
        });

        // Always created, even with nothing selected, and greyed out
        // instead. A panel that comes and goes shifts the automatic widget ids
        // of every panel after it, and egui resolves clicks against the
        // previous frame's rectangles — so the frame right after a selection
        // change could drop a click on the tree.
        let any_selected = !self.selected.is_empty();
        egui::Panel::top("actions_undo").show(ui, |ui| {
            ui.add_enabled_ui(any_selected, |ui| {
                ui.horizontal(|ui| {
                    ui.small(self.tr.msg_backup_first);
                    ui.separator();
                    // The inverses, deliberately smaller: undoing is a normal
                    // thing to want, but it is not what the bar is for.
                    for (label, action) in [
                        (self.tr.act_show, Action::Show),
                        (self.tr.act_always_show, Action::AlwaysShow),
                        (self.tr.act_unblock, Action::Unblock),
                    ] {
                        if ui.small_button(label).clicked() {
                            self.propose(action);
                        }
                    }

                    ui.separator();
                    // Both values verified on Windows 10 by writing probe verbs
                    // and photographing a real right-click: an entry with Top
                    // rises above alphabetically earlier siblings, one with
                    // Bottom sinks below everything. Only three coarse blocks are
                    // on offer, which is all Windows actually gives.
                    ui.small(format!("{}:", self.tr.detail_position));
                    for (label, value) in [
                        (self.tr.pos_top, Some("Top")),
                        (self.tr.pos_bottom, Some("Bottom")),
                        (self.tr.pos_default, None),
                    ] {
                        if ui
                            .small_button(label)
                            .on_hover_text(self.tr.tip_position)
                            .clicked()
                        {
                            self.propose(Action::SetPosition(value.map(str::to_string)));
                        }
                    }
                });
            });
        });
        let _ = ctx;
    }

    /// Der Werkzeugkasten.
    ///
    /// Deliberately not a tree and not a table: this list is short by nature —
    /// it holds what one person reaches for often — and its order is the
    /// user's own, so it is shown exactly as saved.
    fn favourite_list(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading(self.tr.tab_favourites);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.tr.fav_new).clicked() {
                    self.dialog = Some(Dialog::Favourite {
                        draft: Box::new(blank_favourite()),
                        fresh: true,
                    });
                }
            });
        });
        ui.separator();

        if let Some(error) = &self.favourite_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.separator();
        }

        if self.favourites.is_empty() {
            ui.add_space(12.0);
            ui.label(self.tr.fav_empty);
            return;
        }

        // Collected first, applied after the loop: every one of these mutates
        // the list the loop is walking.
        let mut edit: Option<Favourite> = None;
        let mut place: Option<Favourite> = None;
        let mut remove: Option<String> = None;
        let mut shift: Option<(String, bool)> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let count = self.favourites.len();
                for (index, favourite) in self.favourites.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(340.0);
                            ui.strong(&favourite.name);
                            ui.small(describe(favourite, self.tr));
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button(self.tr.fav_remove)
                                .on_hover_text(self.tr.tip_fav_remove)
                                .clicked()
                            {
                                remove = Some(favourite.id.clone());
                            }
                            if ui
                                .small_button(self.tr.fav_edit)
                                .on_hover_text(self.tr.tip_fav_edit)
                                .clicked()
                            {
                                edit = Some(favourite.clone());
                            }
                            if ui
                                .add_enabled(
                                    index + 1 < count,
                                    egui::Button::new("\u{2193}").small(),
                                )
                                .on_hover_text(self.tr.tip_fav_down)
                                .clicked()
                            {
                                shift = Some((favourite.id.clone(), false));
                            }
                            if ui
                                .add_enabled(index > 0, egui::Button::new("\u{2191}").small())
                                .on_hover_text(self.tr.tip_fav_up)
                                .clicked()
                            {
                                shift = Some((favourite.id.clone(), true));
                            }
                            ui.separator();
                            // The whole point of the list: putting a tool
                            // where it can be reached.
                            if ui
                                .button(self.tr.fav_place)
                                .on_hover_text(self.tr.tip_fav_place)
                                .clicked()
                            {
                                place = Some(favourite.clone());
                            }
                        });
                    });
                    ui.separator();
                }
            });

        if let Some(favourite) = edit {
            self.dialog = Some(Dialog::Favourite {
                draft: Box::new(favourite),
                fresh: false,
            });
        }
        if let Some(favourite) = place {
            self.dialog = Some(Dialog::Place {
                favourite: Box::new(favourite),
                category: Category::AllFiles,
                ext: String::new(),
                perceived: "image".into(),
            });
        }
        if let Some(id) = remove {
            self.after_favourite_change(favourites::remove(&id));
        }
        if let Some((id, up)) = shift {
            self.after_favourite_change(favourites::shift(&id, up));
        }
    }

    fn reload_favourites(&mut self) {
        match favourites::load() {
            Ok(list) => {
                self.favourites = list;
                self.favourite_error = None;
            }
            Err(error) => {
                // A damaged file must say so rather than look like an empty
                // tool box, or the next save would overwrite what is left.
                self.favourites.clear();
                self.favourite_error = Some(format!("{error:#}"));
            }
        }
    }

    fn after_favourite_change(&mut self, outcome: anyhow::Result<()>) {
        match outcome {
            Ok(()) => self.reload_favourites(),
            Err(error) => self.favourite_error = Some(format!("{error:#}")),
        }
    }

    /// The dialog that edits one favourite.
    fn favourite_dialog(&mut self, ui: &mut Ui, mut draft: Box<Favourite>, fresh: bool) {
        let mut save = false;
        let mut close = false;

        egui::Window::new(if fresh {
            self.tr.fav_new
        } else {
            self.tr.fav_edit
        })
        .collapsible(false)
        .resizable(true)
        .default_width(640.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            egui::Grid::new("fav-grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(self.tr.fav_name);
                    ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(440.0));
                    ui.end_row();

                    ui.label(self.tr.fav_kind);
                    ui.horizontal(|ui| {
                        let is_program = matches!(draft.tool, Tool::Program { .. });
                        if ui
                            .selectable_label(is_program, self.tr.fav_kind_program)
                            .clicked()
                            && !is_program
                        {
                            draft.tool = Tool::Program {
                                path: std::path::PathBuf::new(),
                                args: String::new(),
                            };
                        }
                        if ui
                            .selectable_label(!is_program, self.tr.fav_kind_web)
                            .clicked()
                            && is_program
                        {
                            draft.tool = Tool::Web(WebTool {
                                mode: WebMode::Clipboard { url: String::new() },
                                allow_insecure: false,
                                confirmed: false,
                            });
                        }
                    });
                    ui.end_row();
                });

            ui.separator();
            match &mut draft.tool {
                Tool::Program { path, args } => program_form(ui, self.tr, path, args),
                Tool::Web(web) => web_form(ui, self.tr, web),
            }

            ui.separator();
            let problems = draft.problems();
            for problem in &problems {
                ui.colored_label(ui.visuals().warn_fg_color, problem);
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let blocked = draft.name.trim().is_empty();
                if ui
                    .add_enabled(!blocked, egui::Button::new(self.tr.fav_save))
                    .clicked()
                {
                    save = true;
                }
                if ui.button(self.tr.btn_cancel).clicked() {
                    close = true;
                }
            });
        });

        if save {
            let outcome = if fresh {
                favourites::add(*draft.clone()).map(|_| ())
            } else {
                favourites::update(*draft.clone())
            };
            self.after_favourite_change(outcome);
        } else if !close {
            self.dialog = Some(Dialog::Favourite { draft, fresh });
        }
    }

    /// The dialog that puts a favourite into the context menu.
    fn place_dialog(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        favourite: Box<Favourite>,
        mut category: Category,
        mut ext: String,
        mut perceived: String,
    ) {
        let mut write = false;
        let mut close = false;

        egui::Window::new(self.tr.fav_place)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.set_min_width(560.0);
                ui.label(self.tr.fmt_fav_place_intro.replace("{}", &favourite.name));
                ui.add_space(6.0);

                for candidate in Category::BASE {
                    let chosen = category == candidate;
                    if ui
                        .radio(chosen, category_label(&candidate, self.tr))
                        .clicked()
                    {
                        category = candidate.clone();
                    }
                }

                ui.separator();
                // The two that need a value of their own: one extension, or a
                // whole class of file.
                ui.horizontal(|ui| {
                    let chosen = matches!(category, Category::ExtAssoc(_));
                    if ui.radio(chosen, self.tr.fav_place_ext).clicked() {
                        category = Category::ExtAssoc(ext.clone());
                    }
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut ext)
                                .desired_width(120.0)
                                .hint_text(".png"),
                        )
                        .changed()
                        && chosen
                    {
                        category = Category::ExtAssoc(ext.clone());
                    }
                });

                ui.horizontal(|ui| {
                    let chosen = matches!(category, Category::PerceivedType(_));
                    if ui.radio(chosen, self.tr.fav_place_perceived).clicked() {
                        category = Category::PerceivedType(perceived.clone());
                    }
                    for kind in ["image", "video", "audio", "text", "compressed"] {
                        if ui.selectable_label(perceived == kind, kind).clicked() {
                            perceived = kind.to_string();
                            category = Category::PerceivedType(perceived.clone());
                        }
                    }
                });

                ui.add_space(6.0);
                let exe = std::env::current_exe().unwrap_or_default();
                let entry = favourite.entry(category.clone(), &exe);
                match entry.target() {
                    Ok(target) => {
                        ui.small(format!("\u{2192} {}", target.full_path()));
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, format!("{error:#}"));
                    }
                }
                ui.small(&entry.command);

                let problems = crate::registry::create::check(&entry);
                let blocked = problems.iter().any(Problem::is_error) || entry.target().is_err();
                for problem in &problems {
                    let colour = match problem {
                        Problem::Error(_) => ui.visuals().error_fg_color,
                        Problem::Warning(_) => ui.visuals().warn_fg_color,
                    };
                    ui.colored_label(colour, problem.message());
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!blocked, egui::Button::new(self.tr.editor_create))
                        .clicked()
                    {
                        write = true;
                    }
                    if ui.button(self.tr.btn_cancel).clicked() {
                        close = true;
                    }
                });
            });

        if write {
            let exe = std::env::current_exe().unwrap_or_default();
            let entry = favourite.entry(category.clone(), &exe);
            match crate::registry::create::create(&entry) {
                Ok(target) => {
                    elevation::notify_shell();
                    self.start_scan(ctx);
                    self.dialog = Some(Dialog::Error(
                        self.tr.fmt_fav_placed.replace("{}", &target.full_path()),
                    ));
                }
                Err(error) => {
                    self.dialog = Some(Dialog::Error(format!("{error:#}")));
                }
            }
        } else if !close {
            self.dialog = Some(Dialog::Place {
                favourite,
                category,
                ext,
                perceived,
            });
        }
    }

    /// The confirmation, progress and result dialogs.
    fn dialogs(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };

        let mut keep = true;
        match dialog {
            Dialog::Confirm {
                plan,
                needs_elevation,
            } => {
                let mut start = false;
                let mut cancel = false;

                egui::Window::new(plan.label.clone())
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.label(
                            self.tr
                                .fmt_selected_count
                                .replace("{}", &plan.operations.len().to_string()),
                        );

                        let irreversible =
                            plan.operations.iter().any(|o| !o.action.is_reversible());
                        if irreversible {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                self.tr.msg_confirm_delete,
                            );
                        } else {
                            ui.label(self.tr.msg_backup_first);
                        }

                        if needs_elevation {
                            ui.add_space(4.0);
                            ui.colored_label(ui.visuals().warn_fg_color, self.tr.msg_needs_admin);
                        }

                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for operation in plan.operations.iter().take(40) {
                                    ui.small(format!(
                                        "{}  ·  {}",
                                        operation.display_name,
                                        operation.target.full_path()
                                    ));
                                }
                                if plan.operations.len() > 40 {
                                    ui.small(format!("… {}", plan.operations.len() - 40));
                                }
                            });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button(self.tr.btn_execute).clicked() {
                                start = true;
                            }
                            if ui.button(self.tr.btn_cancel).clicked() {
                                cancel = true;
                            }
                        });
                    });

                if start {
                    self.apply(plan, ctx);
                    keep = false;
                } else if cancel {
                    keep = false;
                } else {
                    self.dialog = Some(Dialog::Confirm {
                        plan,
                        needs_elevation,
                    });
                    keep = false;
                }
            }

            Dialog::Running => {
                egui::Window::new(self.tr.btn_execute)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(self.tr.status_scanning);
                        });
                    });
                self.dialog = Some(Dialog::Running);
                keep = false;
            }

            Dialog::Done(report) => {
                let mut close = false;
                let mut restore: Option<String> = None;

                egui::Window::new(self.tr.detail_title)
                    .collapsible(false)
                    .resizable(true)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.label(
                            self.tr
                                .fmt_report_counts
                                .replacen("{}", &report.succeeded().to_string(), 1)
                                .replacen("{}", &report.failed().to_string(), 1),
                        );

                        if let Some(directory) = &report.backup_directory {
                            ui.add_space(4.0);
                            ui.label(self.tr.fmt_backup_created.replace("{}", directory));
                            // Offered right here, because a partial failure is
                            // exactly when someone wants to go back and is
                            // least inclined to go hunting for the path.
                            if ui
                                .button(self.tr.btn_restore)
                                .on_hover_text(self.tr.tip_restore)
                                .clicked()
                            {
                                restore = Some(directory.clone());
                            }
                        }

                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height(260.0)
                            .show(ui, |ui| {
                                for result in &report.results {
                                    match &result.error {
                                        None => {
                                            ui.small(format!("✓  {}", result.display_name));
                                        }
                                        Some(error) => {
                                            ui.colored_label(
                                                ui.visuals().error_fg_color,
                                                format!("✗  {}  —  {error}", result.display_name),
                                            );
                                        }
                                    }
                                }
                            });

                        ui.add_space(6.0);
                        ui.small(self.tr.msg_restart_explorer);
                        ui.add_space(6.0);
                        if ui.button(self.tr.btn_cancel).clicked() {
                            close = true;
                        }
                    });

                if let Some(directory) = restore {
                    match backup::restore(std::path::Path::new(&directory)) {
                        Ok(_) => {
                            self.start_scan(ctx);
                            close = true;
                        }
                        Err(error) => {
                            self.dialog = Some(Dialog::Error(format!("{error:#}")));
                            keep = false;
                        }
                    }
                }
                if !close && keep {
                    self.dialog = Some(Dialog::Done(report));
                }
                keep = false;
            }

            Dialog::Error(message) => {
                let mut close = false;
                egui::Window::new(self.tr.title_error)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.colored_label(ui.visuals().error_fg_color, &message);
                        ui.add_space(6.0);
                        if ui.button(self.tr.btn_cancel).clicked() {
                            close = true;
                        }
                    });
                if !close {
                    self.dialog = Some(Dialog::Error(message));
                }
                keep = false;
            }

            Dialog::Favourite { draft, fresh } => {
                self.favourite_dialog(ui, draft, fresh);
                keep = false;
            }

            Dialog::Place {
                favourite,
                category,
                ext,
                perceived,
            } => {
                self.place_dialog(ui, ctx, favourite, category, ext, perceived);
                keep = false;
            }

            Dialog::Editor {
                mut entry,
                recorded,
            } => {
                let mut close = false;
                let mut save = false;

                egui::Window::new(self.tr.editor_title)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.set_min_width(560.0);

                        egui::Grid::new("editor-grid")
                            .num_columns(2)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.label(self.tr.editor_category);
                                egui::ComboBox::from_id_salt("editor-category")
                                    .selected_text(category_label(&entry.category, self.tr))
                                    .show_ui(ui, |ui| {
                                        for candidate in Category::BASE {
                                            let label = category_label(&candidate, self.tr);
                                            ui.selectable_value(
                                                &mut entry.category,
                                                candidate,
                                                label,
                                            );
                                        }
                                    });
                                ui.end_row();

                                ui.label(self.tr.editor_display_name);
                                let before = entry.display_name.clone();
                                ui.add(
                                    egui::TextEdit::singleline(&mut entry.display_name)
                                        .desired_width(400.0),
                                );
                                // The key name follows the display name until the
                                // moment someone edits it by hand; after that it is
                                // theirs and stays put.
                                if entry.display_name != before
                                    && (entry.key_name.is_empty()
                                        || entry.key_name == create::suggest_key_name(&before))
                                {
                                    entry.key_name = create::suggest_key_name(&entry.display_name);
                                }
                                ui.end_row();

                                ui.label(self.tr.editor_key_name);
                                ui.add(
                                    egui::TextEdit::singleline(&mut entry.key_name)
                                        .desired_width(400.0),
                                );
                                ui.end_row();

                                ui.label(self.tr.editor_command);
                                ui.add(
                                    egui::TextEdit::singleline(&mut entry.command)
                                        .desired_width(400.0)
                                        .hint_text(HINT_COMMAND),
                                );
                                ui.end_row();

                                ui.label(self.tr.editor_icon);
                                let mut icon = entry.icon.clone().unwrap_or_default();
                                ui.add(
                                    egui::TextEdit::singleline(&mut icon)
                                        .desired_width(400.0)
                                        .hint_text(HINT_ICON),
                                );
                                entry.icon = (!icon.trim().is_empty()).then_some(icon);
                                ui.end_row();

                                ui.label(self.tr.editor_position);
                                ui.horizontal(|ui| {
                                    for (label, value) in [
                                        (self.tr.pos_default, None),
                                        (self.tr.pos_top, Some("Top")),
                                        (self.tr.pos_bottom, Some("Bottom")),
                                    ] {
                                        let chosen = entry.position.as_deref() == value;
                                        if ui.selectable_label(chosen, label).clicked() {
                                            entry.position = value.map(str::to_string);
                                        }
                                    }
                                });
                                ui.end_row();

                                ui.label(self.tr.editor_visibility);
                                ui.checkbox(&mut entry.extended, self.tr.editor_extended);
                                ui.end_row();
                            });

                        ui.add_space(6.0);
                        match entry.target() {
                            Ok(target) => {
                                ui.small(format!("\u{2192} {}", target.full_path()));
                            }
                            Err(error) => {
                                ui.small(format!("{error:#}"));
                            }
                        }

                        // Live, because a warning after the fact is no use: the %1
                        // trap costs an entry that looks right and does nothing.
                        let problems = create::check(&entry);
                        let blocked = problems.iter().any(Problem::is_error);
                        if !problems.is_empty() {
                            ui.add_space(4.0);
                            for problem in &problems {
                                let colour = match problem {
                                    Problem::Error(_) => ui.visuals().error_fg_color,
                                    Problem::Warning(_) => ui.visuals().warn_fg_color,
                                };
                                ui.colored_label(colour, problem.message());
                            }
                        }

                        if !recorded.is_empty() {
                            ui.add_space(6.0);
                            ui.separator();
                            ui.label(self.tr.editor_created_before);
                            egui::ScrollArea::vertical()
                                .max_height(90.0)
                                .show(ui, |ui| {
                                    for existing in &recorded {
                                        ui.small(format!(
                                            "{}  \u{b7}  {}  \u{b7}  {}",
                                            existing.display_name,
                                            category_label(&existing.category, self.tr),
                                            existing.key_name
                                        ));
                                    }
                                });
                        }

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!blocked, egui::Button::new(self.tr.editor_create))
                                .clicked()
                            {
                                save = true;
                            }
                            if ui.button(self.tr.btn_cancel).clicked() {
                                close = true;
                            }
                        });
                    });

                if save {
                    match create::create(&entry) {
                        Ok(_) => {
                            // Without this the entry exists but the running
                            // Explorer keeps showing yesterday's menu.
                            elevation::notify_shell();
                            self.start_scan(ctx);
                        }
                        Err(error) => {
                            self.dialog = Some(Dialog::Error(format!("{error:#}")));
                        }
                    }
                } else if !close {
                    self.dialog = Some(Dialog::Editor { entry, recorded });
                }
                keep = false;
            }
        }
        let _ = keep;
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
                    ui.label(
                        self.tr
                            .fmt_shown
                            .replace("{}", &self.visible_rows.len().to_string()),
                    );
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
                    if let Some(ms) = self.first_list_ms {
                        ui.separator();
                        ui.label(format!("Start {ms:.0} ms"));
                    }
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
                            self.clear_selection();
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
                                self.clear_selection();
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
                let tr = self.tr;
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
                                type_group_label(group, tr)
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
                    self.clear_selection();
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
                ui.horizontal(|ui| {
                    ui.label(
                        self.tr
                            .fmt_entries_found
                            .replace("{}", &self.groups.len().to_string()),
                    );

                    // The short way from "this program keeps turning up" to
                    // "keep it, I want it elsewhere too". A COM handler has no
                    // command line to reuse, so it cannot become a favourite.
                    let group = self.selected_group.and_then(|i| self.groups.get(i));
                    let usable = group.is_some_and(|g| g.key.to_lowercase().ends_with(".exe"));
                    if ui
                        .add_enabled(
                            usable,
                            egui::Button::new(self.tr.fav_add_from_program).small(),
                        )
                        .clicked()
                        && let Some(group) = group
                    {
                        let draft = Favourite {
                            id: String::new(),
                            name: group.display_name.clone(),
                            icon: group.icon_ref.clone(),
                            note: None,
                            tool: Tool::Program {
                                path: std::path::PathBuf::from(&group.key),
                                args: String::new(),
                            },
                        };
                        self.dialog = Some(Dialog::Favourite {
                            draft: Box::new(draft),
                            fresh: true,
                        });
                    }
                });
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
                    self.clear_selection();
                    self.filter_dirty = true;
                }
            });
    }

    fn entry_table(&mut self, ui: &mut Ui, scroll_to: Option<usize>) {
        // Destructured up front: the row closure needs the entries and the
        // icon cache at the same time, which a plain `&mut self` capture
        // would not allow.
        let Self {
            scan,
            visible_rows,
            icons,
            selected,
            focused,
            tr,
            bench,
            sort,
            ..
        } = self;
        let mut new_sort: Option<SortBy> = None;

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

        // Keeps the keyboard cursor on screen. Without it the selection walks
        // out of view and the list looks frozen.
        if let Some(row) = scroll_to {
            table = table.scroll_to_row(row, None);
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
            .column(Column::initial(240.0).at_least(120.0).clip(true))
            .column(Column::initial(90.0).at_least(70.0).clip(true))
            .column(Column::initial(80.0).at_least(60.0).clip(true))
            // What this entry hangs on: .zip, .rar, image. The column that
            // tells twenty rows of one program apart.
            .column(Column::initial(110.0).at_least(70.0).clip(true))
            .column(Column::initial(110.0).at_least(70.0).clip(true))
            .column(Column::remainder().at_least(140.0).clip(true))
            .header(24.0, |mut header| {
                header.col(|_ui| {});
                for (label, column) in [
                    (tr.col_name, Some(SortBy::Name)),
                    (tr.col_type, Some(SortBy::Kind)),
                    (tr.col_scope, Some(SortBy::Scope)),
                    (tr.col_applies_to, Some(SortBy::AppliesTo)),
                    (tr.col_flags, None),
                    (tr.col_command, Some(SortBy::Command)),
                ] {
                    header.col(|ui| {
                        let Some(column) = column else {
                            ui.strong(label);
                            return;
                        };

                        // The arrow is the whole feedback: without it nobody
                        // can tell which of five columns the order comes from.
                        let caption = if sort.0 == column {
                            format!("{label} {}", if sort.1 { "\u{25b4}" } else { "\u{25be}" })
                        } else {
                            label.to_string()
                        };

                        if ui
                            .add(
                                egui::Label::new(egui::RichText::new(caption).strong())
                                    .sense(Sense::click()),
                            )
                            .on_hover_text(tr.btn_sort_hint)
                            .clicked()
                        {
                            new_sort = Some(column);
                        }
                    });
                }
            })
            .body(|body| {
                // The virtualized variant: only visible rows are built. At a
                // few thousand entries this is the difference between a
                // scrolling list and a slideshow (ToDo 4.5).
                body.rows(26.0, visible_rows.len(), |mut row| {
                    let reference = &visible_rows[row.index()];
                    let index = reference.entry;
                    let Some(entry) = resolve(scan, reference) else {
                        return;
                    };
                    let depth = reference.path.len();

                    // Must precede the first cell; it only affects cells added
                    // after the call. A child is never selected on its own.
                    row.set_selected(reference.is_top() && selected.contains(&index));

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
                        match depth {
                            // The arrow that a cascading menu shows, in the
                            // place the menu itself would show it.
                            0 if children(entry).is_some() => {
                                ui.label(format!("{}  \u{25b8}", entry.display_name));
                            }
                            0 => {
                                ui.label(&entry.display_name);
                            }
                            _ => {
                                ui.weak(format!(
                                    "{}\u{21b3} {}",
                                    "    ".repeat(depth),
                                    entry.display_name
                                ));
                            }
                        };
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
                        // Plain words, because `*` and `Folder` are registry
                        // shorthand that says nothing to the person deciding
                        // whether to delete a row. The real location is one
                        // hover away for anyone who wants it.
                        ui.label(appears_on(entry, tr))
                            .on_hover_text(&entry.registry_path);
                    });
                    row.col(|ui| {
                        badges(ui, entry, tr);
                    });
                    row.col(|ui| {
                        ui.label(detail_text(entry));
                    });

                    let response = row.response();
                    // A click on a submenu child selects its parent rather
                    // than nothing: the child cannot be acted on by itself,
                    // but a row that swallows every click looks broken.
                    if response.clicked() {
                        // Ctrl adds to the selection, a plain click replaces
                        // it — the convention every file manager uses.
                        if response.ctx.input(|i| i.modifiers.ctrl) {
                            if !selected.remove(&index) {
                                selected.insert(index);
                            }
                        } else {
                            selected.clear();
                            selected.insert(index);
                        }
                        *focused = Some(index);
                    }
                });
            });

        if let Some(column) = new_sort {
            // Clicking the column that is already active turns the order
            // around, which is what every file list does.
            self.sort = if self.sort.0 == column {
                (column, !self.sort.1)
            } else {
                (column, true)
            };
            self.filter_dirty = true;
        }
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
                    .focused
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
                        // Always shown, not only when the registry says
                        // something: "where does this thing actually turn up"
                        // is the first question about any entry.
                        field(ui, self.tr.detail_applies_to, &appears_on(entry, self.tr));
                        if let Some(applies) = &entry.applies_to {
                            // The raw AppliesTo query, when there is one. It is
                            // a structured filter Windows evaluates per item,
                            // so it narrows things further than the location.
                            field(ui, "AppliesTo", applies);
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
                                format!("  {missing} ({})", self.tr.badge_missing),
                            );
                        }
                        // What reg.exe said about the gaps, so an incomplete
                        // backup can be judged instead of only noticed.
                        for note in &manifest.notes {
                            ui.small(format!("  {note}"));
                        }
                    });
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// An empty favourite to start from.
///
/// A program rather than a web tool: adding an executable is the common case,
/// and the kind can be switched in one click.
fn blank_favourite() -> Favourite {
    Favourite {
        id: String::new(),
        name: String::new(),
        icon: None,
        note: None,
        tool: Tool::Program {
            path: std::path::PathBuf::new(),
            args: String::new(),
        },
    }
}

/// One line saying what a favourite is and where it points.
fn describe(favourite: &Favourite, tr: &'static Strings) -> String {
    match &favourite.tool {
        Tool::Program { path, args } => {
            let args = args.trim();
            if args.is_empty() {
                format!("{}  \u{b7}  {}", tr.fav_kind_program, path.display())
            } else {
                format!(
                    "{}  \u{b7}  {} {}",
                    tr.fav_kind_program,
                    path.display(),
                    args
                )
            }
        }
        Tool::Web(web) => {
            let mode = match &web.mode {
                WebMode::Open { .. } => tr.fav_mode_open,
                WebMode::Clipboard { .. } => tr.fav_mode_clipboard,
                WebMode::Upload(_) => tr.fav_mode_upload,
            };
            format!(
                "{}  \u{b7}  {}  \u{b7}  {}",
                tr.fav_kind_web,
                mode,
                favourite.address().unwrap_or_default()
            )
        }
    }
}

fn program_form(
    ui: &mut Ui,
    tr: &'static Strings,
    path: &mut std::path::PathBuf,
    args: &mut String,
) {
    egui::Grid::new("fav-program")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr.fav_path);
            let mut text = path.to_string_lossy().to_string();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(440.0)
                        .hint_text(HINT_PROGRAM),
                )
                .changed()
            {
                // Pasted paths often arrive wrapped in quotes, and a quoted
                // path resolves to nothing.
                *path = std::path::PathBuf::from(text.trim().trim_matches('"'));
            }
            ui.end_row();

            ui.label(tr.fav_args);
            ui.add(
                egui::TextEdit::singleline(args)
                    .desired_width(440.0)
                    .hint_text(HINT_ARGS),
            );
            ui.end_row();
        });
    ui.small(tr.fav_args_hint);
}

fn web_form(ui: &mut Ui, tr: &'static Strings, web: &mut WebTool) {
    ui.horizontal(|ui| {
        ui.label(tr.fav_mode);
        let current = mode_index(&web.mode);
        for (index, label) in [tr.fav_mode_clipboard, tr.fav_mode_open, tr.fav_mode_upload]
            .into_iter()
            .enumerate()
        {
            if ui.selectable_label(current == index, label).clicked() && current != index {
                let url = current_url(&web.mode);
                web.mode = match index {
                    0 => WebMode::Clipboard { url },
                    1 => WebMode::Open { url },
                    _ => WebMode::Upload(Upload {
                        endpoint: url,
                        method: "POST".into(),
                        body: UploadBody::Multipart {
                            field: "file".into(),
                        },
                        headers: Vec::new(),
                        fields: Vec::new(),
                        result: ResultAction::Report,
                    }),
                };
            }
        }
    });

    ui.small(match &web.mode {
        WebMode::Clipboard { .. } => tr.fav_mode_clipboard_hint,
        WebMode::Open { .. } => tr.fav_mode_open_hint,
        WebMode::Upload(_) => tr.fav_mode_upload_hint,
    });
    ui.add_space(4.0);

    match &mut web.mode {
        WebMode::Clipboard { url } | WebMode::Open { url } => {
            ui.horizontal(|ui| {
                ui.label(tr.fav_url);
                ui.add(
                    egui::TextEdit::singleline(url)
                        .desired_width(460.0)
                        .hint_text(HINT_URL),
                );
            });
        }
        WebMode::Upload(upload) => upload_form(ui, tr, upload),
    }

    ui.add_space(4.0);
    ui.checkbox(&mut web.allow_insecure, tr.fav_allow_insecure);
    if web.confirmed {
        ui.horizontal(|ui| {
            ui.small(tr.fav_confirmed);
            if ui.small_button(tr.fav_forget_consent).clicked() {
                web.confirmed = false;
            }
        });
    }
}

fn upload_form(ui: &mut Ui, tr: &'static Strings, upload: &mut Upload) {
    egui::Grid::new("fav-upload")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr.fav_endpoint);
            ui.add(
                egui::TextEdit::singleline(&mut upload.endpoint)
                    .desired_width(440.0)
                    .hint_text(HINT_ENDPOINT),
            );
            ui.end_row();

            ui.label(tr.fav_method);
            ui.horizontal(|ui| {
                for verb in ["POST", "PUT"] {
                    if ui.selectable_label(upload.method == verb, verb).clicked() {
                        upload.method = verb.to_string();
                    }
                }
            });
            ui.end_row();

            ui.label(tr.fav_body);
            ui.horizontal(|ui| {
                let multipart = matches!(upload.body, UploadBody::Multipart { .. });
                if ui
                    .selectable_label(multipart, tr.fav_body_multipart)
                    .clicked()
                {
                    upload.body = UploadBody::Multipart {
                        field: "file".into(),
                    };
                }
                if ui.selectable_label(!multipart, tr.fav_body_raw).clicked() {
                    upload.body = UploadBody::Raw;
                }
                if let UploadBody::Multipart { field } = &mut upload.body {
                    ui.label(tr.fav_field);
                    ui.add(egui::TextEdit::singleline(field).desired_width(120.0));
                }
            });
            ui.end_row();

            ui.label(tr.fav_result);
            ui.horizontal(|ui| {
                let current = result_index(&upload.result);
                for (index, label) in [tr.fav_result_save, tr.fav_result_open, tr.fav_result_report]
                    .into_iter()
                    .enumerate()
                {
                    if ui.selectable_label(current == index, label).clicked() && current != index {
                        let source = current_source(&upload.result);
                        upload.result = match index {
                            0 => ResultAction::Save {
                                source,
                                suffix: ".neu".into(),
                            },
                            1 => ResultAction::Open { source },
                            _ => ResultAction::Report,
                        };
                    }
                }
            });
            ui.end_row();

            // Where the answer keeps the result, and what to call it.
            match &mut upload.result {
                ResultAction::Save { source, suffix } => {
                    ui.label(tr.fav_result_source);
                    ui.horizontal(|ui| {
                        source_picker(ui, tr, source);
                        ui.label(tr.fav_suffix);
                        ui.add(egui::TextEdit::singleline(suffix).desired_width(80.0));
                    });
                    ui.end_row();
                }
                ResultAction::Open { source } => {
                    ui.label(tr.fav_result_source);
                    ui.horizontal(|ui| source_picker(ui, tr, source));
                    ui.end_row();
                }
                ResultAction::Report => {}
            }
        });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tr.fav_headers);
        if ui.small_button(tr.fav_header_add).clicked() {
            upload.headers.push(Header {
                name: String::new(),
                value: String::new(),
            });
        }
    });

    let mut drop_header = None;
    for (index, header) in upload.headers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut header.name)
                    .desired_width(160.0)
                    .hint_text("Authorization"),
            );
            ui.add(
                egui::TextEdit::singleline(&mut header.value)
                    .desired_width(320.0)
                    .hint_text("Basic \u{2026}"),
            );
            if ui.small_button("\u{2715}").clicked() {
                drop_header = Some(index);
            }
        });
    }
    if let Some(index) = drop_header {
        upload.headers.remove(index);
    }
}

fn source_picker(ui: &mut Ui, tr: &'static Strings, source: &mut ResultSource) {
    let current = match source {
        ResultSource::Body => 0,
        ResultSource::Location => 1,
        ResultSource::Json { .. } => 2,
    };

    for (index, label) in [
        tr.fav_source_body,
        tr.fav_source_location,
        tr.fav_source_json,
    ]
    .into_iter()
    .enumerate()
    {
        if ui.selectable_label(current == index, label).clicked() && current != index {
            *source = match index {
                0 => ResultSource::Body,
                1 => ResultSource::Location,
                _ => ResultSource::Json {
                    path: "output.url".into(),
                },
            };
        }
    }

    if let ResultSource::Json { path } = source {
        ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(160.0)
                .hint_text("output.url"),
        );
    }
}

fn mode_index(mode: &WebMode) -> usize {
    match mode {
        WebMode::Clipboard { .. } => 0,
        WebMode::Open { .. } => 1,
        WebMode::Upload(_) => 2,
    }
}

/// Keeps the address when the mode changes, so switching to compare does not
/// wipe what was typed.
fn current_url(mode: &WebMode) -> String {
    match mode {
        WebMode::Clipboard { url } | WebMode::Open { url } => url.clone(),
        WebMode::Upload(upload) => upload.endpoint.clone(),
    }
}

fn result_index(result: &ResultAction) -> usize {
    match result {
        ResultAction::Save { .. } => 0,
        ResultAction::Open { .. } => 1,
        ResultAction::Report => 2,
    }
}

fn current_source(result: &ResultAction) -> ResultSource {
    match result {
        ResultAction::Save { source, .. } | ResultAction::Open { source } => source.clone(),
        ResultAction::Report => ResultSource::Body,
    }
}

fn strings_for(language: Language) -> &'static Strings {
    match language {
        Language::German => &i18n::DE,
        Language::English => &i18n::EN,
    }
}

/// Human label for a file type group.
///
/// Four of the nine differ between the languages and live in the table; the
/// rest are the same word twice. Mixing both languages into one label, which
/// is how this started, means every user reads half a caption they did not
/// ask for.
fn type_group_label(
    group: crate::registry::filetypes::TypeGroup,
    tr: &'static Strings,
) -> &'static str {
    use crate::registry::filetypes::TypeGroup as G;
    match group {
        G::Documents => tr.grp_documents,
        G::Images => tr.grp_images,
        G::Archives => tr.grp_archives,
        G::Other => tr.grp_other,
        // The rest are the same word in both languages, and translating
        // "Audio" into "Audio" would only add a way to get them out of step.
        G::Raw => "RAW",
        G::Audio => "Audio",
        G::Video => "Video",
        G::Code => "Code",
        G::System => "System",
    }
}

/// Where an entry turns up, in words rather than registry shorthand.
///
/// `*` means "on every file" and `Folder` means "on folders"; neither says so
/// to anyone who has not read the documentation. For a file type entry the
/// answer is the type itself, which is the difference that matters when one
/// program registers itself twenty times.
fn appears_on(entry: &ContextEntry, tr: &'static Strings) -> String {
    let applies = entry.category.applies_to_label();
    if applies.is_empty() {
        category_label(&entry.category, tr).to_string()
    } else {
        applies
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

/// How long ago this process was created, in milliseconds.
///
/// `GetProcessTimes` hands back a FILETIME, which counts 100-nanosecond ticks
/// from 1601; the wall clock comes from `SystemTime`, which counts from 1970.
/// The constant between them is the number of seconds in those 369 years.
fn milliseconds_since_process_start() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    const UNIX_EPOCH_IN_FILETIME_SECONDS: u64 = 11_644_473_600;
    let ticks = |time: FILETIME| ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();

    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok.is_err() {
        return f64::NAN;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() / 100 + (UNIX_EPOCH_IN_FILETIME_SECONDS as u128) * 10_000_000)
        .unwrap_or(0);

    (now.saturating_sub(ticks(creation) as u128)) as f64 / 10_000.0
}

/// The value a column is ordered by.
///
/// Lowercased throughout: a list where `WinRAR` sorts before `attrib` because
/// of capitalisation is a list nobody can find anything in.
fn sort_key(entry: &ContextEntry, column: SortBy) -> String {
    match column {
        SortBy::Name => entry.display_name.to_lowercase(),
        SortBy::Kind => entry.kind.type_label().to_string(),
        SortBy::Scope => entry.scope.label().to_string(),
        // Empty last rather than first: the rows that have nothing to say in
        // this column are the ones the reader is not looking for here.
        SortBy::AppliesTo => match entry.category.applies_to_label() {
            label if label.is_empty() => "\u{ffff}".to_string(),
            label => label.to_lowercase(),
        },
        SortBy::Command => detail_text(entry).to_lowercase(),
    }
}

fn matches_search(entry: &ContextEntry, needle: &str) -> bool {
    // A submenu child matches for its parent: searching for the child and
    // being told there is no such entry would be a lie, since the right-click
    // menu does offer it.
    if let Some(kids) = children(entry)
        && kids.iter().any(|child| matches_search(child, needle))
    {
        return true;
    }

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
    start_search: String,
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
        Box::new(move |cc| {
            Ok(Box::new(App::new(
                cc,
                synthetic,
                bench_frames,
                start_tab,
                start_search.clone(),
            )))
        }),
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

    /// Builds a parent with one child and one grandchild.
    #[cfg(test)]
    fn cascading() -> ContextEntry {
        fn leaf(name: &str, kids: Vec<ContextEntry>) -> ContextEntry {
            let mut entry = synthetic::scan_result(1).entries.remove(0);
            entry.display_name = name.to_string();
            entry.key_name = name.to_string();
            entry.kind = EntryKind::Verb {
                command: None,
                sub_commands: kids,
            };
            entry
        }

        leaf(
            "Eltern",
            vec![leaf("Kind", vec![leaf("Enkelkind", Vec::new())])],
        )
    }

    #[test]
    fn a_submenu_becomes_one_row_per_level() {
        let parent = cascading();
        let mut rows = Vec::new();
        push_with_children(&mut rows, &parent, Row::top(7));

        assert_eq!(rows.len(), 3, "parent, child and grandchild");
        assert!(rows.iter().all(|r| r.entry == 7), "all belong to one entry");
        assert_eq!(rows[0].path, Vec::<usize>::new());
        assert_eq!(rows[1].path, vec![0]);
        assert_eq!(rows[2].path, vec![0, 0]);

        // Exactly one selectable row: a child key is not a RegTarget.
        assert_eq!(rows.iter().filter(|r| r.is_top()).count(), 1);
    }

    #[test]
    fn a_row_resolves_to_the_entry_it_names() {
        let mut scan = synthetic::scan_result(3);
        scan.entries[1] = cascading();

        let mut rows = Vec::new();
        push_with_children(&mut rows, &scan.entries[1], Row::top(1));

        let names: Vec<&str> = rows
            .iter()
            .map(|row| resolve(&scan, row).expect("row fits").display_name.as_str())
            .collect();
        assert_eq!(names, ["Eltern", "Kind", "Enkelkind"]);

        // A path that no longer fits must not panic; it is dropped for a frame.
        assert!(
            resolve(
                &scan,
                &Row {
                    entry: 1,
                    path: vec![9]
                }
            )
            .is_none()
        );
    }

    #[test]
    fn searching_finds_an_entry_by_the_name_of_its_child() {
        let parent = cascading();

        // The user sees "Enkelkind" in the right-click menu, so searching for
        // it must lead somewhere.
        assert!(matches_search(&parent, "enkelkind"));
        assert!(matches_search(&parent, "kind"));
        assert!(!matches_search(&parent, "gibt es nicht"));
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
